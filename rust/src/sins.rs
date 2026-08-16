use crate::types::{ByteVec, FastbootDevice, FastbootHeader, Slot};
use crate::utils::*;
use anyhow::{ensure, Context};
use flate2::read::GzDecoder;
use std::ffi::CStr;
use std::fs::File;
use std::io::{BufRead, BufReader, Read, Seek};
use std::path::{Path, PathBuf};
use tar::{Archive, Entry, EntryType};

pub fn process_sins(
    usb: &mut FastbootDevice,
    sin_path: PathBuf,
    fb_end_cmd: &str,
    curr_slot: Slot,
) -> anyhow::Result<()> {

    let flash_prefix = validate_prefix(sin_path.as_path())?;

    let flash_both_slots =  flash_prefix == "bootloader" || flash_prefix == "bluetooth" || flash_prefix == "dsp" || flash_prefix == "modem" || flash_prefix == "rdimage";

    process_sins_slot(usb, &sin_path, fb_end_cmd, flash_prefix.as_str(), curr_slot)?;

    // TODO: try to avoid reading the entry twice
    if flash_both_slots {
        process_sins_slot(usb, &sin_path, fb_end_cmd, flash_prefix.as_str(), curr_slot.other())?;
    }

    Ok(())
}

fn process_sins_slot(
    usb: &mut FastbootDevice,
    sin_path: &Path,
    fb_end_cmd: &str,
    flash_prefix: &str,
    target_slot: Slot,
) -> anyhow::Result<()> {
    if !fb_end_cmd.is_ascii() {
        panic!();
    }

    // TODO: move to main
    let mut keep_userdata: bool = true;

    let working_path = std::env::current_dir()?;

    let mut file_found_in_updatexml: bool = false;

    let base_fn = sin_path.file_name().unwrap().to_str().unwrap();

    let update_xml_path = working_path.join("update.xml");

    if update_xml_path.is_file() {
        if keep_userdata {
            file_found_in_updatexml = check_in_updatexml_rs(&update_xml_path, base_fn);
        }
    }

    if file_found_in_updatexml {
        println!(" Skipping {}", base_fn);
        return Ok(());
    }

    println!("Processing {}", base_fn);

    let has_slot = fb_end_cmd == "flash" && {
        let cmd = format!("getvar:has-slot:{flash_prefix}");
        match usb.command(cmd.as_str()) {
            Ok(_) => usb.reply == b"yes",
            Err(e) => {
                eprintln!("{e}");
                false
            }
        }
    };

    for (i, mut entry) in open_sin_archive(&sin_path)?.entries()?
        .into_iter()
        .flatten()
        .filter(|e| filter_entry(e))
        .enumerate() {
        let entry_name = CStr::from_bytes_until_nul(&entry.header().as_ustar().unwrap().name)?.to_string_lossy().to_string();
        if i == 0 {
            transfer_cms(usb, &mut entry, &entry_name)?;
        } else {
            println!("Uploading sparse chunk {}", entry_name);
            usb.download_tar_entry(&mut entry)?;

            // erase partition
            if i == 1 && fb_end_cmd == "flash" {
                let erase = format!("erase:{flash_prefix}");

                let erase_current = format!("{erase}_{}", target_slot);
                let erase_other = format!("{erase}_{}", target_slot.other());

                // if flash_both_slots && has_slot {
                //     println!("    {erase_current}");
                //     usb.write_and_expect_reply(erase_current.as_bytes(), FastbootHeader::Okay)?;
                //     println!("    {erase_other}");
                //     usb.write_and_expect_reply(erase_other.as_bytes(), FastbootHeader::Okay)?;
                // } else {
                    let cmd = match (has_slot, entry_name.contains("_other")) {
                        (false, _) => erase,
                        (true, true) => erase_other,
                        (true, false) => erase_current,
                    };

                    println!("    {cmd}");
                    usb.write_and_expect_reply(cmd.as_bytes(), FastbootHeader::Okay)?;
                // }
            }

            /* Oreo changed partition image name, so this is a quick fix */
            if fb_end_cmd == "Repartition" && flash_prefix.starts_with("partitionimage_") {
                let command = flash_prefix.replace("partitionimage_", "Repartition:");
                println!("    {command}");
                usb.write_and_expect_reply(command.as_bytes(), FastbootHeader::Okay)?;
                println!("    OKAY");
            } else {
/*                if flash_both_slots {
                    flash_entry(usb, fb_end_cmd, &flash_prefix, Some(target_slot))?;
                    // TODO: command not authenticated??
                    flash_entry(usb, fb_end_cmd, &flash_prefix, Some(target_slot.other()))?;
                } else*/
                if has_slot {
                    let target_slot = match entry_name.contains("_other") {
                        true => target_slot.other(),
                        false => target_slot
                    };
                    flash_entry(usb, fb_end_cmd, &flash_prefix, Some(target_slot))?;
                } else {
                    flash_entry(usb, fb_end_cmd, &flash_prefix, None)?;
                }
            }
        }
    }

    Ok(())
}



