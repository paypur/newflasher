#[cfg(test)]
mod tests {
    use std::ffi::{c_char, c_uint, CString};
    use std::fs;
    use crate::*;

    use std::fs::{DirEntry, File};
    use std::io::Read;
    use std::path::{Path, PathBuf};
    use std::slice::from_raw_parts;
    use std::sync::{Mutex, OnceLock};
    use tar::Archive;
    use crate::ta::{TrimArea, BootConfigUnit};
    use crate::types::{ByteVec, FastbootHeader, Slot};
    use crate::xml_parser::{boot_delivery, partition_delivery};

    #[repr(C)]
    #[derive(PartialEq, Debug)]
    struct TA {
        partition: u8,
        unit: usize,
        data: *const u8,
        size: usize
    }

    unsafe extern "C" {
        fn process_ta_file_c(file: *const c_char) -> TA;
    }

    const VID: u16 = 0x0FCE;
    const PID: u16 = 0xB00B;

    static DEVICE: OnceLock<Mutex<FastbootDevice>> = OnceLock::new();

    fn get_device() -> &'static Mutex<FastbootDevice> {
        DEVICE.get_or_init(|| Mutex::new(get_flash_mode_rs(VID, PID)))
    }

    fn test_in_dir(dir: impl AsRef<Path>, f: impl Fn()) {
        let dir = dir.as_ref();
        let cwd = std::env::current_dir().unwrap();

        std::env::set_current_dir(dir).unwrap();

        f();

        std::env::set_current_dir(cwd).unwrap();
    }

    #[test]
    fn test_xml() {
        env_logger::builder()
            .format_timestamp(None)
            .init();

        test_in_dir("tests/data/H8314_Customized_US_52.1.A.3.137-R7C/", || {
            let boot = boot_delivery("C8D92A2DBA1482588EFF712963329500AC7155CEA8494FDD4B7B180F2F871801").unwrap();
            assert_eq!(boot.space_id, String::from("1310-7079"));
            assert_eq!(boot.config._name, String::from("COMMERCIAL_0008B0E1"));
            assert!(boot.config.plf_root_hash.starts_with("C8D92A2DBA1482588EFF712963329500AC7155CEA8494FDD"));
            assert_eq!(boot.config.hw_config_rev, String::from("HWC_Tama_Com_001"));
            assert_eq!(boot.config.boot_config, String::from("Apollo_XBootConfig_MiscTA.ta"));
            assert_eq!(boot.config.boot_images, vec![String::from("bootloader_X_BOOT_SDM845_LA2_0_1_Q_208_X-FLASH-ALL-B6B5.sin")]);
        });

        test_in_dir("tests/data/XQ-EC72_Customized_HK_69.2.A.4.110/", || {
            let boot = boot_delivery(std::env::var("ROOT_KEY_HASH").unwrap().as_str()).unwrap();
            assert_eq!(boot.space_id, String::from("8650-0001"));
            assert_eq!(boot.config._name, String::from("COMMERCIAL_002270E1"));
            assert!(boot.config.plf_root_hash.starts_with("2B9EB2535D4A1179B294D4D083F68D9D3DDC95164111B84157D8BE0169E35DE5C6F79EA6E1034FE3D7A684F3C5166250"));
            assert_eq!(boot.config.hw_config_rev, String::from("HWC_XGeneric_001"));
            assert_eq!(boot.config.boot_config, String::from("PDX245_XBootConfig_MiscTA.ta"));
            assert_eq!(boot.config.boot_images, vec![String::from("bootloader_X_BOOT_SM8650_LA1_0_U_37_X-FLASH-ALL-88DF.sin")]);
        });

        // test_in_dir("tests/data/XQ-GE74_Customized_HK_73.1.A.2.61/", || {
        //     let boot = boot_delivery("2008B0E1", "C8D92A2DBA1482588EFF712963329500AC7155CEA8494FDD4B7B180F2F871801").unwrap();
        //     assert_eq!(boot.space_id, String::from("8850-0001"));
        //     assert_eq!(boot.config._name, String::from("COMMERCIAL_002B30E1"));
        //     assert_eq!(boot.config.platform_id, None);
        //     assert_eq!(boot.config.plf_root_hash, String::from("D3090E374B6A20700E326BE82BB24BF04A492459DC8FD6BF24C6E5F8DD60A0F84221BE752E8F4BC0802F578E16B84E90"));
        //     assert_eq!(boot.config.hw_config_rev, String::from("HWC_XGeneric_001"));
        //     assert_eq!(boot.config.boot_config, String::from("PDX267_XBootConfig_MiscTA.ta"));
        //     assert_eq!(boot.config.boot_images, vec![String::from("bootloader_X_BOOT_SM8850_LA1_0_A16_22_X-FLASH-ALL-DCBF.sin")]);
        // });

    }

