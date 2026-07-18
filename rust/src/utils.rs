use std::ffi::{c_char, CStr};
use std::fs::File;
use std::path::Path;
use std::slice;

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
