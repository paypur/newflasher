mod tests;

use std::cmp::{PartialEq};
use nusb::transfer::{Bulk, Direction, In, Out};
use nusb::{Device, Interface, MaybeFuture};
use std::ffi::{c_char, c_int, c_uchar, c_ushort, CStr};
use std::fs::File;
use std::io::{Read, Write};

use nusb::io::{EndpointRead, EndpointWrite};
use std::os::fd::RawFd;
use std::path::Path;
use std::slice;
use std::time::Duration;
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

#[repr(C)]
pub struct CVec {
    pub ptr: *mut u8,
    pub len: usize,
    pub capacity: usize,
}

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
    let handle = Box::new(UsbHandle {
        fname: [0; 64],
        file_desc: raw_fd,
        ep_in: IN,
        ep_out: OUT,
        _stuff: usb
    });

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

    let reader = interface.endpoint::<Bulk, In>(IN).unwrap().reader(1024).with_num_transfers(2).with_read_timeout(TWO_MIN);
    let writer = interface.endpoint::<Bulk, Out>(OUT).unwrap().writer(1024).with_num_transfers(2).with_write_timeout(TWO_MIN);

    UsbInterfaces {device, interface, reader, writer}
}

#[unsafe(no_mangle)]
pub extern "C" fn transfer_bulk_ffi(unsafe_handle: *mut UsbHandle, ep: i32, chars: *mut u8, len: usize, capacity: usize, _timeout: i32, exact: i32) -> usize {
    let handle = unsafe { &mut *unsafe_handle };

    match ep {
        0 => {
            let mut vec = unsafe { Vec::from_raw_parts(chars, len, capacity) };
            let res = input_bulk(&mut handle._stuff, &mut vec, exact != 0);
            let _ = vec.into_raw_parts(); // make sure rust doesnt drop this
            res
        },
        1 => output_bulk(&mut handle._stuff, unsafe { slice::from_raw_parts(chars, len) }),
        _ => panic!("Invalid endpoint direction: {:?}", ep)
    }.unwrap_or_else(|_| 0)
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
    exact: bool
) -> std::io::Result<usize> {
    vec.clear();
    if exact {
        match stuff.reader.read_exact(vec) {
            Ok(_) => Ok(vec.len()),
            Err(e) => Err(e)
        }
    } else {
        let mut short_reader = stuff.reader.until_short_packet();
        let len = short_reader.read_to_end(vec);
        short_reader.consume_end().unwrap();
        len
    }
}

#[unsafe(no_mangle)]
pub extern "C" fn get_reply_ffi(unsafe_handle: *mut UsbHandle, cvec: &mut CVec, exact: i32) -> FastbootReply {
    let handle = unsafe { &mut *unsafe_handle };
    let mut vec = from_cvec(cvec);
    let res = get_reply(&mut handle._stuff, &mut vec, exact != 0);
    return_vec(cvec, vec);
    res.unwrap_or_else(|_| FastbootReply::Error)
}

#[repr(C)]
#[derive(PartialEq, Eq)]
#[derive(Debug)]
pub enum FastbootReply {
    Error,
    Okay,
    Data,
    Fail,
    NoHeader,
}

impl From<&[u8]> for FastbootReply {
    fn from(value: &[u8]) -> Self {
        match value {
            b"OKAY" => FastbootReply::Okay,
            b"DATA" => FastbootReply::Data,
            b"FAIL" => FastbootReply::Fail,
            _ => FastbootReply::NoHeader
        }
    }
}

pub fn get_reply(stuff: &mut UsbInterfaces, reply: &mut Vec<u8>, exact: bool) -> Result<FastbootReply, std::io::Error> {
    let mut len = input_bulk(stuff, reply, exact)?;

    let prefix: FastbootReply = reply[0..4].into();

    match prefix {
        FastbootReply::Okay | FastbootReply::Fail if len == 4 => {
            reply.clear();
        },
        FastbootReply::Okay | FastbootReply::Fail if len > 4 => {
            // strip the prefix and copy the rest
            reply.drain(0..4);
        }
        // xperia 10 mark 3 XQ-BT41 send 13 bytes where last byte is null termination
        FastbootReply::Data if len == 12 || len == 13 => {
            len = usize::from_str_radix(str::from_utf8(&reply[4..12]).expect("Failed to parse DATA header as str"), 16).expect("Failed to parse DATA header as hexadecimal");

            // second read for actual data
            input_bulk(stuff, reply, exact)?;
            if FastbootReply::from(&reply[..4]) == FastbootReply::Okay { todo!("unimplemented OKAY after DATA") }
            assert_eq!(len, reply.len());

            // if not OKAY in 2nd read, should read a 3rd time to acknowledge
            let mut vec4 = Vec::<u8>::with_capacity(4);
            input_bulk(stuff, &mut vec4, exact)?;
            assert_eq!(Into::<FastbootReply>::into(&vec4[..4]), FastbootReply::Okay)
        },
        _ => panic!("Invalid fastboot message format {:?}", reply)
    };

    Ok(prefix)
}

#[unsafe(no_mangle)]
pub extern "C" fn getvar_ffi(usb_handle: *mut UsbHandle, cvec: &mut CVec, var: *const u8, str: *mut u8, len: usize) {
    let usb = &mut unsafe { &mut *usb_handle }._stuff;
    let mut buffer = from_cvec(cvec);
    getvar(usb, &mut buffer, unsafe { &CStr::from_ptr(var as *const c_char) }, unsafe { slice::from_raw_parts_mut(str, len) });
    return_vec(cvec, buffer);
}

pub fn getvar(usb: &mut UsbInterfaces, buffer: &mut Vec<u8>, var: &CStr, str: &mut [u8]) {
    buffer.clear();

    if let Err(e) = output_bulk(usb, var.to_bytes()) {
        error!("{}", e);
        return;
    }

    match get_reply(usb, buffer, false) {
        Ok(reply) => assert_ne!(reply, FastbootReply::Error),
        Err(e) => {
            error!("{}", e);
            return;
        }
    }

    let len = buffer.len();
    str[..len].clone_from_slice(&buffer.as_slice()[..len]);
}

#[unsafe(no_mangle)]
pub extern "C" fn getvar_u32_ffi(usb_handle: *mut UsbHandle, cvec: &mut CVec, var: *const u8, default: u32) -> u32 {
    let usb = &mut unsafe { &mut *usb_handle }._stuff;
    let mut buffer = from_cvec(cvec);
    let u = getvar_u32(usb, &mut buffer, unsafe { &CStr::from_ptr(var as *const c_char) }, default);
    return_vec(cvec, buffer);
    u
}

pub fn getvar_u32(usb: &mut UsbInterfaces, buffer: &mut Vec<u8>, var: &CStr, default: u32) -> u32 {
    buffer.clear();

    if let Err(e) = output_bulk(usb, var.to_bytes()) {
        error!("{}", e);
        return default;
    }

    match get_reply(usb, buffer, false) {
        Ok(reply) => if reply == FastbootReply::Fail {
            return default;
        }
        Err(e) => {
            error!("{}", e);
            return default;
        }
    }

    u32_from_bytes(buffer)
}

pub fn u32_from_bytes(buffer: &[u8]) -> u32 {
    let str = str::from_utf8(buffer).expect(&format!("Failed to parse {:?} as str", buffer));
    str.parse::<u32>().expect(&format!("Failed to parse {:?} as u32", str))
}