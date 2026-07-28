use crate::utils::*;
use crate::*;
use flate2::read::GzDecoder;
use log::error;
use std::ffi::{CStr};
use std::fs::File;
use std::io::{BufRead, BufReader, ErrorKind, Read, Seek, Write};
use std::os::raw::{c_char};
use std::path::{Path, PathBuf};
use anyhow::{ensure, Context};
use tar::{Archive, Entry, EntryType};

unsafe extern "C" {
    static current_slot: [u8; 2];
}

#[unsafe(no_mangle)]
pub extern "C" fn process_sins_ffi(device_ptr: *mut FastbootDeviceFFI, filename: *mut c_char, endcommand: *mut c_char) -> bool {
    unsafe {
        let mut usb: FastbootDevice = device_ptr.into();

        let file_path = PathBuf::from(CStr::from_ptr(filename).to_string_lossy().as_ref());
        let cmd = CStr::from_ptr(endcommand).to_string_lossy();

        if let Err(e) = process_sins_rs(&mut usb, file_path, cmd.as_ref(), if current_slot[0] == b'b' { Slot::B } else { Slot::A }) {
            eprintln!("{}", e);
            ptr::write(device_ptr, FastbootDeviceFFI::from(usb));
            return false;
        };

        ptr::write(device_ptr, FastbootDeviceFFI::from(usb));
        true
    }
}

pub fn process_sins_rs(
    usb: &mut FastbootDevice,
    sin_path: PathBuf,
    fb_end_cmd: &str,
    curr_slot: Slot,
) -> anyhow::Result<()> {
    if !fb_end_cmd.is_ascii() {
        panic!();
    }

    let mut keep_userdata: bool = true;

    let working_path = std::env::current_dir()?;
    // println!("cwd {}", working_path.to_string_lossy());

    let prefix = sin_path.file_prefix().unwrap().to_str().unwrap();
    // TODO: needs to be used
    let flash_both_slots = prefix == "bootloader" || prefix == "bluetooth" || prefix == "dsp" || prefix == "modem" || prefix == "rdimage";

    let mut magic_numbers = [0u8; 2];
    let mut sin_file = File::open(&sin_path)?;

    sin_file.read_exact(magic_numbers.as_mut_slice())?;
    sin_file.rewind()?;

    let mut has_slot = false;

    let mut file_found_in_updatexml: bool = false;

    let base_fn = sin_path.file_name().unwrap().to_str().unwrap();


    let update_xml_path = working_path.join("update.xml");

    if update_xml_path.is_file() {
        if keep_userdata {
            file_found_in_updatexml = check_in_updatexml_rs(&update_xml_path, base_fn);
        }
    }

    if file_found_in_updatexml {
        println!(" - Skipping {}", base_fn);
        return Ok(());
    }

    println!("Processing {}", base_fn);

    let flash_prefix = {
        let reader: Box<dyn Read> = match magic_numbers {
            [0x1F, 0x8B] => Box::new(GzDecoder::new(sin_file)),
            _ => Box::new(sin_file)
        };

        let mut archive = Archive::new(reader);

        let mut prefix_iter = archive.entries()?
            // TODO: move into fn
            .filter_map(|entry| {
                let e = entry.ok()?;

                if e.size() == 0 {
                    return None
                }

                match e.header().entry_type() {
                    EntryType::Regular | EntryType::Continuous => Some(e),
                    et => {
                        println!(" - Ignoring {:?}", et);
                        None
                    }
                }
            })
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

        flash_prefix
    };

    // need to reopen the archive and create a new iterator
    // since we used the iterator already
    let sin_file = File::open(&sin_path)?;

    let reader: Box<dyn Read> = match magic_numbers {
        [0x1F, 0x8B] => Box::new(GzDecoder::new(sin_file)),
        _ => Box::new(sin_file)
    };

    let mut archive = Archive::new(reader);

    for (i, mut entry) in archive.entries()?
        .into_iter()
        .flatten()
        .filter(|e| match e.header().entry_type() {
            EntryType::Regular | EntryType::Continuous => true,
            et => {
                println!("Ignoring {:?}", et);
                false
            }
        })
        .enumerate() {
        let entry_name = CStr::from_bytes_until_nul(&entry.header().as_ustar().unwrap().name)?.to_string_lossy().to_string();
        if i == 0 {
            transfer_cms(usb, &mut entry, &entry_name)?;
        } else {
            println!("Uploading sparse chunk {}", entry_name);

            usb.download_tar_entry(&mut entry)?;

            // erase partition
            if i == 1 && fb_end_cmd == "flash" {
                let mut erase_cmd = format!("erase:{flash_prefix}");

                let getvar_cmd = format!("getvar:has-slot:{flash_prefix}");
                usb.command(getvar_cmd.as_str()).unwrap_or_else(|e| panic!("Failed to execute {getvar_cmd}! {e}"));

                has_slot = usb.reply == b"yes";
                if has_slot {
                    let target_slot = if entry_name.contains("_other") {
                        curr_slot.other()
                    } else {
                        curr_slot
                    };
                    erase_cmd.extend(["_", target_slot.into()])
                }

                println!("    {erase_cmd}");

                usb.write_and_expect_reply(erase_cmd.as_bytes(), FastbootHeader::Okay)?;
            }

            let mut command : String;

            /* Oreo changed partition image name, so this is a quick fix */
            if fb_end_cmd == "Repartition" && flash_prefix.starts_with("partitionimage_") {
                command = flash_prefix.replace("partitionimage_", "Repartition:");
            } else {
                command = format!("{fb_end_cmd}:{flash_prefix}");

                if has_slot {
                    let target_slot = if entry_name.contains("_other") {
                        curr_slot.other()
                    } else {
                        curr_slot
                    };
                    command.extend(["_", target_slot.into()])
                }
            }

            println!("    {command}");

            usb.write_and_expect_reply(command.as_bytes(), FastbootHeader::Okay)?;

            println!("    OKAY");
        }
    }

    Ok(())
}

pub fn transfer_cms(usb: &mut FastbootDevice, entry: &mut Entry<Box<dyn Read>>, entry_name: &str) -> anyhow::Result<()> {
    let mut is_2021_device: bool = false;

    let hex_len = ByteVec::from_len(entry.size() as usize);
    println!("- Uploading signature: {}", entry_name);

    let cstr = CStr::from_bytes_until_nul(&entry.header().as_ustar().unwrap().name)?.to_string_lossy();
    // let string = format!("{entry_name}.cms");
    ensure!(cstr == entry_name, "Invalid cms string!");

    loop { // repeat_here
        let cmd_str = if is_2021_device {
            format!("download:{}", hex_len)
        } else {
            format!("signature:{}", hex_len)
        };

        println!("    {}", cmd_str);

        if !cmd_str.is_ascii() {
            eprintln!("    Invalid command string: {}", cmd_str);
        }

        if usb.write_and_read_reply(cmd_str.as_bytes()).expect("    Error writing signature command!") == FastbootHeader::Fail && !is_2021_device {
            println!("    device from 2021 and up?");
            is_2021_device = true;
            continue; // goto repeat_here
        }
        break;
    }

    ensure!(hex_len == usb.reply, format!("Invalid DATA reply string, Expected {hex_len:?}, received {}", usb.reply));

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

pub fn check_in_updatexml_rs(xml_file: &Path, searchfor: &str) -> bool {
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
                    if str.contains("<NOERASE>") && str.contains(searchfor)
                    {
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