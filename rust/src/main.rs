use crate::sins::{process_sins};
use crate::types::{FastbootDevice, FastbootHeader, Slot};
use crate::utils::{is_sin_file, is_ta_file};
use nusb::MaybeFuture;
use nusb::{Device, Interface};
use std::fs;
use std::path::PathBuf;
use crate::ta::{flash_trim_area, process_trim_area};
use crate::xml_parser::boot_delivery;

pub mod tests;
pub mod types;
pub mod utils;
pub mod sins;
pub mod xml_parser;
pub mod ta;

const VID: u16 = 0x0FCE;
const PID: u16 = 0xB00B;

fn main() {
    let mut usb = get_flash_mode_rs(VID, PID);

    /* fastboot variables */
    let max_download_size = usb.getvar_u32("getvar:max-download-size", 0);
    let product = usb.getvar_string("getvar:product").unwrap();
    let version = usb.getvar_string("getvar:version").unwrap();
    let version_bootloader = usb.getvar_string("getvar:version-bootloader").unwrap();
    let serial_number = usb.getvar_string("getvar:serialno").unwrap();
    let is_secure = usb.getvar_string("getvar:secure").unwrap() == "yes";
    let sector_size = usb.getvar_u32("getvar:Sector-size", 0);
    let loader_version = usb.getvar_string("getvar:Loader-version").unwrap();
    let phone_id = usb.getvar_string("getvar:Phone-id").unwrap();
    let device_id = usb.getvar_string("getvar:Device-id").unwrap();
    let platform_id = usb.getvar_string("getvar:Platform-id").unwrap();
    let rooting_status = usb.getvar_string("getvar:Rooting-status").unwrap();
    let ufs_info = usb.getvar_string("getvar:Ufs-info").unwrap();
    let emmc_info = usb.getvar_string("getvar:Emmc-info").unwrap();
    let default_security = usb.getvar_string("getvar:Default-security").unwrap() == "ON";
    let keystore_counter = usb.getvar_string("getvar:Keystore-counter").unwrap();
    let security_state = usb.getvar_string("getvar:Security-state").unwrap();
    let s1_root = usb.getvar_string("getvar:S1-root").unwrap();
    let sake_root = usb.getvar_string("getvar:Sake-root").unwrap();

    usb.get_data("Get-root-key-hash").unwrap();
    let root_key_hash = usb.reply.iter().map(|b| format!("{:02X}", b)).collect::<String>();

    let slot_count = usb.getvar_u32("getvar:slot-count", 1);
    let current_slot: Slot = usb.getvar_string("getvar:current-slot").unwrap().as_str().into();
    let battery = usb.getvar_u32("getvar:Battery", 0);

    // if battery < 15 {
    //     println!("Battery level is too low, charge your device before flashing!");
    //     std::process::exit(1);
    // }

    // TODO: remove this
    std::env::set_current_dir("../../H8314_O2_Pay_monthly_UK_52.1.A.3.49-R6C/").unwrap();

    enter_flash_mode(&mut usb);

    println!("Processing ./partition files ───────────────────────────────────────────────────────────────────────\n");

    // TODO: probably should use xml_parser::partition_delivery() instead of this
    fs::read_dir("./partition/").unwrap()
        .filter_map(|entry| is_sin_file(entry))
        .for_each(|path| process_sins(&mut usb, path, "Repartition", current_slot).unwrap());

    println!("Processing .sin files ──────────────────────────────────────────────────────────────────────────────\n");

    fs::read_dir("./").unwrap()
        .filter_map(|entry| is_sin_file(entry))
        .for_each(|path| process_sins(&mut usb, path, "flash", current_slot).unwrap());

    println!("Processing .ta files ───────────────────────────────────────────────────────────────────────────────\n");

    fs::read_dir("./").unwrap()
        .filter_map(|entry| is_ta_file(entry))
        .map(|path| process_trim_area(path).unwrap()) // can't recover from this error
        .for_each(|path| flash_trim_area(&mut usb, path).unwrap());

    println!("Processing boot delivery ───────────────────────────────────────────────────────────────────────────\n");

    match boot_delivery(PathBuf::from("./boot/boot_delivery.xml")) {
        Ok(bd) => {
            println!("{:#?}", bd.configurations);

            // TODO: why???
            let mut modified = platform_id.clone();
            modified.replace_range(..2, "00");

            // TODO: check bd.space_id with version_bootloader
            bd.configurations.iter()
                .filter(|bc| bc.platform_id == modified && root_key_hash.contains(bc.plf_root_hash.as_str()))
                .for_each(|bc| {
                    let opt = process_trim_area(PathBuf::from(format!("./boot/{}", bc.boot_config))).unwrap();
                    if let ta = opt {
                        flash_trim_area(&mut usb, ta).unwrap();

                        for img in &bc.boot_images {
                            let path = PathBuf::from(format!("./boot/{}", img));
                            if img.contains("bootloader") {
                                process_sins(&mut usb, path, "flash", current_slot).unwrap();
                            } else {
                                println!("Skipping non bootloader {} file", path.display());
                            }
                        }
                    }
                });
        },
        Err(e) => eprintln!("{e}"),
    }

    exit_flash_mode(&mut usb);

    // TODO: shouldn't this switch slots??
    // set_active_slot(&mut usb, current_slot.other());

    usb.command_expect("Sync", FastbootHeader::Okay).unwrap();

    print_firmware_history(&mut usb);
}

fn enter_flash_mode(usb: &mut FastbootDevice) {
    usb.download(&[1u8]).unwrap();
    usb.command_expect("Write-TA:2:10100", FastbootHeader::Okay).unwrap();
}

fn exit_flash_mode(usb: &mut FastbootDevice) {
    usb.download(&[0u8]).unwrap();
    usb.command_expect("Write-TA:2:10100", FastbootHeader::Okay).unwrap();
}

fn set_active_slot(usb: &mut FastbootDevice, slot: Slot) {
    let s: &str = slot.as_str();
    let cmd = format!("set_active:{s}");
    usb.command_expect(cmd.as_str(), FastbootHeader::Okay).unwrap();
}

fn get_flash_mode_rs(vid: u16, pid: u16) -> FastbootDevice {
    let di = nusb::list_devices()
        .wait()
        .unwrap()
        .find(|d| d.vendor_id() == vid && d.product_id() == pid)
        .expect("Failed to find device at /dev/bus/usb! Device should be connected via flash mode (green *)");

    let device: Device = di.open().wait().unwrap_or_else(|e| panic!("Failed to open device (VID: {vid}, PID: {pid})\n{e}"));
    let interface: Interface = device.claim_interface(0).wait().unwrap();

    FastbootDevice::new(device, interface)
}

fn print_firmware_history(usb: &mut FastbootDevice) {
    usb.command_expect("Read-TA:2:2475", FastbootHeader::Data).inspect_err(|e| eprintln!("{e}"));

    if let Ok(len) = usb.reply.as_hexadecimal() {
        usb.read_reply().expect("Failed to read reply");

        assert_eq!(usb.reply.len(), len as usize);

        println!("Firmware History ───────────────────────────────────────────────────────────────────────────────────\n{}", usb.reply);
    }

    ()
}