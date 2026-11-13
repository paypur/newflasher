#[cfg(test)]
mod tests {
    use crate::*;
    use std::{env, fs};

    use crate::ta::{BootConfigUnit, TrimArea};
    use crate::types::ByteVec;
    use std::fs::File;
    use std::io::Read;
    use std::path::{Path, PathBuf};
    use std::sync::{Mutex, OnceLock};
    use tar::Archive;
    use crate::sin::Sin;
    use crate::utils::is_sin_file;

    static DEVICE: OnceLock<Mutex<FastbootDevice>> = OnceLock::new();

    fn get_device() -> &'static Mutex<FastbootDevice> {
        DEVICE.get_or_init(|| Mutex::new(FastbootDevice::open().expect("Failed to open fastboot device")))
    }

    fn test_in_dir(dir: impl AsRef<Path>, f: impl Fn()) {
        let dir = dir.as_ref();
        let cwd = std::env::current_dir().unwrap();

        std::env::set_current_dir(dir).unwrap();

        f();

        std::env::set_current_dir(cwd).unwrap();
    }

    #[test]
    fn recover_com() {
        env::set_current_dir("/media/secondary/xperia/H8314_Customized_US_52.1.A.3.137-R7C").unwrap();
        // env::set_current_dir("/media/secondary/xperia/XQ-EC72_Customized_HK_69.2.A.4.110").unwrap();

        let to_mib = |n: usize| -> usize { n / 1024 / 1024 };

        let mut dev = get_device().lock().unwrap();

        let paths = fs::read_dir(".").unwrap()
            .filter_map(is_sin_file)
            .max_by_key(|p| fs::metadata(p).unwrap().len())
            .unwrap();

        println!("{:?}", paths);

        let sin = Sin::parse(paths).unwrap();

        println!("Speed: {:?}", dev.speed());

        let mut total = 0;
        let mut duration = std::time::Duration::new(0, 0);

        for (i, chunk) in sin.chunks().enumerate() {
            let now = std::time::Instant::now();
            dev.download(chunk).unwrap();
            let elapsed = now.elapsed();

            total += chunk.len();
            duration += elapsed;

            println!("Downloaded chunk {i}, {} MiB in {:3.3}s. {:3.0}MiB/s", to_mib(chunk.len()), elapsed.as_secs_f64(), to_mib(chunk.len()) as f64 / elapsed.as_secs_f64());
        }

        // xc2c usb2.1 43 MiB/s
        // xc2c usb3 265 MiB/s
        // 1VI usb3 414 MiB/s
        println!("Downloaded {} MiB in {:3.3}s. Average of {:3.0}MiB/s", to_mib(total), duration.as_secs_f64(), to_mib(total) as f64 / duration.as_secs_f64());
    }