/*    #[test]
    fn test_ta_files() {
        let compare = |file: &str, partition: u8, unit: usize, data: &[u8] | {
            let result = TrimArea::try_from_file(PathBuf::from(file)).unwrap();
            let units = match data.is_empty() {
                false => vec![BootConfigUnit { unit, data: ByteVec::from(data)}],
                true => vec![],
            };
            // TODO: needs to be public, but dont want to expose
            let expected = TrimArea::new(partition, units);
            assert_eq!(result, expected)
        };

        compare("../../XQ-EC72_Customized_HK_69.2.A.4.90/auto-boot.ta", 2, 0x907, &[0]);
        compare("../../XQ-EC72_Customized_HK_69.2.A.4.90/CustomerID_S20000480_001_HK_c001526.ta", 2, 0x87B, &[0x63, 0x30, 0x30, 0x31, 0x35, 0x32, 0x36]);
        compare("../../XQ-EC72_Customized_HK_69.2.A.4.90/osv-restriction.ta", 2, 0x91A, &[0x0]);
        compare("../../XQ-EC72_Customized_HK_69.2.A.4.90/reset-kernel-cmd-debug.ta", 2, 0x9A9, &[0x0]);
        compare("../../XQ-EC72_Customized_HK_69.2.A.4.90/reset-retail-demo-active-sts.ta", 2, 0xA1E, &[]);

        compare("../../H8314_O2_Pay_monthly_UK_52.1.A.3.49-R6C/auto-boot.ta", 2, 0x90C, &[0x0]);
        compare("../../H8314_O2_Pay_monthly_UK_52.1.A.3.49-R6C/cust-reset.ta", 2, 0x8A4, &[0x0, 0x0, 0x0, 0x0, 0x0, 0x0, 0x0, 0x0, 0x0, 0x0, 0x0, 0x0, 0x0, 0x0]);
        compare("../../H8314_O2_Pay_monthly_UK_52.1.A.3.49-R6C/master-reset.ta", 2, 0x9F6, &[0x1]);
        compare("../../H8314_O2_Pay_monthly_UK_52.1.A.3.49-R6C/osv-restriction.ta", 2, 0x91A, &[0x0]);
        compare("../../H8314_O2_Pay_monthly_UK_52.1.A.3.49-R6C/reset-kernel-cmd-debug.ta", 2, 0x9A9, &[0x0]);
        compare("../../H8314_O2_Pay_monthly_UK_52.1.A.3.49-R6C/reset-non-secure-adb.ta",2, 0x9B6, &[]);
        compare("../../H8314_O2_Pay_monthly_UK_52.1.A.3.49-R6C/reset-wipe-reason.ta", 2, 0x9F9, &[]);
        compare("../../H8314_O2_Pay_monthly_UK_52.1.A.3.49-R6C/simlock.ta", 2, 0, &[]); // should be none
    }*/

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

        assert_eq!(usb.getvar_u32("getvar:max-download-size", 0), 104857600);
        assert_eq!(usb.getvar_string("getvar:product").unwrap(), "H8314");
        assert_eq!(usb.getvar_string("getvar:version").unwrap(), "0.4");
        assert_eq!(usb.getvar_string("getvar:version-bootloader").unwrap(), "1310-7079_X_Boot_SDM845_LA2.0.1_Q_207");
        assert_eq!(usb.getvar_string("getvar:serialno").unwrap(), "BH9006VJCU");
        assert_eq!(usb.getvar_string("getvar:secure").unwrap(), "no");
        assert_eq!(usb.getvar_string("getvar:Sector-size").unwrap(), "4096");
        assert_eq!(usb.getvar_string("getvar:Loader-version").unwrap(), "XFL-SDM845-O-25");
        assert_eq!(usb.getvar_string("getvar:Phone-id").unwrap(), "0000:35498709917379");
        assert_eq!(usb.getvar_string("getvar:Device-id").unwrap(), "A09A3D8A");
        assert_eq!(usb.getvar_string("getvar:Platform-id").unwrap(), "2008B0E1");
        assert_eq!(usb.getvar_string("getvar:Rooting-status").unwrap(), "ROOTED");
        assert_eq!(usb.getvar_string("getvar:Ufs-info").unwrap(), "TOSHIBA,THGAF8G9T43BAIRB,0300");
        assert_eq!(usb.getvar_string("getvar:Emmc-info").unwrap(), "Emmc-info not supported");
        assert_eq!(usb.getvar_string("getvar:Default-security").unwrap(), "ON");
        assert_eq!(usb.getvar_string("getvar:Keystore-counter").unwrap(), "2");
        assert_eq!(usb.getvar_string("getvar:Security-state").unwrap(), "U0vSz8eM1gk1FZup2A6hQlhcZnS9ZR3Up1pg2aPJewY=");
        assert_eq!(usb.getvar_string("getvar:S1-root").unwrap(), "S1_Root_e090");
        assert_eq!(usb.getvar_string("getvar:Sake-root").unwrap(), "B7DF");

        usb.get_data("Get-root-key-hash").unwrap();
        assert_eq!(usb.reply.len(), 32);
        assert_eq!(usb.reply.iter().map(|b| format!("{:02X}", b)).collect::<String>(), "C8D92A2DBA1482588EFF712963329500AC7155CEA8494FDD4B7B180F2F871801");

        assert_eq!(usb.getvar_string("getvar:slot-count").unwrap(), "2");
        assert_eq!(usb.getvar_string("getvar:current-slot").unwrap(), "a");


        assert_eq!(usb.getvar_string("getvar:has-slot:oem").unwrap(), "yes");
        assert_eq!(usb.getvar_string("getvar:has-slot:system").unwrap(), "yes");
        assert_eq!(usb.getvar_string("getvar:has-slot:fsmetadata").unwrap(), "no");
        assert_eq!(usb.getvar_string("getvar:has-slot:dsp").unwrap(), "yes");
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

    #[test]
    fn test_firmware_history() {
        let mut usb = get_device().lock().unwrap();

        usb.get_data("Read-TA:2:2475").unwrap();

        println!("{:#?}", usb.reply);
    }

}