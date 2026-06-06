use nusb::{Device, Endpoint, Interface, MaybeFuture};
use std::ffi::{c_char, c_int, c_uchar, c_ushort, c_void};
use nusb::transfer::{Bulk, In, Out};

use std::os::fd::RawFd;

#[repr(C)]
pub struct UsbHandle {
    fname: [c_char; 64],
    file_desc: c_int,
    ep_in: c_uchar,
    ep_out: c_uchar,
    _context: Stuff
}

struct Stuff {
    device: Device,
    interface: Interface
}

// C globals and functions
unsafe extern "C" {
}

#[unsafe(no_mangle)]
pub extern "C" fn get_flash_mode(vid: c_ushort, pid: c_ushort) -> *const c_void {
    let di = nusb::list_devices()
        .wait()
        .unwrap()
        .find(|d| d.vendor_id() == vid && d.product_id() == pid)
        .expect("Device should be connected ");

    let device: Device = di.open().wait().expect(format!("Failed to open device (VID: {:?}, PID: {:?})", vid, pid).as_str());
    let interface: Interface = device.claim_interface(0).wait().unwrap();

    let raw_fd = unsafe { *(&device as *const Device as *const RawFd) };

    // TODO: only for flash mode
    let o = 0x01;
    let i = 0x81;

    let ep_in: Endpoint<Bulk, In> = interface.endpoint::<Bulk, In>(i).unwrap();
    let ep_out: Endpoint<Bulk, Out> = interface.endpoint::<Bulk, Out>(o).unwrap();

    // technically a memory leak, but we only call this once
    let context = Box::new(UsbHandle { fname: [0; 64], file_desc: raw_fd, ep_in: ep_in.endpoint_address(), ep_out: ep_out.endpoint_address(), _context: Stuff {device, interface} });

    Box::into_raw(context) as *mut c_void
}


// #[unsafe(no_mangle)]
// pub extern "C" fn transfer_bulk_ffi(handle: *const libusb_device_handle, ep: c_int, bytes: *const c_char, size: c_ulong, timeout: c_int, exact: c_int) -> u64 {
//     let nn_handle = NonNull::from(handle);
//     DeviceHandle::from_libusb(nn_handle)
// }
//
// fn transfer_bulk<T: UsbContext>(
//     device: &mut Device<T>,
//     direction: Direction,
//     buf: &mut [u8],
//     timeout: Duration,
// ) -> Result<usize, rusb::Error> {
//     let ep_address = match direction {
//         Direction::In => 0x81,
//         Direction::Out => 0x01,
//     };
//
//     let handle = device.open()?;
//
//     if direction == Direction::In {
//         handle.read_bulk(ep_address, buf, timeout)
//     } else {
//         handle.write_bulk(ep_address, buf, timeout)
//     }
// }