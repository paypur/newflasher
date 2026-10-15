use crate::fastboot::{FastbootDevice, Header};
use crate::types::{ProgressBar, Slot};
use crate::utils::*;
use anyhow::{Context, anyhow};
use flate2::read::GzDecoder;
use log::{debug, error, info};
use std::fs::File;
use std::io::{Read, Seek};
use std::path::Path;
use tar::{Archive, Entry, EntryType};

pub fn process_sins(
    usb: &mut FastbootDevice,
    sin_path: &Path,
    target_slot: Slot,
) -> anyhow::Result<()> {
    let flash_prefix = validate_prefix(sin_path)?;

    let flash_both_slots = flash_prefix == "bootloader" || flash_prefix == "bluetooth" || flash_prefix == "dsp" || flash_prefix == "modem" || flash_prefix == "rdimage";

    process_sins_slot(usb, sin_path, flash_prefix.as_str(), target_slot, flash_both_slots)?;

    if flash_both_slots {
        process_sins_slot(usb, sin_path, flash_prefix.as_str(), target_slot.other(), flash_both_slots)?;
    }

    Ok(())
}

fn process_sins_slot(
    usb: &mut FastbootDevice,
    sin_path: &Path,
    flash_prefix: &str,
    target_slot: Slot,
    flash_both_slots: bool,
) -> anyhow::Result<()> {
    // info!("Processing {}", sin_path.to_string_lossy());

    // TODO: move to main
    let mut keep_userdata: bool = true;

    let file_name = sin_path.file_name().unwrap().to_string_lossy();

    if keep_userdata && noerase_in_updatexml(sin_path.file_name().unwrap()) {
        info!("Skipping {}", file_name);
        return Ok(());
    }

    let parts = open_sin_archive(&sin_path)?
        .entries()?
        .count();

    let prefix = match file_name.split_once('_') {
        Some((first, _)) => first,
        None => file_name.as_ref(),
    };

    let text = if flash_both_slots {
        format!("Processing {prefix} ({target_slot})")
    } else {
        format!("Processing {prefix}")
    };

    let progress = ProgressBar::new(parts as u64, text);

    let other = sin_path.to_string_lossy().contains("_other");

    // TODO: is this assumption always true?
    let is_partition_image = flash_prefix.starts_with("partitionimage_");

    let has_slot = !is_partition_image && {
        let cmd = format!("getvar:has-slot:{flash_prefix}");
        match usb.command(cmd.as_str()) {
            Ok(res) => res.data == b"yes",
            Err(e) => {
                error!("{e}");
                false
            }
        }
    };

    for (i, mut entry) in open_sin_archive(&sin_path)?
        .entries()?
        .into_iter()
        .flatten()
        .filter(filter_entry)
        .enumerate() {
        if i == 0 {
            usb.download_cms(&mut entry)?;
        } else {
            usb.download_tar_entry(&mut entry).with_context(|| format!("Chunk {i} of {parts}"))?;

            if is_partition_image {
                let cmd = flash_prefix.replace("partitionimage_", "Repartition:");
                usb.command_expect_header(cmd, Header::Okay)?;
            } else {
                if i == 1 {
                    let erase_cmd = match (has_slot, other) {
                        (false, _) => format!("erase:{flash_prefix}"),
                        (true, false) => format!("erase:{flash_prefix}_{target_slot}"),
                        (true, true) => format!("erase:{flash_prefix}_{}", target_slot.other()),
                    };
                    usb.command_expect_header(erase_cmd, Header::Okay)?;
                }

                let flash_cmd = match (has_slot, other) {
                    (false, _) => format!("flash:{flash_prefix}"),
                    (true, false) => format!("flash:{flash_prefix}_{target_slot}"),
                    (true, true) => format!("flash:{flash_prefix}_{}", target_slot.other()),
                };
                usb.command_expect_header(flash_cmd, Header::Okay)?;
            }
        }
        progress.set_position(i as u64);
    }

    progress.okay();

    Ok(())
}

fn validate_prefix(sin_path: &Path) -> anyhow::Result<String> {
    let mut archive = open_sin_archive(&sin_path)?;

    let mut prefix_iter = archive.entries()?
        .flatten()
        .filter(|e| filter_entry(e))
        .map(|entry| {
            // ensure!(entry.size() != 0, "Tar entry contained 0 bytes!");
            let name = tar_entry_header_name(&entry).ok()?;
            Some(Path::new(name).with_extension(""))
        })
        .into_iter();

    let flash_prefix = match prefix_iter.next().flatten() {
        Some(prefix) => prefix,
        None => return Err(anyhow!("Empty tar entry name!"))
    };

    if !prefix_iter.all(|opt| opt.map(|s| s == flash_prefix).unwrap_or(false)) {
        return Err(anyhow!("Mismatched tar entry name!"));
    }

    Ok(flash_prefix.to_str().ok_or_else(|| anyhow!("Invalid UTF-8 in sin archive entry prefix!"))?.to_string())
}

fn open_sin_archive(sin_path: &Path) -> anyhow::Result<Archive<Box<dyn Read>>> {
    let mut sin_file = File::open(sin_path)?;

    let mut magic_numbers = [0u8; 2];

    sin_file.read_exact(magic_numbers.as_mut_slice())?;
    sin_file.rewind()?;

    let reader: Box<dyn Read> = match magic_numbers {
        [0x1F, 0x8B] => Box::new(GzDecoder::new(sin_file)),
        _ => Box::new(sin_file)
    };

    Ok(Archive::new(reader))
}

fn filter_entry(entry: &Entry<Box<dyn Read>>) -> bool {
    if entry.size() == 0 {
        return false;
    }
    match entry.header().entry_type() {
        EntryType::Regular | EntryType::Continuous => true,
        et => {
            debug!("Ignoring {:?}", et);
            false
        }
    }
}