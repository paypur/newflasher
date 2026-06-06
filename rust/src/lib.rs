mod tests;

use nusb::transfer::{Bulk, Direction, In, Out};
use nusb::{Device, Interface, MaybeFuture};
use std::ffi::{c_char, c_int, c_uchar, c_ulong, c_ushort};
use std::io::{Read, Write};

use std::os::fd::RawFd;
use std::slice;
use std::time::Duration;
use nusb::io::{EndpointRead, EndpointWrite};

const IN: u8 = 0x81;
const OUT: u8 = 0x01;
const TWO_MIN: Duration = Duration::from_mins(2);

#[repr(C)]
pub struct UsbHandle {
    fname: [c_char; 64],
    file_desc: c_int,
    ep_in: c_uchar,
    ep_out: c_uchar,
    _context: Stuff
}

#[repr(C)]
struct Stuff {
    device: Device,
    interface: Interface,
    reader: EndpointRead<Bulk>,
    writer: EndpointWrite<Bulk>
}

// C globals and functions
unsafe extern "C" {
}

#[unsafe(no_mangle)]
pub extern "C" fn get_flash_mode(vid: c_ushort, pid: c_ushort) -> *mut UsbHandle {
    let di = nusb::list_devices()
        .wait()
        .unwrap()
        .find(|d| d.vendor_id() == vid && d.product_id() == pid)
        .expect("Failed to find device at /dev/bus/usb! Device should be connected via flash mode (green *)");

    let device: Device = di.open().wait().expect(format!("Failed to open device (VID: {:?}, PID: {:?})", vid, pid).as_str());
    let interface: Interface = device.claim_interface(0).wait().unwrap();

    let raw_fd = unsafe { *(&device as *const Device as *const RawFd) };

    let reader = interface.endpoint::<Bulk, In>(IN).unwrap().reader(1024).with_num_transfers(2).with_read_timeout(TWO_MIN);
    let writer = interface.endpoint::<Bulk, Out>(OUT).unwrap().writer(1024).with_num_transfers(2).with_write_timeout(TWO_MIN);

    // technically a memory leak, but we only call this once
    let handle = Box::new(UsbHandle { fname: [0; 64], file_desc: raw_fd, ep_in: IN, ep_out: OUT,
        _context: Stuff {device, interface, reader, writer}
    });

    Box::into_raw(handle)
}


#[unsafe(no_mangle)]
pub extern "C" fn transfer_bulk_ffi(unsafe_handle: *mut UsbHandle, ep: c_int, chars: *mut c_char, size: c_ulong, _timeout: c_int, _exact: c_int) -> u64 {
    let handle = unsafe { &mut *unsafe_handle };
    let direction = match ep {
        0 => Direction::In,
        1 => Direction::Out,
        _ => return 0
    };
    let slice = unsafe { slice::from_raw_parts_mut(chars as *mut u8, size as usize) };

    match transfer_bulk(&mut handle._context, direction, slice, Duration::default()) {
        Ok(_) => size,
        Err(_) => 0
    }
}

fn transfer_bulk(
    stuff: &mut Stuff,
    direction: Direction,
    buffer: &mut [u8],
    _timeout: Duration,
) -> std::io::Result<()> {
    if direction == Direction::In {
         stuff.reader.read_exact(buffer)
    } else {
        stuff.writer.write_all(buffer)
    }
}