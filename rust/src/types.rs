use nusb::io::{EndpointRead, EndpointWrite};
use nusb::transfer::{Bulk, In, Out};
use nusb::{Device, Interface};
use std::ffi::{c_char, c_int, c_uchar};
use std::time::Duration;

const IN: u8 = 0x81;
const OUT: u8 = 0x01;

#[repr(C)]
pub struct CVec {
    pub ptr: *mut u8,
    pub len: usize,
    pub capacity: usize,
}

#[repr(C)]
pub struct UsbHandle {
    #[deprecated]
    pub file_desc: c_int,
    #[deprecated] // TODO: remove, useless
    ep_out: c_uchar,
    #[deprecated]
    ep_in: c_uchar,
    pub _usb: UsbInterfaces,
}

impl UsbHandle {
    pub fn new(fd: c_int, usb: UsbInterfaces) -> Self {
        Self {
            file_desc: fd,
            ep_in: IN,
            ep_out: OUT,
            _usb: usb
        }
    }
}

#[repr(C)]
pub struct UsbInterfaces {
    pub device: Device,
    pub interface: Interface,
    pub reader: EndpointRead<Bulk>,
    pub writer: EndpointWrite<Bulk>,
}

const BUFFER_SIZE: usize = 1024;
const NUM_TRANSFERS: usize = 4;
const TEN_SEC: Duration = Duration::from_secs(10);

impl UsbInterfaces {
    pub fn new(device: Device, interface: Interface) -> Self {
        let reader = interface.endpoint::<Bulk, In>(IN).unwrap().reader(BUFFER_SIZE).with_num_transfers(NUM_TRANSFERS).with_read_timeout(TEN_SEC);
        let writer = interface.endpoint::<Bulk, Out>(OUT).unwrap().writer(BUFFER_SIZE).with_num_transfers(NUM_TRANSFERS).with_write_timeout(TEN_SEC);
        Self {device, interface, reader, writer}
    }
}

#[repr(C)]
#[derive(PartialEq, Eq, Debug)]
pub enum FastbootHeader {
    Error = 0,
    Okay,
    Data,
    Fail,
    NoHeader,
}

impl From<&[u8]> for FastbootHeader {
    fn from(value: &[u8]) -> Self {
        match value {
            b"OKAY" => FastbootHeader::Okay,
            b"DATA" => FastbootHeader::Data,
            b"FAIL" => FastbootHeader::Fail,
            _ => FastbootHeader::NoHeader,
        }
    }
}
