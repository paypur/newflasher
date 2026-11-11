use crate::fastboot::{FastbootDevice, Header};
use crate::types::{ByteVec, ProgressBar, Slot};
use crate::utils::*;
use anyhow::{anyhow, ensure, Context};
use flate2::read::GzDecoder;
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

        let mut prefix = None;
        let other = path.to_string_lossy().contains("_other");
        let mut cms = Vec::<u8>::new();
        let mut chunks = Vec::<Vec<u8>>::new();

        for (i, entry) in open_sin_archive(path)?
            .entries()?
            .enumerate() {
            let mut entry = entry.with_context(|| format!("Failed to read tar entry from {}", path.display()))?;

            if !filter_entry(&entry) {
                continue;
            }

            match i {
                0 => {
                    let _ = prefix.insert(Self::entry_prefix(&entry)?);
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


        Ok(Self { prefix: prefix.ok_or_else(|| anyhow!("Failed to get prefix from {}", path.display()))?, other, cms, chunks })
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
        let mut progress = ProgressBar::new(parts, format!("{prefix} ({target_slot})"));

        progress.step(|| {
            self.download_cms(usb)?;

        progress.step(|| self.download_cms(usb))?;

        for (i, chunk) in self.chunks.iter().enumerate() {
            info!("Downloading sparse chunk {prefix}.{i}");
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

        Ok(progress.okay())
    }

    /// Attempts to write cms to device
    fn download_cms(&self, dev: &mut FastbootDevice) -> anyhow::Result<()> {
        info!("Downloading signature {}", self.prefix);

        let hex_len = ByteVec::from_len(self.cms.len() as u32);

        let mut res = dev.command(match dev.is_2021_or_newer {
            true => format!("download:{hex_len}"), // seems to work on older devices anyways
            false => format!("signature:{hex_len}")
        })?;

        match res.header {
            Header::Okay | Header::Data => (),
            Header::Fail if dev.is_2021_or_newer => {
                dev.is_2021_or_newer = false;
                res = dev.command(format!("signature:{hex_len}"))?;
            },
            h => anyhow::bail!("Unexpected command response: {h}: {}", res.data)
        }

        ensure!(hex_len == res.data, "Invalid DATA reply string, Expected {hex_len}, received {}", res.data);

        let _ = dev.write_and_expect_header(&self.cms, Header::Okay).context("Failed to write cms! cms is possibly corrupt.")?;

        if dev.is_2021_or_newer {
            let _ = dev.write_and_expect_header("signature", Header::Okay)?;
        }

        Ok(())
    }

    /// Returns tar entry name without the extension
    fn entry_prefix(entry: &Entry<Box<dyn Read>>) -> anyhow::Result<String> {
        let path = Path::new(tar_entry_header_name(&entry)?).with_extension("");
        let str = path.to_str().context("Failed convert path back into string")?;
        Ok(str.to_owned())
    }
}

pub fn parse_all_cwd() -> anyhow::Result<Vec<Sin>> {
    let sins = parse_all(fs::read_dir("./")?.filter_map(is_sin_file))?;
    info!("Found {} .sin files", sins.len());
    Ok(sins)
}

pub fn parse_all(paths: impl IntoIterator<Item = PathBuf>) -> anyhow::Result<Vec<Sin>> {
    paths.into_iter()
        .filter(|p| {
            if noerase_in_updatexml(p.file_name().unwrap()) || p.file_name().unwrap().to_string_lossy().contains("persist") {
                info!("Skipping {}", p.display());
                return false;
            }
            true
        })
        .map(Sin::open)
        .collect::<anyhow::Result<Vec<Sin>>>()
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