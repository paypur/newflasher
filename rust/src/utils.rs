use std::fmt::Write;
use std::ffi::{c_char, CStr};
use std::fs::{DirEntry, File};
use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};
use std::slice;
use log::{log_enabled, trace, Level, error, debug};

#[unsafe(no_mangle)]
pub extern "C" fn file_exist(ptr: *const c_char) -> i32 {
    let file = unsafe { CStr::from_ptr(ptr) }.to_string_lossy();
    let path = Path::new(file.as_ref());
    path.is_file() as i32
}

#[unsafe(no_mangle)]
pub extern "C" fn file_size(ptr: *const c_char) -> u32 {
    let file_name = unsafe { CStr::from_ptr(ptr) }.to_string_lossy();
    let path = Path::new(file_name.as_ref());
    if let Ok(file) = File::open(path) {
        if let Ok(meta) = file.metadata() {
            return meta.len() as u32;
        }
    }
    0
}

#[unsafe(no_mangle)]
pub extern "C" fn parseoct(ptr: *const c_char, n: usize) -> u32 {
    if ptr.is_null() || n == 0 {
        return 0;
    }

    parse_oct_rs(unsafe { slice::from_raw_parts(ptr as *const u8, n) })
}

fn parse_oct_rs(slice: &[u8]) -> u32 {
    slice.iter()
         .skip_while(|&&c| c < b'0' || c > b'7')
         .take_while(|&&c| c >= b'0' && c <= b'7')
         .fold(0, |acc, &c| (acc * 8) + (c - b'0') as u32)
}

// Verify tar checksum
#[unsafe(no_mangle)]
pub extern "C" fn verify_checksum(ptr: *const c_char) -> bool {
    let buf = unsafe { slice::from_raw_parts(ptr as *const u8, 512) };

    let mut sum: u32 = 0;

    for (i, b) in buf.iter().enumerate() {
        // Standard tar checksum adds unsigned bytes.
        if i < 148 || i > 155 {
            sum += *b as u32;
        } else {
            sum += 0x20;
        }
    }

    sum == parse_oct_rs(&buf[148..155])
}

#[unsafe(no_mangle)]
pub extern "C" fn is_end_of_archive(ptr: *const c_char) -> bool {
    for n in (0..512).rev() {
        if unsafe { *ptr.add(n) } != b'\0' as c_char {
            return false;
        }
    }
    true
}

pub fn trim_rs(string: &mut String) {
    string.retain(|c| c != ' ' && c != '\t' && c != '\n' && c != '\r');
}

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
            Ok(mut str) => {
                if !str.is_empty() {
                    trim_rs(&mut str);
                    if str == format!("<NOERASE>{search_for}</NOERASE>") {
                        debug!("{}", str);
                        return true;
                    }
                }
            }
            Err(e) => {
                error!("{}", e);
            }
        }
    }

    false
}