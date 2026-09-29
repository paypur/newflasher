use crate::sins::{process_sins};
use crate::types::{FastbootDevice, FastbootHeader, ProgressBar, Slot};
use crate::utils::{is_sin_file, is_ta_file, noerase_in_updatexml};
use nusb::MaybeFuture;
use nusb::{Device, Interface};
use std::fs;
use std::path::PathBuf;
use std::process::exit;
use anyhow::{ensure, Context};
use log::{debug, info};
use crate::ta::{TrimArea};
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
    env_logger::builder()
        .format_timestamp(None)
        .init();

    let args: Vec<String> = std::env::args().collect();

    if args.len() < 2 {
        eprintln!("Missing directory argument");
        exit(1);
    }

    std::env::set_current_dir(args[1].as_str()).unwrap();

    let mut usb = get_flash_mode_rs(VID, PID);

    /* fastboot variables */
    let max_download_size = usb.getvar_u32("getvar:max-download-size", 0);
    let product = usb.getvar_string("getvar:product").unwrap();

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

    if battery < 15 {
      println!("Battery level is too low. Charge your device before flashing!");
      exit(1);
    }

    /* parse relevant files first */
    let tas = fs::read_dir("./").unwrap()
        .filter_map(|entry| is_ta_file(entry))
        .filter(|path| !noerase_in_updatexml(path.file_name().unwrap()))
        .map(|path| TrimArea::try_from_file(path))
        .collect::<Result<Vec<_>, _>>()
        .expect("All .ta files could not be successfully parsed!");
    if tas.is_empty() {
        eprintln!("No .ta files found!");
        exit(1);
    } else {
        info!("Found {} .ta files", tas.len());
    }

    let partition = xml_parser::partition_delivery().unwrap();
    info!("Found {} partition files", partition.len());

    let sins = fs::read_dir("./")
        .unwrap()
        .filter_map(|entry| is_sin_file(entry))
        .collect::<Vec<_>>();
    info!("Found {} .sin files", sins.len());

    let boot = boot_delivery(&root_key_hash).unwrap();
    info!("Found {} configuration in boot.xml", boot.config._name);
    debug!("{:#?}", boot);

    let boot_ta = TrimArea::try_from_file(PathBuf::from("./boot").join(&boot.config.boot_config)).unwrap();
    debug!("{:#?}", boot_ta);


    /* begin flashing */
    enter_flash_mode(&mut usb);

    println!("{:─<97}", "Processing ./partition files ");

    partition.iter().for_each(|path| process_sins(&mut usb, path.as_path(), "Repartition", current_slot.other()).unwrap());

    println!("{:─<97}", "Processing .sin files ");

    for path in sins {
        process_sins(&mut usb, path.as_path(), "flash", current_slot.other()).with_context(|| format!("Failed to flash {}", path.display())).unwrap()
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

    for img in &boot.config.boot_images {
        let path = PathBuf::from("./boot").join(img);
        process_sins(&mut usb, path.as_path(), "flash", current_slot).unwrap();
    }

    panic!("Skipped syncing");

    print_firmware_history(&mut usb).unwrap();

    set_active_slot(&mut usb, current_slot.other());
    exit_flash_mode(&mut usb);

    usb.command_expect("Sync", FastbootHeader::Okay).unwrap();

    // reboot to system
    usb.command_expect("continue", FastbootHeader::Okay).unwrap();
}

fn enter_flash_mode(usb: &mut FastbootDevice) {
    usb.download(&[1u8]).unwrap();
    usb.command_expect("Write-TA:2:10100", FastbootHeader::Okay).unwrap();
    info!("Entered flashmode");
}

fn exit_flash_mode(usb: &mut FastbootDevice) {
    usb.download(&[0u8]).unwrap();
    usb.command_expect("Write-TA:2:10100", FastbootHeader::Okay).unwrap();
    info!("Exited flashmode");
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

fn print_firmware_history(usb: &mut FastbootDevice) -> anyhow::Result<()> {
     usb.command_expect("Read-TA:2:2475", FastbootHeader::Data).context("Failed to read firmware history")?;

    let len = usb.reply.as_hexadecimal()?;
    usb.read_reply().expect("Failed to read reply");

    ensure!(usb.reply.len() == len as usize);

    println!("Firmware History ───────────────────────────────────────────────────────────────────────────────────\n{}", usb.reply);

    ensure!(usb.read_reply()? == FastbootHeader::Okay);

    Ok(())
}