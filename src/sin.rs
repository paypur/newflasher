use crate::fastboot::{FastbootDevice, Header};
use crate::types::{ByteVec, ProgressBar, Slot};
use crate::utils::*;
use anyhow::{Context, anyhow, ensure};
use flate2::read::GzDecoder;
use indicatif::{ProgressIterator, ProgressStyle};
use log::{debug, error, info};
use std::fs;
use std::fs::File;
use std::io::{Read, Seek};
use std::path::{Path, PathBuf};
use tar::{Archive, Entry, EntryType};

pub struct Sin {
    pub prefix: String,
    other: bool,
    cms: Vec<u8>,
    chunks: Vec<Vec<u8>>,
}

impl Sin {
    pub fn open(path: impl AsRef<Path>) -> anyhow::Result<Self> {
        let path = path.as_ref();

        let prefix = Self::validate_prefix(path)?;
        let mut cms = Vec::<u8>::new();
        let mut chunks = Vec::<Vec<u8>>::new();

        for (i, mut entry) in open_sin_archive(path)?
            .entries()?
            .map(|e| e.unwrap())
            .filter(filter_entry)
            .enumerate() {
            match i {
                0 => {
                    cms.reserve_exact(entry.size() as usize);
                    entry.read_to_end(cms.as_mut())?;
                },
                _ => {
                    let mut chunk = Vec::<u8>::with_capacity(entry.size() as usize);
                    entry.read_to_end(&mut chunk)?;
                    chunks.push(chunk);
                }
            }
        }

        Ok(Self { prefix, other: path.to_str().unwrap().contains("_other"), cms, chunks })
    }

    pub fn open_all_cwd() -> anyhow::Result<Vec<Self>> {
        let sins = Sin::open_all(fs::read_dir("./")?.filter_map(is_sin_file).collect())?;
        info!("Found {} .sin files", sins.len());
        Ok(sins)
    }

    pub fn open_all(paths: Vec<PathBuf>) -> anyhow::Result<Vec<Self>> {
        let files = paths
            .iter()
            .filter(|p| {
                if noerase_in_updatexml(p.file_name().unwrap()) || p.file_name().unwrap().to_string_lossy().contains("persist") {
                    info!("Skipping {}", p.display());
                    return false;
                }
                true
            })
            .collect::<Vec<_>>();

        files.iter()
             .progress_with_style(ProgressStyle::default_bar().progress_chars("#*-"))
             .map(Sin::open)
             .collect::<anyhow::Result<Vec<Self>>>()
    }

    pub fn flash(&self, usb: &mut FastbootDevice, target_slot: Slot) -> anyhow::Result<()> {
        // let flash_prefix = validate_prefix(sin_path)?;

        // let flash_both_slots = flash_prefix == "bootloader" || flash_prefix == "bluetooth" || flash_prefix == "dsp" || flash_prefix == "modem" || flash_prefix == "rdimage";

        self.process_sins_slot(usb, target_slot, false)?;

        // TODO: flashing both slots is just not safe
        // if flash_both_slots {
        //     process_sins_slot(usb, sin_path, flash_prefix.as_str(), target_slot.other(), flash_both_slots)?;
        // }

        Ok(())
    }

    fn process_sins_slot(
        &self,
        usb: &mut FastbootDevice,
        target_slot: Slot,
        flash_both_slots: bool,
    ) -> anyhow::Result<()> {
        let prefix = self.prefix.as_str();

        info!("Processing {}", prefix);

        // TODO: is this assumption always true?
        let is_partition_image = self.prefix.starts_with("partitionimage_");

        let has_slot = !is_partition_image && {
            let cmd = format!("getvar:has-slot:{prefix}");
            match usb.command(cmd.as_str()) {
                Ok(res) => res.data == b"yes",
                Err(e) => {
                    error!("{e}");
                    false
                }
            }
        };

        let parts = 1 + self.chunks.len() as u64;
        let mut progress = ProgressBar::new(parts, format!("Flashing {prefix} ({target_slot})"));

        self.download_cms(usb)?;
        progress.inc();

        for (i, chunk) in self.chunks.iter().enumerate() {
            info!("Downloading sparse chunk {prefix}");

            usb.download(chunk).with_context(|| format!("Failed to transfer {prefix}, chunk {i} of {parts}"))?;

            if is_partition_image {
                let cmd = prefix.replace("partitionimage_", "Repartition:");
                usb.command_expect_header(cmd, Header::Okay)?;
            } else {
                if i == 0 {
                    let erase_cmd = match (has_slot, self.other) {
                        (false, _) => format!("erase:{prefix}"),
                        (true, false) => format!("erase:{prefix}_{target_slot}"),
                        (true, true) => format!("erase:{prefix}_{}", target_slot.other()),
                    };
                    usb.command_expect_header(erase_cmd, Header::Okay)?;
                }

                let flash_cmd = match (has_slot, self.other) {
                    (false, _) => format!("flash:{prefix}"),
                    (true, false) => format!("flash:{prefix}_{target_slot}"),
                    (true, true) => format!("flash:{prefix}_{}", target_slot.other()),
                };
                usb.command_expect_header(flash_cmd, Header::Okay)?;
            }

            progress.inc();
        }

        progress.okay();

        Ok(())
    }

    /// Attempts to write cms to device
    fn download_cms(&self, dev: &mut FastbootDevice) -> anyhow::Result<()> {
        // let entry_name = tar_entry_header_name(entry)?;
        info!("Downloading signature {}", self.prefix);

        let hex_len = ByteVec::from_len(self.cms.len() as u32);

        let mut res = dev.command(format!("signature:{hex_len}"))?;

        let is_2021_device: bool = res.header == Header::Fail;
        if is_2021_device {
            info!("device from 2021 and up?");
            res = dev.command(format!("download:{hex_len}"))?;
        }

        ensure!(hex_len == res.data, "Invalid DATA reply string, Expected {hex_len}, received {}", res.data);

        let _ = dev.write_and_expect_header(&self.cms, Header::Okay).context("Failed to write cms! cms is possibly corrupt.")?;

        if is_2021_device {
            let _ = dev.write_and_expect_header("signature", Header::Okay)?;
        }

        Ok(())
    }

    fn validate_prefix(path: &Path) -> anyhow::Result<String> {
        let mut archive = open_sin_archive(&path)?;

        let mut prefix_iter = archive.entries()?
            .map(|e| e.unwrap())
            .filter(filter_entry)
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