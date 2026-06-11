use core::ffi::c_void;
use std::ffi::{CStr, OsString};
use std::fs::{File};
use std::io::{BufRead, BufReader, ErrorKind, Read, Seek, Write};
use std::os::raw::{c_char, c_uint};
use std::path::{Path, PathBuf};
use flate2::read::GzDecoder;
use log::error;
use tar::{Archive, EntryType};
use crate::*;
use crate::utils::*;

unsafe extern "C" {
    static mut keep_userdata: bool;
    static mut current_slot: *mut c_char;
}

//
// #[unsafe(no_mangle)]
// unsafe fn process_sins_fii (
//     dev: HANDLE,
//     a: *mut libc::FILE,
//     filename: *mut c_char,
//     full_path: *mut c_char,
//     outfolder: *mut c_char,
//     endcommand: *mut c_char,
// ) -> c_int {
//     unsafe {
//         process_sins_rs(dev,
//                         a,
//                         PathBuf::from(CStr::from_ptr(filename).to_str().unwrap()),
//                         PathBuf::from(CStr::from_ptr(full_path).to_str().unwrap()),
//                         PathBuf::from(CStr::from_ptr(outfolder).to_str().unwrap()), endcommand) as c_int
//     }
// }

fn process_sins_rs (
    usb: &mut UsbInterfaces,
    // mut decompressed_sin: File, // ./partition/converted.file
    sin_path: PathBuf, // ./partition/partition-image-LUN0_124936192_X-FLASH-ALL-88DF.sin
    _working_dir: PathBuf,
    // out_dir: PathBuf, // partition
    fb_end_cmd: &str, // Repartition
) -> Result<(), Box<dyn std::error::Error>> {
    if !fb_end_cmd.is_ascii() {
        panic!();
    }

    // TODO: remove, and use argument
    let mut reply = Vec::<u8>::new();

    let magic_numbers = [0u8; 2];
    let mut sin_file = File::open(&sin_path)?;

    sin_file.read_exact(&mut reply)?;
    sin_file.rewind()?;

    let mut has_slot = false;

    let reader: Box<dyn Read> = match magic_numbers {
        [0x1F, 0x8B] => Box::new(GzDecoder::new(sin_file)),
        _ =>  Box::new(sin_file)
    };

    let mut archive = Archive::new(reader);

    let mut file_found_in_updatexml: bool = false;

    let base_fn = sin_path.file_name().unwrap().to_str().unwrap();

    let working_path = std::env::current_dir()?;

    let update_xml_path = working_path.join("update.xml");

    if update_xml_path.is_file() {
        if unsafe { keep_userdata } {
            file_found_in_updatexml = check_in_updatexml_rs(&update_xml_path, base_fn);
        }
    }

    if file_found_in_updatexml {
        println!(" - Skipping {}", base_fn);
        return Ok(());
    }

    println!(" - Extracting from {}", base_fn);

    let mut entries = Vec::new();

    for entry in archive.entries()? {
        let entry = entry?;
        match entry.header().entry_type() {
            EntryType::Regular | EntryType::Continuous => entries.push(entry),
            et => println!(" - Ignoring {:?}", et),
        }
    }

    let mut entry_prefix_opt: Option<String> = None;

    for entry in entries.iter() {
        if entry.size() == 0 {
            return Err(Box::new(io::Error::new(ErrorKind::InvalidData, "tar entry contained 0 bytes!".to_string())));
        };

        let name = CStr::from_bytes_until_nul(&entry.header().as_ustar().unwrap().name)?;
        let prefix = Path::new(name.to_string_lossy().as_ref()).with_extension("").to_string_lossy().to_string();

        match &entry_prefix_opt {
            None => entry_prefix_opt = Some(prefix),
            Some(prefix_) => if *prefix_ != prefix {
                return Err(Box::new(io::Error::new(ErrorKind::InvalidData, format!("Mismatched tar entry name! Expected {prefix_}, got {prefix}"))));
            }
        }
    }

    let flash_prefix = entry_prefix_opt.unwrap();

    for (i, mut entry) in entries.into_iter().enumerate() {
        let file_size = entry.size();
        let entry_name = CStr::from_bytes_until_nul(&entry.header().as_ustar().unwrap().name)?.to_string_lossy().to_string();
        if i == 0 {
            let mut is_2021_device: bool = false;

            let hex_len = format!("{:08X}", file_size);
            println!(" - Uploading signature {}", entry_name);

            loop { // repeat_here
                let cmd_str = if is_2021_device {
                    format!("download:{}", hex_len)
                } else {
                    format!("signature:{}", hex_len)
                };

                println!("      {}", cmd_str);

                if !cmd_str.is_ascii() {
                    error!("     - Invalid command string: {}", cmd_str);
                }

                if output_input_bulk(usb, &mut reply, cmd_str.as_bytes()).expect("      Error writing signature command!") == FastbootHeader::Fail && !is_2021_device {
                    println!("      device from 2021 and up?");
                    is_2021_device = true;
                    continue; // goto repeat_here
                }
                break;
            }

            if hex_len.as_bytes().ne(&reply) {
                return Err(Box::new(io::Error::new(ErrorKind::InvalidData, format!("Invalid DATA reply string, Expected {hex_len:?}, received {reply:?}"))));
                // println!("      Error, signature DATA reply string unexpected!");
            }

            (&mut entry).take(16384);
            std::io::copy(&mut entry, &mut usb.writer).expect("Error writing signature command!");
            usb.writer.flush().expect("Error flushing signature command");

            let mut buffer = Vec::new();
            match get_reply(usb, &mut buffer) {
                Ok(header) => if header != FastbootHeader::Okay {
                    return Err(Box::new(io::Error::new(ErrorKind::InvalidData,format!("Invalid header! Expected OKAY, received: {header:?}"))));
                }
                Err(e) => {
                    return Err(Box::new(e));
                }
            }

            println!("      OKAY.");

            if is_2021_device {
                bulk_transfer_expect(usb, &mut reply, b"signature", FastbootHeader::Okay)?;
                println!("      OKAY.");
            }
        } else {
            println!(" - Uploading sparse chunk {}", entry_name);

            // chunk file into smaller buffer
            fastboot_download_entry(usb, &mut entry)?;

            println!("      OKAY.");

            let slot = unsafe { CStr::from_ptr(current_slot) }.to_bytes();

            // erase partition
            if i == 1 && fb_end_cmd == "flash" {

                let mut erase_cmd = format!("erase:{flash_prefix}");

                if slot == b"a" || slot == b"b" {
                    let getvar_cmd = format!("getvar:has-slot:{flash_prefix}");
                    fastboot_cmd(usb, &mut reply, getvar_cmd.as_bytes()).unwrap_or_else(|e| panic!("Failed to execute {getvar_cmd}! {e}"));

                    has_slot = reply == b"yes";
                    if has_slot {
                        let is_other = entry_name.contains("_other");

                        let target_slot = match slot {
                            b"a" => if is_other { "b" } else { "a" },
                            b"b" => if is_other { "a" } else { "b" },
                            _ => panic!("This should be impossible to reach"),
                        };

                        erase_cmd.extend(["_", target_slot])
                    }
                }

                println!("      {erase_cmd}");

                bulk_transfer_expect(usb, &mut reply, erase_cmd.as_bytes(), FastbootHeader::Okay)?;
            }

            let mut command = String::new();

            /* Oreo changed partition image name, so this is a quick fix */
            if fb_end_cmd == "Repartition" && flash_prefix.starts_with("partitionimage_") {
                command = flash_prefix.replace("partitionimage", "Repartition");
            }
            else {
                command = format!("{fb_end_cmd}:{flash_prefix}");

                if has_slot {
                    let is_other = entry_name.contains("_other");

                    let target_slot = match slot {
                        b"a" => if is_other { "b" } else { "a" },
                        b"b" => if is_other { "a" } else { "b" },
                        _ => panic!("This should be impossible to reach"),
                    };

                    command.extend(["_", target_slot])
                }
            }

            println!("      {command}");

            bulk_transfer_expect(usb, &mut reply, command.as_bytes(), FastbootHeader::Okay)?;

            println!("      OKAY.");
        }
    }

    Ok(())
}


pub fn check_in_updatexml_rs(xml_file: &Path, searchfor: &str) -> bool {
    let file = match File::open(xml_file) {
        Ok(f) => f,
        Err(e) => { error!("{}", e); return false; },
    };

    let reader = BufReader::new(file);

    for line in reader.lines().into_iter() {
        match line {
            Ok(mut str) => {
                if !str.is_empty() {
                    str = trim_rs(&str);
                    if str.contains("<NOERASE>") && str.contains(searchfor)
                    {
                        println!("{}", str);
                        return true;
                    }
                }
            }
            Err(e) => {
                error!("{}", e);
                return false;
            }
        }
    }

    false
}