use anyhow::anyhow;
use log::{Level, error, log_enabled, trace};
use std::ffi::OsStr;
use std::fmt::Write;
use std::fs::{DirEntry, File};
use std::io::{BufRead, BufReader, Read};
use std::path::PathBuf;
use tar::Entry;

pub fn trace_formatted_hex(message: &str, buffer: &[u8]) {
    if log_enabled!(Level::Trace) {
        trace!("{}", formatted_hex(message, buffer));
    }
}

pub fn formatted_hex(message: &str, buffer: &[u8]) -> String {
    let mut builder = String::with_capacity(0xF00);

    let _ = writeln!(builder, "{}[0x{:X}]:", message, buffer.len());

    buffer.chunks(16)
          .take(64)
          .enumerate()
          .for_each(|(i, chunk)| {
              let _ = writeln!(builder, "0x{i:07X}0  {:<48} {}", chunk.iter().map(|b| format!("{b:02X} ")).collect::<String>(), u8_ascii(chunk));
          });

    builder
}

pub fn u8_ascii(line: &[u8]) -> String {
    line.iter()
        .map(|b| match *b >= 33 && *b < 127 {
            true => *b as char,
            false => '.',
        }).collect::<String>()
}

pub fn is_sin_file(entry: std::io::Result<DirEntry>) -> Option<PathBuf> {
    let path = entry.ok()?.path();
    if path.extension()? == "sin" {
        Some(path)
    } else {
        None
    }
}

pub fn is_ta_file(entry: std::io::Result<DirEntry>) -> Option<PathBuf> {
    let path = entry.ok()?.path();
    if path.extension()? == "ta" {
        Some(path)
    } else {
        None
    }
}

pub fn noerase_in_updatexml(file_name: &OsStr) -> bool {
    let search_for = file_name.to_str().unwrap();

    let file = match File::open("update.xml") {
        Ok(f) => f,
        Err(e) => {
            error!("{}", e);
            return false;
        },
    };

    let reader = BufReader::new(file);

    for line in reader.lines().into_iter() {
        match line {
            Ok(str) => {
                if !str.is_empty() && str.trim() == format!("<NOERASE>{search_for}</NOERASE>") {
                    return true;
                }
            }
            Err(e) => {
                error!("{}", e);
            }
        }
    }

    false
}

pub fn tar_entry_header_name<'a>(entry: &'a Entry<Box<dyn Read>>) -> anyhow::Result<&'a str> {
    let header = entry.header().as_ustar().ok_or_else(|| anyhow!("Failed to parse Tar Entry header as ustar"))?;
    Ok(str::from_utf8(&header.name)?)
}