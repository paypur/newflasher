mod tests;
mod types;

use nusb::{Device, Interface, MaybeFuture};
use std::ffi::{c_char, c_ushort, CStr};
use std::fmt::format;
use std::fs::File;
use std::io::{BufRead, Read, Write};

use std::os::fd::RawFd;
use std::path::Path;
use std::{io, ptr, slice};
use std::cmp::min;
use std::error::Error;
use log::error;
use crate::types::*;

// C globals and functions
unsafe extern "C" {
    pub fn display_buffer_hex_ascii(message: *const c_char, buffer: *const c_char, size: usize);
}

#[unsafe(no_mangle)]
pub extern "C" fn new_cvec(capacity: usize) -> CVec {
    into_cvec(Vec::with_capacity(capacity))
}

fn into_cvec(vec: Vec<u8>) -> CVec {
    let (ptr, len, capacity) = vec.into_raw_parts();
    CVec { ptr, len, capacity }
}

fn from_cvec(cvec: &CVec) -> Vec<u8> {
    unsafe { Vec::from_raw_parts(cvec.ptr, cvec.len, cvec.capacity) }
}

fn return_vec(cvec: &mut CVec, vec: Vec<u8>) {
    let (ptr, len, capacity) = vec.into_raw_parts();
    cvec.ptr = ptr;
    cvec.len = len;
    cvec.capacity = capacity;
}


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
pub extern "C" fn get_flash_mode(vid: c_ushort, pid: c_ushort) -> *mut UsbHandle {
    let usb = get_flash_mode_rs(vid, pid);

    let raw_fd = unsafe { *(&usb.device as *const Device as *const RawFd) };

    // technically a memory leak, but we only call this function once
    let handle = Box::new(UsbHandle::new(raw_fd, usb));
    Box::into_raw(handle)
}

pub fn get_flash_mode_rs(vid: u16, pid: u16) -> UsbInterfaces {
    let di = nusb::list_devices()
        .wait()
        .unwrap()
        .find(|d| d.vendor_id() == vid && d.product_id() == pid)
        .expect("Failed to find device at /dev/bus/usb! Device should be connected via flash mode (green *)");

    let device: Device = di.open().wait().expect(format!("Failed to open device (VID: {:?}, PID: {:?})", vid, pid).as_str());
    let interface: Interface = device.claim_interface(0).wait().unwrap();

    UsbInterfaces::new(device, interface)
}

#[unsafe(no_mangle)]
pub extern "C" fn transfer_bulk_ffi(unsafe_handle: *mut UsbHandle, ep: i32, chars: *mut u8, len: usize, capacity: usize, exact: i32) -> usize {
    let handle = unsafe { &mut *unsafe_handle };

    match ep {
        0 => {
            let mut vec = unsafe { Vec::from_raw_parts(chars, len, capacity) };
            let res = input_bulk(&mut handle._usb, &mut vec);
            let _ = vec.into_raw_parts(); // make sure rust doesnt drop this
            res
        },
        1 => output_bulk(&mut handle._usb, unsafe { slice::from_raw_parts(chars, len) }),
        _ => panic!("Invalid endpoint direction: {:?}", ep)
    }.unwrap_or_else(|_| 0)
}

#[unsafe(no_mangle)]
pub extern "C" fn get_reply_ffi(unsafe_handle: *mut UsbHandle, cvec: &mut CVec, exact: i32) -> FastbootHeader {
    let handle = unsafe { &mut *unsafe_handle };
    let mut vec = from_cvec(cvec);
    let res = get_reply(&mut handle._usb, &mut vec);
    return_vec(cvec, vec);
    res.unwrap_or_else(|_| FastbootHeader::Error)
}

#[unsafe(no_mangle)]
pub extern "C" fn fastboot_cmd_ffi(usb_handle: *mut UsbHandle, cvec: &mut CVec, cmd: *const c_char, str: *mut u8, mut len: usize) -> bool {
    let usb = &mut unsafe { &mut *usb_handle }._usb;
    let mut buffer = from_cvec(cvec);
    let cstr = unsafe { CStr::from_ptr(cmd) };

    if let Err(e) = fastboot_cmd(usb, &mut buffer, cstr.to_bytes()) {
        error!("{}", e);
        return false;
    }

    if str as *const u8 != ptr::null() && len != 0 {
        len = len.min(buffer.len());

        let string = unsafe { slice::from_raw_parts_mut(str, len) };
        string[..len].clone_from_slice(&buffer.as_slice()[..len]);

        // write the null terminator for C strings
        string[min(len, string.len() - 1)] = 0;
    }

    return_vec(cvec, buffer);
    true
}

#[unsafe(no_mangle)]
pub extern "C" fn getvar_u32_ffi(usb_handle: *mut UsbHandle, cvec: &mut CVec, cmd: *const c_char, fallback: u32) -> u32 {
    let usb = &mut unsafe { &mut *usb_handle }._usb;
    let mut buffer = from_cvec(cvec);

    let u = getvar_u32(usb, &mut buffer, unsafe { CStr::from_ptr(cmd) }.to_bytes(), fallback);
    return_vec(cvec, buffer);
    u
}

