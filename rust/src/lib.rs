mod tests;

use nusb::transfer::{Bulk, Direction, In, Out};
use nusb::{Device, Interface, MaybeFuture};
use std::ffi::{c_char, c_int, c_uchar, c_ushort, CStr};
use std::fs::File;
use std::io::{BufRead, Read, Write};

use nusb::io::{EndpointRead, EndpointWrite};
use std::os::fd::RawFd;
use std::path::Path;
use std::slice;
use std::time::Duration;
use arrayvec::ArrayVec;
use log::error;

const IN: u8 = 0x81;
const OUT: u8 = 0x01;
const TWO_MIN: Duration = Duration::from_secs(10);

#[repr(C)]
pub struct UsbHandle {
    fname: [c_char; 64],
    file_desc: c_int,
    ep_in: c_uchar,
    ep_out: c_uchar,
    _stuff: UsbInterfaces
}

#[repr(C)]
pub struct UsbInterfaces {
    device: Device,
    interface: Interface,
    reader: EndpointRead<Bulk>,
    writer: EndpointWrite<Bulk>
}

// C globals and functions
unsafe extern "C" {
    pub fn display_buffer_hex_ascii(message: *const c_char, buffer: *const c_char, size: usize);
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

// #[unsafe(no_mangle)]
// pub extern "C" fn get_flash_mode(vid: c_ushort, pid: c_ushort) -> *mut UsbHandle {
//     let usb = get_flash_mode_rs(vid, pid);
//
//     let raw_fd = unsafe { *(&usb.device as *const Device as *const RawFd) };
//
//     // technically a memory leak, but we only call this function once
//     let handle = Box::new(UsbHandle {
//         fname: [0; 64],
//         file_desc: raw_fd,
//         ep_in: IN,
//         ep_out: OUT,
//         _stuff: usb
//     });
//
//     Box::into_raw(handle)
// }

pub fn get_flash_mode_rs(vid: u16, pid: u16) -> UsbInterfaces {
    let di = nusb::list_devices()
        .wait()
        .unwrap()
        .find(|d| d.vendor_id() == vid && d.product_id() == pid)
        .expect("Failed to find device at /dev/bus/usb! Device should be connected via flash mode (green *)");

    let device: Device = di.open().wait().expect(format!("Failed to open device (VID: {:?}, PID: {:?})", vid, pid).as_str());
    let interface: Interface = device.claim_interface(0).wait().unwrap();

    let reader = interface.endpoint::<Bulk, In>(IN).unwrap().reader(1024).with_num_transfers(2).with_read_timeout(TWO_MIN);
    let writer = interface.endpoint::<Bulk, Out>(OUT).unwrap().writer(1024).with_num_transfers(2).with_write_timeout(TWO_MIN);

    UsbInterfaces {device, interface, reader, writer}
}

#[unsafe(no_mangle)]
pub extern "C" fn transfer_bulk_ffi(unsafe_handle: *mut UsbHandle, ep: i32, chars: *mut u8, size: usize, _timeout: i32, exact: i32) -> u64 {
    let handle = unsafe { &mut *unsafe_handle };
    let direction = match ep {
        0 => Direction::In,
        1 => Direction::Out,
        _ => return 0
    };
    let slice = unsafe { slice::from_raw_parts_mut(chars, size) };

    match transfer_bulk_rs(&mut handle._stuff, direction, slice, exact != 0) {
        Ok(_) => size as u64,
        Err(_) => 0
    }
}

pub fn transfer_bulk_rs(
    stuff: &mut UsbInterfaces,
    direction: Direction,
    buffer: &mut [u8],
    exact: bool
) -> std::io::Result<usize> {
    if exact {
        match match direction {
            Direction::In => stuff.reader.read_exact(buffer),
            Direction::Out => {
                stuff.writer.write_all(buffer).expect("Failed to write to device!");
                stuff.writer.flush_end()
            }
        } {
            Ok(_) => Ok(buffer.len()),
            Err(e) => Err(e)
        }
    } else {
        if direction == Direction::In {
            return stuff.reader.read(buffer);
        }
        // TODO: not writing all doesnt really make sense
        todo!();
        Err(std::io::Error::new(std::io::ErrorKind::TimedOut, "timeout"))
    }
}

const BUFF_MAX: usize = 0x100_0000;

// TODO: reply can just be an array
pub fn get_reply_rs(stuff: &mut UsbInterfaces, reply: &mut ArrayVec<u8, 4096>, exact: bool) -> bool {
    reply.clear();
    let mut buffer = [0u8; 4096];

    let Ok(ret_len) = transfer_bulk_rs(stuff, Direction::In, buffer.as_mut(), exact) else {
        return false;
    };

    if ret_len > BUFF_MAX {
        println!("Bug!!! ret_len: {:#x} > BUFF_MAX: {:#x}", ret_len, BUFF_MAX);
        return false;
    }

    let prefix = &buffer[..4];

    let b = match prefix {
        b"OKAY" | b"FAIL" if ret_len == 4 => {
            reply.try_extend_from_slice(&buffer[..4]).is_ok()
        },
        b"OKAY" if ret_len > 4 => {
            // strip the "OKAY" prefix and copy the rest
            reply.try_extend_from_slice(&buffer[4..ret_len]).is_ok()
        }
        b"FAIL" if ret_len > 4 => {
            reply.try_extend_from_slice(&buffer[..ret_len]).is_ok()
        },
        // xperia 10 mark 3 XQ-BT41 send 13 bytes where last byte is null termination, fixing it to 12
        b"DATA" if ret_len == 12 || ret_len == 13 => {
            reply.try_extend_from_slice(&buffer[..12]).is_ok()
        },
        _ => unsafe {
            if prefix == b"DATA" {
                error!(" - Erroneous DATA reply!");
                display_buffer_hex_ascii(c"Replied with ".as_ptr(), buffer.as_ptr() as *const c_char, ret_len);
            }
            false
        }
    };

    // add the null terminator since the C functions expect it
    if let Err(e) = reply.try_push(0) {
        return false;
    }
    b
}