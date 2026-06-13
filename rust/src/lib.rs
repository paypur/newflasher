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
}

#[unsafe(no_mangle)]
pub extern "C" fn get_flash_mode(vid: c_ushort, pid: c_ushort) -> *mut FastbootDeviceFFI {
    let dev = get_flash_mode_rs(vid, pid);
    let ffi = FastbootDeviceFFI::from(dev);
    let handle = Box::new(ffi);
    // technically a memory leak, but we only call this function once
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
pub extern "C" fn transfer_bulk_ffi(device_ptr: *mut FastbootDeviceFFI, ep: i32, chars: *mut u8, len: usize) -> usize {
    let mut usb: FastbootDevice = device_ptr.into();

    let res = match ep {
        0 => usb.read(),
        1 => usb.write(unsafe { slice::from_raw_parts(chars, len) }),
        _ => panic!("Invalid endpoint direction: {:?}", ep)
    }.unwrap_or(0);

    unsafe { ptr::write(device_ptr, FastbootDeviceFFI::from(usb)) };
    res
}

#[unsafe(no_mangle)]
pub extern "C" fn get_reply_ffi(device_ptr: *mut FastbootDeviceFFI) -> FastbootHeader {
    let mut usb: FastbootDevice = device_ptr.into();
    let res = usb.read_reply().unwrap_or_else(|_| FastbootHeader::Error);

    unsafe { ptr::write(device_ptr, FastbootDeviceFFI::from(usb)) };
    res
}

#[unsafe(no_mangle)]
pub extern "C" fn fastboot_cmd_ffi(device_ptr: *mut FastbootDeviceFFI, cmd: *const c_char, str: *mut u8, len: usize) -> bool {
    let mut usb: FastbootDevice = device_ptr.into();
    let cstr = unsafe { CStr::from_ptr(cmd) }.to_string_lossy();

    if let Err(e) = usb.command(cstr.as_ref()) {
        error!("{}", e);
        unsafe { ptr::write(device_ptr, FastbootDeviceFFI::from(usb)) };
        return false;
    }

    if str as *const u8 != ptr::null() && len != 0 {
        let min_len = len.min(usb.reply.len());

        let string = unsafe { slice::from_raw_parts_mut(str, min_len) };
        string[..min_len].clone_from_slice(&usb.reply[..min_len]);

        // write the null terminator for C strings
        string[min(min_len, string.len() - 1)] = 0;
    }

    unsafe { ptr::write(device_ptr, FastbootDeviceFFI::from(usb)) };
    true
}

#[unsafe(no_mangle)]
pub extern "C" fn fastboot_download_ffi(device_ptr: *mut FastbootDeviceFFI, data: *const u8, len: usize) -> bool {
    let mut usb: FastbootDevice = device_ptr.into();
    let slice = unsafe { slice::from_raw_parts(data, len) };

    if let Err(e) = usb.download(slice) {
        error!("{}", e);
        unsafe { ptr::write(device_ptr, FastbootDeviceFFI::from(usb)) };
        return false;
    }

    unsafe { ptr::write(device_ptr, FastbootDeviceFFI::from(usb)) };
    true
}

#[unsafe(no_mangle)]
pub extern "C" fn getvar_u32_ffi(device_ptr: *mut FastbootDeviceFFI, cmd: *const c_char, fallback: u32) -> u32 {
    let mut usb: FastbootDevice = device_ptr.into();
    let res = usb.getvar_u32(unsafe { CStr::from_ptr(cmd) }.to_string_lossy().as_ref(), fallback);

    unsafe { ptr::write(device_ptr, FastbootDeviceFFI::from(usb)) };
    res
}