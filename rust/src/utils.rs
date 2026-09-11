use log::{Level, debug, error, log_enabled, trace};
use std::fmt::Write;
use std::fs::{DirEntry, File};
use std::io::{BufRead, BufReader};
use std::path::PathBuf;

pub fn trace_formatted_hex(message: &str, buffer: &[u8]) {
    if log_enabled!(Level::Trace) {
        let mut builder = String::with_capacity(0xF00);

        let _ = writeln!(builder, "{}:", message);

        buffer.chunks(16)
              .take(64)
              .enumerate()
              .for_each(|(i, chunk)| {
                  let _ = writeln!(builder, "0x{i:07X}0  {:<48} {}", chunk.iter().map(|b| format!("{b:02X} ")).collect::<String>(), u8_ascii(chunk));
              });

        trace!("{builder}");
    }
}

pub fn u8_ascii(line: &[u8]) -> String {
    line.iter()
        .map(|b| match *b as char {
            '\n' | '\r' | '\t' => ' ',
            c => c,
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

pub fn noerase_in_updatexml(search_for: &str) -> bool {
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
                    debug!("{}", str);
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