mod tests;
mod types;
mod utils;
mod sins;
mod xml_parser;

use nusb::{Device, Interface, MaybeFuture};
use std::ffi::{c_char, c_ushort, CStr};

use crate::types::*;
use log::error;
use std::cmp::min;
use std::{io, ptr, slice};

// C globals and functions
unsafe extern "C" {
    pub fn display_buffer_hex_ascii(message: *const c_char, buffer: *const c_char, size: usize);
}

pub fn new_cvec(capacity: usize) -> ByteVec {
    into_cvec(Vec::with_capacity(capacity))
}

fn into_cvec(vec: Vec<u8>) -> ByteVec {
    ByteVec::from(vec)
}

fn from_cvec(cvec: &ByteVec) -> Vec<u8> {
    vec![]
}

fn return_vec(cvec: &mut ByteVec, vec: Vec<u8>) {}

#[unsafe(no_mangle)]
pub extern "C" fn get_flash_mode(vid: c_ushort, pid: c_ushort) -> *mut FastbootDevice {
    let dev = get_flash_mode_rs(vid, pid);
    // technically a memory leak, but we only call this function once
    let handle = Box::new(dev);
    Box::into_raw(handle)
}

pub fn get_flash_mode_rs(vid: u16, pid: u16) -> FastbootDevice {
    let di = nusb::list_devices()
        .wait()
        .unwrap()
        .find(|d| d.vendor_id() == vid && d.product_id() == pid)
        .expect("Failed to find device at /dev/bus/usb! Device should be connected via flash mode (green *)");

    let device: Device = di.open().wait().unwrap_or_else(|e| panic!("Failed to open device (VID: {vid}, PID: {pid})\n{e}"));
    let interface: Interface = device.claim_interface(0).wait().unwrap();

    FastbootDevice::new(device, interface)
}

#[unsafe(no_mangle)]
pub extern "C" fn transfer_bulk_ffi(unsafe_handle: *mut FastbootDevice, ep: i32, chars: *mut u8, len: usize, capacity: usize) -> usize {
    let handle = unsafe { &mut *unsafe_handle };

    match ep {
        0 => {
            let mut vec = unsafe { Vec::from_raw_parts(chars, len, capacity) };
            let res = handle.transfer_in();
            let _ = vec.into_raw_parts(); // make sure rust doesnt drop this
            res
        },
        1 => handle.transfer_out(unsafe { slice::from_raw_parts(chars, len) }),
        _ => panic!("Invalid endpoint direction: {:?}", ep)
    }.unwrap_or(0)
}

#[unsafe(no_mangle)]
pub extern "C" fn get_reply_ffi(unsafe_handle: *mut FastbootDevice, cvec: &mut ByteVec, exact: i32) -> FastbootHeader {
    let handle = unsafe { &mut *unsafe_handle };
    let mut vec = from_cvec(cvec);
    let res = handle.read_reply();
    return_vec(cvec, vec);
    res.unwrap_or_else(|_| FastbootHeader::Error)
}

#[unsafe(no_mangle)]
pub extern "C" fn fastboot_cmd_ffi(usb_handle: *mut FastbootDevice, cvec: &mut ByteVec, cmd: *const c_char, str: *mut u8, mut len: usize) -> bool {
    let usb = unsafe { &mut *usb_handle };
    let mut buffer = from_cvec(cvec);
    let cstr = unsafe { CStr::from_ptr(cmd) };

    if let Err(e) = usb.command(cstr.to_bytes()) {
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
pub extern "C" fn fastboot_download_ffi(usb_handle: *mut FastbootDevice, cvec: &mut ByteVec, data: *const u8, len: usize) -> bool {
    let usb = unsafe { &mut *usb_handle };
    let mut buffer = from_cvec(cvec);
    let slice = unsafe { slice::from_raw_parts(data, len) };

    if let Err(e) = usb.download(slice) {
        error!("{}", e);
        return false;
    }

    true
}

#[unsafe(no_mangle)]
pub extern "C" fn getvar_u32_ffi(usb_handle: *mut FastbootDevice, cvec: &mut ByteVec, cmd: *const c_char, fallback: u32) -> u32 {
    let usb = unsafe { &mut *usb_handle };
    let mut buffer = from_cvec(cvec);

    let u = usb.getvar_u32(unsafe { CStr::from_ptr(cmd) }.to_bytes(), fallback);
    return_vec(cvec, buffer);
    u
}

pub fn u32_from_bytes(slice: &[u8]) -> u32 {
    let str = str::from_utf8(slice).expect(&format!("Failed to parse {:?} as str", slice));
    str.parse::<u32>().expect(&format!("Failed to parse {:?} as u32", str))
}