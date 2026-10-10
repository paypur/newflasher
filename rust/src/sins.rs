use crate::types::{ByteVec, ProgressBar, Slot};
use crate::utils::*;
use anyhow::{ensure, Context, anyhow};
use flate2::read::GzDecoder;
use std::ffi::CStr;
use std::fs::File;
use std::io::{Read, Seek};
use std::path::{Path};
use log::{debug, error, info};
use tar::{Archive, Entry, EntryType};
use crate::fastboot::{FastbootDevice, FastbootHeader};

pub fn process_sins(
    usb: &mut FastbootDevice,
    sin_path: &Path,
    fb_end_cmd: &str,
    target_slot: Slot,
) -> anyhow::Result<()> {
    let flash_prefix = validate_prefix(sin_path)?;

    let flash_both_slots = flash_prefix == "bootloader" || flash_prefix == "bluetooth" || flash_prefix == "dsp" || flash_prefix == "modem" || flash_prefix == "rdimage";

    process_sins_slot(usb, &sin_path, fb_end_cmd, flash_prefix.as_str(), target_slot, flash_both_slots)?;

    if flash_both_slots {
        process_sins_slot(usb, &sin_path, fb_end_cmd, flash_prefix.as_str(), target_slot.other(), flash_both_slots)?;
    }

    Ok(())
}

fn process_sins_slot(
    usb: &mut FastbootDevice,
    sin_path: &Path,
    fb_end_cmd: &str,
    flash_prefix: &str,
    target_slot: Slot,
    flash_both_slots: bool,
) -> anyhow::Result<()> {
    // TODO: move to main
    let mut keep_userdata: bool = true;

    let file_name = sin_path.file_name().unwrap().to_string_lossy();

    if keep_userdata && noerase_in_updatexml(sin_path.file_name().unwrap()) {
        info!("Skipping {}", file_name);
        return Ok(());
    }

    // info!("Processing {}", sin_path.to_string_lossy());

    let has_slot = fb_end_cmd == "flash" && {
        let cmd = format!("getvar:has-slot:{flash_prefix}");
        match usb.command(cmd.as_str()) {
            Ok(_) => usb.reply == b"yes",
            Err(e) => {
                error!("{e}");
                false
            }
        }
    };

    let prefix = match file_name.split_once('_') {
        Some((first, _)) => first,
        None => file_name.as_ref(),
    };

    let parts = open_sin_archive(&sin_path)?
        .entries()?
        .count();

    let text = if flash_both_slots {
        format!("Processing {prefix} ({target_slot})")
    } else {
        format!("Processing {prefix}")
    };

    let progress = ProgressBar::new(parts as u64, text);

    for (i, mut entry) in open_sin_archive(&sin_path)?
        .entries()?
        .into_iter()
        .flatten()
        .filter(|e| filter_entry(e))
        .enumerate() {
        let entry_name = CStr::from_bytes_until_nul(&entry.header().as_ustar().unwrap().name)?.to_string_lossy().to_string();
        let other = sin_path.to_string_lossy().contains("_other");

        if i == 0 {
            download_cms(usb, &mut entry, &entry_name).with_context(|| format!("Failed to transfer '{entry_name}' cms"))?;
        } else {
            debug!("Uploading sparse chunk {}", entry_name);
            usb.download_tar_entry(&mut entry).with_context(|| format!("Failed to transfer '{entry_name}' chunk {i}"))?;

            // erase partition
            if i == 1 && fb_end_cmd == "flash" {
                let cmd = match (has_slot, other) {
                    (false, _) => format!("erase:{flash_prefix}"),
                    (true, false) => format!("erase:{flash_prefix}_{target_slot}"),
                    (true, true) => format!("erase:{flash_prefix}_{}", target_slot.other()),
                };

                debug!("  {cmd}");
                usb.command_expect(&cmd, FastbootHeader::Okay)?;
            }

            /* Oreo changed partition image name, so this is a quick fix */
            if fb_end_cmd == "Repartition" && flash_prefix.starts_with("partitionimage_") {
                let cmd = flash_prefix.replace("partitionimage_", "Repartition:");
                debug!("{cmd}");
                usb.command_expect(&cmd, FastbootHeader::Okay)?;
            } else {
                let cmd = match (has_slot, other) {
                    (false, _) => format!("{fb_end_cmd}:{flash_prefix}"),
                    (true, false) => format!("{fb_end_cmd}:{flash_prefix}_{target_slot}"),
                    (true, true) => format!("{fb_end_cmd}:{flash_prefix}_{}", target_slot.other()),
                };

                usb.command_expect(&cmd, FastbootHeader::Okay)?;
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
            let name = CStr::from_bytes_until_nul(&entry.header().as_ustar()?.name).ok()?.to_str().ok()?;
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

fn download_cms(usb: &mut FastbootDevice, entry: &mut Entry<Box<dyn Read>>, entry_name: &str) -> anyhow::Result<()> {
    let mut is_2021_device: bool = false;

    let hex_len = ByteVec::from_len(entry.size() as usize);
    info!("Downloading signature {entry_name}");

    let cstr = CStr::from_bytes_until_nul(&entry.header().as_ustar().unwrap().name)?.to_string_lossy();
    ensure!(cstr == entry_name, "Invalid cms string!");

    let cmd = format!("signature:{hex_len}");
    debug!("  {cmd}");

    if usb.write_and_read_reply(cmd).context("Error writing signature command!")? == FastbootHeader::Fail {
        is_2021_device = true;
        info!("device from 2021 and up?");

        let cmd = format!("download:{hex_len}");
        debug!("  {cmd}");

        usb.write_and_read_reply(cmd).context("Error writing signature command!")?;
    }

    ensure!(hex_len == usb.reply, format!("Invalid DATA reply string, Expected {hex_len}, received {}", usb.reply));

    let mut buf = vec![];
    entry.read_to_end(&mut buf)?;

    // let written = std::io::copy(entry, &mut usb.writer).context("Error writing signature!")?;
    // usb.writer.flush_end()?;
    usb.write(&buf)?;

    // println!("      Wrote {} bytes", written);

    let header = usb.read_reply()?;
    ensure!(header == FastbootHeader::Okay, format!("Invalid header! Expected OKAY, received: {header:?}"));

    if is_2021_device {
        usb.write_and_expect_reply(b"signature", FastbootHeader::Okay)?;
    }

    Ok(())
}
