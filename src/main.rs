use crate::fastboot::FastbootDevice;
use crate::sin::Sin;
use crate::ta::TrimArea;
use crate::types::{ProgressBar, Slot};
use crate::xml::BootConfiguration;
use anyhow::{Context, bail, ensure};
use log::warn;
use nusb::Speed;
use std::io::{Read, stdin};

pub mod tests;
pub mod types;
pub mod utils;
pub mod sin;
pub mod xml;
pub mod ta;
mod fastboot;

fn main() -> anyhow::Result<()> {
    env_logger::builder()
        .format_timestamp(None)
        // .format(|buf, record| writeln!(buf, "{}", record.args()))
        .init();

    let args: Vec<String> = std::env::args().collect();

    ensure!(args.len() >= 2, "Missing directory argument");

    std::env::set_current_dir(args[1].as_str())?;

    let mut dev = FastbootDevice::open()?;

    /* fastboot variables */
    let _max_download_size = dev.getvar_u32("max-download-size")?;
    let product = dev.getvar_string("product")?;

    // basic checks that these files are for the correct device
    if !std::env::current_dir()?
        .file_name()
        .unwrap()
        .to_string_lossy()
        .contains(product.as_str()) {
        bail!("Specified directory is probably not for this device!");
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
    let root_key_hash = dev.get_data("Get-root-key-hash")?.to_hexadecimal_string();

    let _slot_count = dev.getvar_u32("slot-count")?;

    ensure!(_slot_count == 2, "Slot count is not 2! Potentially unsupported device");

    let current_slot: Slot = dev.getvar_string("current-slot")?.try_into()?;
    let other_slot = current_slot.other();
    let battery = dev.getvar_u32("Battery");

    match battery {
        Ok(b) => if b < 15 { bail!("Battery level is too low ({b} < 15). Charge your device before flashing!") },
        Err(e) => warn!("{e}"),
    }

    /* parse relevant files first */
    let mut bar = ProgressBar::new(4, "Parsing firmware files");
    let sins = bar.step(|| sin::parse_all_cwd())?;
    let tas = bar.step(|| ta::parse_all_cwd())?;
    let boot = bar.step(|| xml::boot_delivery(Some(&root_key_hash)))?;
    let partition = bar.step(|| xml::partition_delivery())?;
    bar.okay();

    println!("Active Slot: {current_slot}");
    println!("Ready to flash device, press any key to continue ...");

    loop {
        if let Ok(_) = stdin().read(&mut [0u8]) {
            break;
        }
    }

    /* begin flashing */
    match (|| -> anyhow::Result<()> {
        dev.enter_flash_mode()?;
        flash_sins(&mut dev, sins, other_slot)?;
        flash_trim_areas(&mut dev, tas)?;
        flash_boot(&mut dev, boot, other_slot)?;
        flash_partitions(&mut dev, partition, other_slot)
    })() {
        Ok(_) => {
            dev.exit_flash_mode()?;
            dev.sync()?;

            println!("Flashing complete");

            dev.set_active_slot(other_slot)?;
            dev.print_firmware_history()?;
            dev.reboot_system()
        },
        Err(e) if is_connection_aborted(&e) => {
            bail!("{}", e.context("Device disconnected! Check the USB connection and try again"))
        },
        Err(e) => {
            dev.exit_flash_mode()?;
            dev.sync()?;
            bail!("{}", e)
        }
    }
}

fn flash_sins(dev: &mut FastbootDevice, sins: Vec<Sin>, slot: Slot) -> anyhow::Result<()> {
    println!("{:─<97}", "Processing .sin files ");

    // can flash some sin files and still boot
    for sin in sins {
        sin.flash(dev, slot).with_context(|| format!("Failed to flash {}", sin.prefix))?
    };

    Ok(())
}

fn flash_trim_areas(dev: &mut FastbootDevice, tas: Vec<TrimArea>) -> anyhow::Result<()> {
    println!("{:─<97}", "Processing .ta files ");

    // can flash only one ta at a time and still boot
    for ta in tas {
        ta.flash(dev).with_context(|| format!("Failed to flash {}", ta.name))?;
    }

    Ok(())
}

fn flash_boot(dev: &mut FastbootDevice, boot: BootConfiguration, slot: Slot) -> anyhow::Result<()> {
    println!("{:─<97}", "Processing boot delivery ");

    boot.boot_config.flash(dev)?;

    // seems to be just one file
    for sin in &boot.boot_images {
        sin.flash(dev, slot).with_context(|| format!("Failed to flash {}", sin.prefix))?;
    }

    Ok(())
}

fn flash_partitions(dev: &mut FastbootDevice, partition: Vec<Sin>, slot: Slot) -> anyhow::Result<()> {
    println!("{:─<97}", "Processing ./partition files ");

    // failing here causes "your device is corrupted and cannot be trusted" error to appear on boot
    for sin in partition {
        sin.flash(dev, slot).with_context(|| format!("Failed to flash {}", sin.prefix))?;
    }

    Ok(())
}

fn is_connection_aborted(error: &anyhow::Error) -> bool {
    error.chain()
         .filter_map(|cause| cause.downcast_ref::<std::io::Error>())
         .any(|io_error| io_error.kind() == std::io::ErrorKind::ConnectionAborted)
}