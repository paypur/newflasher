use crate::sins::{process_sins};
use crate::types::{ProgressBar, Slot};
use crate::utils::{is_sin_file};
use std::fs;
use std::io::{stdin, Read};
use std::path::PathBuf;
use std::process::exit;
use anyhow::{Context};
use log::{debug, info};
use crate::fastboot::{FastbootDevice, Header};
use crate::ta::{TrimArea};

pub mod tests;
pub mod types;
pub mod utils;
pub mod sins;
pub mod xml_parser;
pub mod ta;
mod fastboot;

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

    let mut dev = FastbootDevice::new();

    /* fastboot variables */
    let max_download_size = dev.getvar_u32("max-download-size").unwrap();
    let product = dev.getvar_string("product").unwrap();

    // basic checks that these files are for the correct device
    if !std::env::current_dir()
        .unwrap()
        .file_name()
        .unwrap()
        .to_string_lossy()
        .contains(product.as_str()) {
        println!("Specified directory is probably not for this device!");
        exit(-1);
    }

    let _version = dev.getvar_string("version").unwrap();
    let _version_bootloader = dev.getvar_string("version-bootloader").unwrap();
    let _serial_number = dev.getvar_string("serialno").unwrap();
    let _is_secure = dev.getvar_string("secure").unwrap() == "yes";
    let _sector_size = dev.getvar_u32("Sector-size").unwrap();
    let _loader_version = dev.getvar_string("Loader-version").unwrap();
    let _phone_id = dev.getvar_string("Phone-id").unwrap();
    let _device_id = dev.getvar_string("Device-id").unwrap();
    let _platform_id = dev.getvar_string("Platform-id").unwrap();
    let _rooting_status = dev.getvar_string("Rooting-status").unwrap();
    let _ufs_info = dev.getvar_string("Ufs-info").unwrap();
    let _emmc_info = dev.getvar_string("Emmc-info");
    let _default_security = dev.getvar_string("Default-security").unwrap() == "ON";
    let _keystore_counter = dev.getvar_string("Keystore-counter").unwrap();
    let _security_state = dev.getvar_string("Security-state").unwrap();
    let _s1_root = dev.getvar_string("S1-root").unwrap();
    let _sake_root = dev.getvar_string("Sake-root").unwrap();

    let root_key_hash = dev.get_data("Get-root-key-hash").unwrap().to_hexadecimal_string();

    let _slot_count = dev.getvar_u32("slot-count").unwrap_or(1);
    let current_slot: Slot = dev.getvar_string("current-slot").unwrap().into();
    let other_slot = current_slot.other();
    let battery = dev.getvar_u32("Battery").unwrap();

    if battery < 15 {
      println!("Battery level is too low. Charge your device before flashing!");
      exit(1);
    }

    /* parse relevant files first */
    let tas = ta::parse_all().unwrap();

    let partition = xml_parser::partition_delivery().unwrap();

    let sins = sin_files();

    let boot = xml_parser::boot_delivery(&root_key_hash).unwrap();
    debug!("{:#?}", boot);

    let boot_ta = TrimArea::try_from_file(PathBuf::from("./boot").join(&boot.boot_config)).unwrap();
    debug!("{:#?}", boot_ta);

    println!("Active Slot: {current_slot}");
    println!("Ready to flash device, press any key to continue ...");

    loop {
        let mut b = [0; 1];
        if let Ok(l) = stdin().read(&mut b) && l > 0{
            break;
        }
    }

    /* begin flashing */
    dev.enter_flash_mode().unwrap();

    println!("{:─<97}", "Processing ./partition files ");

    for path in partition {
        process_sins(&mut dev, path.as_path(), other_slot).unwrap();
    }

    println!("{:─<97}", "Processing .sin files ");

    for path in sins {
        process_sins(&mut dev, path.as_path(), other_slot).with_context(|| format!("Failed to flash {}", path.display())).unwrap()
    };
 
    println!("{:─<97}", "Processing .ta files ");

    for ta in tas {
        let progress = ProgressBar::new(1, format!("Processing {}", ta.name));
        ta.flash(&mut dev).unwrap();
        progress.set_position(1);
        progress.okay();
    }

    println!("{:─<97}", "Processing boot delivery ");

    boot_ta.flash(&mut dev).unwrap();

    for img in &boot.boot_images {
        let path = PathBuf::from("./boot").join(img);
        process_sins(&mut dev, path.as_path(), other_slot).unwrap();
    }

    // print_firmware_history(&mut usb).unwrap();

    dev.set_active_slot(other_slot).unwrap();
    dev.exit_flash_mode().unwrap();

    dev.command_expect_header("Sync", Header::Okay).unwrap();

    dev.reboot_system().unwrap();
}

fn sin_files() -> Vec<PathBuf> {
    let sins = fs::read_dir("./")
        .unwrap()
        .filter_map(is_sin_file)
        .filter(|path| !path.file_name().unwrap() .to_string_lossy().contains("persist"))
        .collect::<Vec<_>>();
    info!("Found {} .sin files", sins.len());
    sins
}