/*    // https://android.googlesource.com/platform/system/core/+/master/fastboot/README.md
    // #[test]
    fn test_fastboot_vars() {
        let mut usb = get_device().lock().unwrap();

        assert_eq!(reply_str(&mut usb, "getvar:max-download-size").parse::<u32>().unwrap(), 805306368);
        assert_eq!(reply_str(&mut usb, "getvar:product"), "XQ-EC72");
        assert_eq!(reply_str(&mut usb, "getvar:version"), "0.4");
        assert_eq!(reply_str(&mut usb, "getvar:version-bootloader"), "8650-0001_X_Boot_SM8650_LA1.0_U_36");
        assert_eq!(reply_str(&mut usb, "getvar:serialno"), std::env::var("SERIAL_NO").unwrap().as_str());
        assert_eq!(reply_str(&mut usb, "getvar:secure"), "no");
        assert_eq!(reply_str(&mut usb, "getvar:Sector-size"), "4096");
        assert_eq!(reply_str(&mut usb, "getvar:Loader-version"), "8650-0001_X_Boot_SM8650_LA1.0_U_36");
        assert_eq!(reply_str(&mut usb, "getvar:Phone-id"), std::env::var("PHONE_ID").unwrap().as_str());
        assert_eq!(reply_str(&mut usb, "getvar:Device-id"), std::env::var("DEVICE_ID").unwrap().as_str());
        assert_eq!(reply_str(&mut usb, "getvar:Platform-id"), "202270E1");
        assert_eq!(reply_str(&mut usb, "getvar:Rooting-status"), "ROOTED");
        assert_eq!(reply_str(&mut usb, "getvar:Ufs-info"), "KIOXIA,THGJFLT1E45BATPB,0100");
        assert_eq!(reply_str(&mut usb, "getvar:Emmc-info"), "Emmc-info not supported");
        assert_eq!(reply_str(&mut usb, "getvar:Default-security"), "ON");
        assert_eq!(reply_str(&mut usb, "getvar:Keystore-counter"), "2");
        assert_eq!(reply_str(&mut usb, "getvar:Security-state"), std::env::var("SECURITY_STATE").unwrap().as_str());
        assert_eq!(reply_str(&mut usb, "getvar:S1-root"), "S1_Root_398d");
        assert_eq!(reply_str(&mut usb, "getvar:Sake-root"), "5515");

        usb.get_data("Get-root-key-hash").unwrap();
        assert_eq!(usb.reply.len(), 48);
        assert_eq!(usb.reply.iter().map(|b| format!("{:02X}", b)).collect::<String>(), std::env::var("ROOT_KEY_HASH").unwrap());

        assert_eq!(reply_str(&mut usb, "getvar:slot-count"), "2");
        assert_eq!(reply_str(&mut usb, "getvar:current-slot"), "a");
        assert_eq!(reply_str(&mut usb, "getvar:Battery"), "Battery not supported");
    }*/

    #[test]
    fn test_fastboot_vars_xz2c() {
        let mut usb = get_device().lock().unwrap();

        assert_eq!(usb.getvar_u32("max-download-size").unwrap(), 104857600);
        assert_eq!(usb.getvar_string("product").unwrap(), "H8314");
        assert_eq!(usb.getvar_string("version").unwrap(), "0.4");
        assert_eq!(usb.getvar_string("version-bootloader").unwrap(), "1310-7079_X_Boot_SDM845_LA2.0.1_Q_207");
        assert_eq!(usb.getvar_string("serialno").unwrap(), "BH9006VJCU");
        assert_eq!(usb.getvar_string("secure").unwrap(), "no");
        assert_eq!(usb.getvar_string("Sector-size").unwrap(), "4096");
        assert_eq!(usb.getvar_string("Loader-version").unwrap(), "XFL-SDM845-O-25");
        assert_eq!(usb.getvar_string("Phone-id").unwrap(), "0000:35498709917379");
        assert_eq!(usb.getvar_string("Device-id").unwrap(), "A09A3D8A");
        assert_eq!(usb.getvar_string("Platform-id").unwrap(), "2008B0E1");
        assert_eq!(usb.getvar_string("Rooting-status").unwrap(), "ROOTED");
        assert_eq!(usb.getvar_string("Ufs-info").unwrap(), "TOSHIBA,THGAF8G9T43BAIRB,0300");
        assert_eq!(usb.getvar_string("Emmc-info").unwrap(), "Emmc-info not supported");
        assert_eq!(usb.getvar_string("Default-security").unwrap(), "ON");
        assert_eq!(usb.getvar_string("Keystore-counter").unwrap(), "2");
        assert_eq!(usb.getvar_string("Security-state").unwrap(), "U0vSz8eM1gk1FZup2A6hQlhcZnS9ZR3Up1pg2aPJewY=");
        assert_eq!(usb.getvar_string("S1-root").unwrap(), "S1_Root_e090");
        assert_eq!(usb.getvar_string("Sake-root").unwrap(), "B7DF");

        assert_eq!(usb.get_data("Get-root-key-hash ").unwrap().to_hexadecimal_string(), "C8D92A2DBA1482588EFF712963329500AC7155CEA8494FDD4B7B180F2F871801");

        assert_eq!(usb.getvar_string("slot-count").unwrap(), "2");
        assert_eq!(usb.getvar_string("current-slot").unwrap(), "a");


        assert_eq!(usb.getvar_string("has-slot:oem").unwrap(), "yes");
        assert_eq!(usb.getvar_string("has-slot:system").unwrap(), "yes");
        assert_eq!(usb.getvar_string("has-slot:fsmetadata").unwrap(), "no");
        assert_eq!(usb.getvar_string("has-slot:dsp").unwrap(), "yes");
    }

/*    #[test]
    fn test_fastboot_flashmode() {
        let mut usb = get_device().lock().unwrap();

        enter_flash_mode(&mut *usb);

        // usb.get_data("Get-ufs-info").unwrap();
        // print_hex_ascii("READ", usb.reply.as_slice());

        // let path = PathBuf::from("../../H8314_O2_Pay_monthly_UK_52.1.A.3.49-R6C/partition/partition-image-LUN0_X-FLASH-ALL-B6B5.sin");
        // process_sins_rs(&mut usb, path, "Repartition").unwrap();

        let path = PathBuf::from("../../H8314_O2_Pay_monthly_UK_52.1.A.3.49-R6C/oem_X-FLASH-CUST-B6B5.sin");
        process_sins_rs(&mut usb, path, "flash", Slot::A);
    }*/

    #[test]
    fn test_sin_signature() {
        // let mut usb = get_device().lock().unwrap();

        std::env::set_current_dir("../../H8314_O2_Pay_monthly_UK_52.1.A.3.49-R6C/").unwrap();

        let paths = fs::read_dir(".").unwrap()
                                     .filter(|entry| {
                if let Ok(e) = entry {
                    e.file_name().to_string_lossy().ends_with(".sin")
                } else {
                    false
                }
            })
                                     .map(|entry| entry.unwrap().path());

        let path = PathBuf::from("partition/partition-image-LUN0_X-FLASH-ALL-B6B5.sin");

        // for path in paths {
            let file = File::open(path).unwrap();
            let mut archive = Archive::new(Box::new(file) as Box<dyn Read>);

            for e in archive.entries().unwrap() {
                if let Ok(mut entry) = e {
                    let mut buf = vec![];
                    entry.read_to_end(&mut buf).unwrap();

                    let x = entry.header().as_ustar().unwrap().name;
                    let str = str::from_utf8(&x).unwrap().split('\0').next().unwrap();

                    println!("{} {},\n {}", str, buf.len(), buf.iter().map(|b| format!("{:02X} ", b)).collect::<String>());
                }
            }
        // }

        // transfer_cms(&mut usb, &mut fst, "partitionimage_0").unwrap();
    }

}