use crate::sins::{process_sins};
use crate::types::{FastbootDevice, FastbootHeader, ProgressBar, Slot};
use crate::utils::{is_sin_file};
use nusb::MaybeFuture;
use nusb::{Device, Interface};
use std::fs;
use std::path::PathBuf;
use std::process::exit;
use anyhow::{ensure, Context};
use log::{debug, info};
use crate::ta::{TrimArea};

pub mod tests;
pub mod types;
pub mod utils;
pub mod sins;
pub mod xml_parser;
pub mod ta;

const VID: u16 = 0x0FCE;
const PID: u16 = 0xB00B;

fn main() {
    env_logger::builder()
        .format_timestamp(None)
        // .format(|buf, record| writeln!(buf, "{}", record.args()))
        .init();

    let args: Vec<String> = std::env::args().collect();

    if args.len() < 2 {
        eprintln!("Missing directory argument");
        exit(1);
    }

    std::env::set_current_dir(args[1].as_str()).unwrap();

    let mut usb = get_flash_mode_rs(VID, PID);

    /* fastboot variables */
    let max_download_size = usb.getvar_u32("max-download-size").unwrap();
    let product = usb.getvar_string("product").unwrap();

    // basic checks that these files are for the correct device
    if !std::env::current_dir()
        .unwrap()
        .file_name()
        .unwrap()
        .to_string_lossy()
        .contains(product.as_str()) {
        println!("Specified folder is probably not for this device!");
        exit(-1);
    }

    let _version = usb.getvar_string("version").unwrap();
    let _version_bootloader = usb.getvar_string("version-bootloader").unwrap();
    let _serial_number = usb.getvar_string("serialno").unwrap();
    let _is_secure = usb.getvar_string("secure").unwrap() == "yes";
    let _sector_size = usb.getvar_u32("Sector-size").unwrap();
    let _loader_version = usb.getvar_string("Loader-version").unwrap();
    let _phone_id = usb.getvar_string("Phone-id").unwrap();
    let _device_id = usb.getvar_string("Device-id").unwrap();
    let _platform_id = usb.getvar_string("Platform-id").unwrap();
    let _rooting_status = usb.getvar_string("Rooting-status").unwrap();
    let _ufs_info = usb.getvar_string("Ufs-info").unwrap();
    let _emmc_info = usb.getvar_string("Emmc-info").unwrap();
    let _default_security = usb.getvar_string("Default-security").unwrap() == "ON";
    let _keystore_counter = usb.getvar_string("Keystore-counter").unwrap();
    let _security_state = usb.getvar_string("Security-state").unwrap();
    let _s1_root = usb.getvar_string("S1-root").unwrap();
    let _sake_root = usb.getvar_string("Sake-root").unwrap();

    usb.get_data("Get-root-key-hash").unwrap();
    let root_key_hash = usb.reply.iter().map(|b| format!("{:02X}", b)).collect::<String>();

    let _slot_count = usb.getvar_u32("slot-count").unwrap_or(1);
    let current_slot: Slot = usb.getvar_string("current-slot").unwrap().into();
    let other_slot = current_slot.other();
    let battery = usb.getvar_u32("Battery").unwrap();

    if battery < 15 {
      println!("Battery level is too low. Charge your device before flashing!");
      exit(1);
    }

    /* parse relevant files first */
    let tas = ta::parse_all_files().unwrap();

    let partition = xml_parser::partition_delivery().unwrap();

    let sins = fs::read_dir("./")
        .unwrap()
        .filter_map(|entry| is_sin_file(entry))
        .collect::<Vec<_>>();
    info!("Found {} .sin files", sins.len());

    let boot = xml_parser::boot_delivery(&root_key_hash).unwrap();
    debug!("{:#?}", boot);

    let boot_ta = TrimArea::try_from_file(PathBuf::from("./boot").join(&boot.boot_config)).unwrap();
    debug!("{:#?}", boot_ta);


    /* begin flashing */
    usb.enter_flash_mode().unwrap();

    println!("{:─<97}", "Processing ./partition files ");

    for path in partition {
        process_sins(&mut usb, path.as_path(), "Repartition", other_slot).unwrap();
    }

    println!("{:─<97}", "Processing .sin files ");

    for path in sins {
        if path.file_name().unwrap().to_string_lossy().contains("persist") {
            info!("Skipping {}", path.to_string_lossy());
            continue;
        }

        process_sins(&mut usb, path.as_path(), "flash", other_slot).with_context(|| format!("Failed to flash {}", path.display())).unwrap()
    };
 
    println!("{:─<97}", "Processing .ta files ");

    for ta in tas {
        let progress = ProgressBar::new(1, format!("Processing {}", ta.name));
        ta.flash(&mut usb).unwrap();
        progress.set_position(1);
        progress.okay();
    }

    println!("{:─<97}", "Processing boot delivery ");

    boot_ta.flash(&mut usb).unwrap();

    for img in &boot.boot_images {
        let path = PathBuf::from("./boot").join(img);
        process_sins(&mut usb, path.as_path(), "flash", other_slot).unwrap();
    }

    // print_firmware_history(&mut usb).unwrap();

    usb.set_active_slot(other_slot).unwrap();
    usb.exit_flash_mode().unwrap();

    usb.command_expect("Sync", FastbootHeader::Okay).unwrap();

    usb.reboot_system().unwrap();
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

fn print_firmware_history(usb: &mut FastbootDevice) -> anyhow::Result<()> {
     usb.command_expect("Read-TA:2:2475", FastbootHeader::Data).context("Failed to read firmware history")?;

    let len = usb.reply.as_hexadecimal()?;
    usb.read_reply().expect("Failed to read reply");

    ensure!(usb.reply.len() == len as usize);

    println!("Firmware History ───────────────────────────────────────────────────────────────────────────────────\n{}", usb.reply);

    ensure!(usb.read_reply()? == FastbootHeader::Okay);

    Ok(())
}