fn flash_entry(usb: &mut FastbootDevice, fb_end_cmd: &str, flash_prefix: &str, target: Option<Slot>) -> anyhow::Result<()> {
    let command = match target {
        Some(slot) => format!("{fb_end_cmd}:{flash_prefix}_{slot}"),
        None => format!("{fb_end_cmd}:{flash_prefix}")
    };

    println!("    {command}");
    usb.write_and_expect_reply(command.as_bytes(), FastbootHeader::Okay)?;
    println!("    OKAY");

    Ok(())
}

fn validate_prefix(sin_path: &Path) -> anyhow::Result<String> {
    let mut archive = open_sin_archive(&sin_path)?;

    let mut prefix_iter = archive.entries()?
        .flatten()
        .filter(|e| filter_entry(e))
        .map(|entry| {
            // ensure!(entry.size() != 0, "Tar entry contained 0 bytes!");
            let name = CStr::from_bytes_until_nul(&entry.header().as_ustar()?.name).ok()?;
            Some(Path::new(name.to_string_lossy().as_ref()).with_extension("").to_string_lossy().to_string())
        })
        .into_iter();

    let flash_prefix = match prefix_iter.next().flatten() {
        Some(prefix) => prefix,
        None => return Err(anyhow::Error::msg("Empty tar entry name!"))
    };

    if !prefix_iter.all(|opt| opt.map(|s| s == flash_prefix).is_some()) {
        return Err(anyhow::Error::msg("Mismatched tar entry name!"));
    }

    Ok(flash_prefix)
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
            println!("Ignoring {:?}", et);
            false
        }
    }
}

fn transfer_cms(usb: &mut FastbootDevice, entry: &mut Entry<Box<dyn Read>>, entry_name: &str) -> anyhow::Result<()> {
    let mut is_2021_device: bool = false;

    let hex_len = ByteVec::from_len(entry.size() as usize);
    println!("- Uploading signature: {}", entry_name);

    let cstr = CStr::from_bytes_until_nul(&entry.header().as_ustar().unwrap().name)?.to_string_lossy();
    ensure!(cstr == entry_name, "Invalid cms string!");

    let cmd = format!("signature:{hex_len}");
    println!("    {cmd}");

    if usb.write_and_read_reply(cmd.as_bytes()).context("Error writing signature command!")? == FastbootHeader::Fail {
        is_2021_device = true;
        println!("device from 2021 and up?");

        let cmd = format!("download:{hex_len}");
        println!("    {cmd}");

        usb.write_and_read_reply(cmd.as_bytes()).context("Error writing signature command!")?;
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

    println!("    OKAY.");

    if is_2021_device {
        usb.write_and_expect_reply(b"signature", FastbootHeader::Okay)?;
        println!("    OKAY.");
    }

    Ok(())
}

fn check_in_updatexml_rs(xml_file: &Path, searchfor: &str) -> bool {
    let file = match File::open(xml_file) {
        Ok(f) => f,
        Err(e) => {
            eprintln!("{}", e);
            return false;
        },
    };

    let reader = BufReader::new(file);

    for line in reader.lines().into_iter() {
        match line {
            Ok(mut str) => {
                if !str.is_empty() {
                    trim_rs(&mut str);
                    if str == format!("<NOERASE>{searchfor}</NOERASE>") {
                        println!("{}", str);
                        return true;
                    }
                }
            }
            Err(e) => {
                eprintln!("{}", e);
                return false;
            }
        }
    }

    false
}