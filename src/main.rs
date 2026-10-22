use crate::fastboot::FastbootDevice;
use crate::sins::Sin;
use crate::ta::TrimArea;
use crate::types::Slot;
use crate::xml_parser::BootConfiguration;
use anyhow::Context;
use std::io::{Read, stdin};
use std::path::PathBuf;
use std::process::exit;

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
        panic!("Specified directory is probably not for this device!");
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
        panic!("Battery level is too low ({battery} < 15). Charge your device before flashing!");
    }

    /* parse relevant files first */
    let tas = ta::parse_all().unwrap();
    let partition = xml_parser::partition_delivery().unwrap().iter().map(|p| Sin::open(p).unwrap()).collect();
    let sins = Sin::open_all().unwrap();
    let boot = xml_parser::boot_delivery(&root_key_hash).unwrap();
    let boot_ta = TrimArea::try_from_file(PathBuf::from("./boot").join(&boot.boot_config)).unwrap();

    println!("Active Slot: {current_slot}");
    println!("Ready to flash device, press any key to continue ...");

    loop {
        if let Ok(_) = stdin().read(&mut [0u8]) {
            break;
        }
    }

    /* begin flashing */
    dev.enter_flash_mode();

    flash_sins(&mut dev, sins, other_slot);
    flash_trim_areas(&mut dev, tas);
    flash_boot(&mut dev, boot, boot_ta, other_slot);
    flash_partitions(&mut dev, partition, other_slot);

    // print_firmware_history(&mut usb).unwrap();

    dev.set_active_slot(other_slot);
    dev.exit_flash_mode();
    dev.sync();
    dev.reboot_system();
}

fn flash_sins(dev: &mut FastbootDevice, sins: Vec<Sin>, slot: Slot) {
    println!("{:─<97}", "Processing .sin files ");

    // can flash some sin files and still boot
    for sin in sins {
        if let Err(e) = sin.flash(dev, slot).with_context(|| format!("Failed to flash {}", sin.prefix)) {
            dev.exit_flash_mode();
            dev.sync();
            panic!("{e}");
        }
    };
}

fn flash_trim_areas(dev: &mut FastbootDevice, tas: Vec<TrimArea>) {
    println!("{:─<97}", "Processing .ta files ");

    // can flash only one ta at a time and still boot
    for ta in tas {
        if let Err(e) = ta.flash(dev).with_context(|| format!("Failed to flash {}", ta.name)) {
            dev.exit_flash_mode();
            dev.sync();
            panic!("{e}");
        }
    }
}

fn flash_boot(dev: &mut FastbootDevice, boot: BootConfiguration, boot_ta: TrimArea, slot: Slot) {
    println!("{:─<97}", "Processing boot delivery ");

    boot_ta.flash(dev).unwrap();

    // seems to be just one file
    for img in &boot.boot_images {
        let path = PathBuf::from("./boot").join(img);
        let sin = Sin::open(path).unwrap();
        sin.flash(dev, slot).with_context(|| format!("Failed to flash {}", sin.prefix)).unwrap();
    }
}

fn flash_partitions(dev: &mut FastbootDevice, partition: Vec<Sin>, slot: Slot) {
    println!("{:─<97}", "Processing ./partition files ");

    // failing here causes "your device is corrupted and cannot be trusted" error to appear on boot
    for sin in partition {
        sin.flash(dev, slot).with_context(|| format!("Failed to flash {}", sin.prefix)).unwrap();
    }
}