pub fn fastboot_cmd(usb: &mut UsbInterfaces, buffer: &mut Vec<u8>, cmd: &[u8]) -> Result<(), Box<dyn Error>> {
    if cmd.starts_with(b"getvar:") || cmd.starts_with(b"download:") {
        transfer(usb, buffer, cmd)?;
    }
    else if (cmd.starts_with(b"Get-")) {
        output_bulk(usb, cmd)?;

        let header = get_reply(usb, buffer)?;
        if header != FastbootHeader::Data || buffer.len() != 8 {
            return Err(io::Error::new(io::ErrorKind::InvalidData, format!("Expected DATA header, received {:?}", header)).into());
        }

        let len = usize::from_str_radix(str::from_utf8(&buffer).expect("Failed to parse buffer as str"), 16).expect("Failed to parse str as hexadecimal");

        // second read for actual data
        input_bulk(usb, buffer)?;
        if FastbootHeader::from(buffer.as_slice()) == FastbootHeader::Okay { todo!("unimplemented OKAY after DATA") }
        assert_eq!(len, buffer.len());

        // if not OKAY in 2nd read, should read a 3rd time to acknowledge
        assert_eq!(get_reply(usb, &mut vec![0, 0, 0, 0])?, FastbootHeader::Okay);
    }
    else {
        panic!("Fastboot command prefix '{}' not found", str::from_utf8(cmd).unwrap());
    }

    Ok(())
}

/// Tires to write data to device
pub fn fastboot_download(usb: &mut UsbInterfaces, buffer: &mut Vec<u8>, data: &[u8]) -> Result<(), Box<dyn Error>> {
    let mut full_cmd = b"download:".to_vec();
    let string = format!("{:08X}", data.len());
    let hex_len = string.as_bytes();
    full_cmd.extend_from_slice(hex_len);

    let header = transfer(usb, buffer, &mut full_cmd)?;
    if header != FastbootHeader::Data {
        return Err(io::Error::new(io::ErrorKind::InvalidData, format!("Expected DATA header, received {:?}", header)).into());
    }
    if buffer.ne(&hex_len) {
        return Err(io::Error::new(io::ErrorKind::InvalidData, format!("Expected {:?}, received {:?}", hex_len, buffer)).into());
    }

    let header = transfer(usb, buffer, data)?;
    if header != FastbootHeader::Okay {
        return Err(io::Error::new(io::ErrorKind::InvalidData, format!("Expected OKAY header, received {:?}", header)).into());
    }

    Ok(())
}

pub fn transfer(usb: &mut UsbInterfaces, buffer: &mut Vec<u8>, var: &[u8]) -> Result<FastbootHeader, Box<dyn Error>> {
    output_bulk(usb, var)?;
    get_reply(usb, buffer)
}

pub fn getvar_u32(usb: &mut UsbInterfaces, buffer: &mut Vec<u8>, var: &[u8], default: u32) -> u32 {
    buffer.clear();

    if let Err(e) = output_bulk(usb, var) {
        error!("{}", e);
        return default;
    }

    match get_reply(usb, buffer) {
        Ok(reply) => if reply == FastbootHeader::Fail {
            return default;
        }
        Err(e) => {
            error!("{}", e);
            return default;
        }
    }

    u32_from_bytes(buffer)
}

pub fn get_reply(stuff: &mut UsbInterfaces, reply: &mut Vec<u8>) -> Result<FastbootHeader, Box<dyn Error>> {
    let len = input_bulk(stuff, reply)?;
    if len < 4 { return Ok(FastbootHeader::NoHeader); }

    let prefix: FastbootHeader = reply[0..4].into();
    match prefix {
        FastbootHeader::Okay | FastbootHeader::Fail if len == 4 => {
            reply.clear();
        },
        FastbootHeader::Okay | FastbootHeader::Fail if len > 4 => {
            // strip the prefix and copy the rest
            reply.drain(0..4);
        }
        // xperia 10 mark 3 XQ-BT41 send 13 bytes where last byte is null termination
        FastbootHeader::Data if len == 12 || len == 13 => {
            reply.truncate(12);
            reply.drain(0..4);
        },
        _ => panic!("Invalid fastboot message format: {:?}", reply)
    };

    Ok(prefix)
}

pub fn output_bulk(
    stuff: &mut UsbInterfaces,
    data: &[u8],
) -> std::io::Result<usize> {
    if let Err(e) = stuff.writer.write_all(data) {
        return Err(e);
    }

    match stuff.writer.flush_end() {
        Ok(_) => Ok(data.len()),
        Err(e) => Err(e)
    }
}

pub fn input_bulk(
    stuff: &mut UsbInterfaces,
    vec: &mut Vec<u8>,
) -> std::io::Result<usize> {
    vec.clear();
    let mut short_reader = stuff.reader.until_short_packet();
    let r_len = short_reader.read_to_end(vec);
    if let Ok(len) = r_len && let Err(e) =short_reader.consume_end() {
        println!("{}", e);
    }
    r_len
}

pub fn u32_from_bytes(slice: &[u8]) -> u32 {
    let str = str::from_utf8(slice).expect(&format!("Failed to parse {:?} as str", slice));
    str.parse::<u32>().expect(&format!("Failed to parse {:?} as u32", str))
}
