#[cfg(test)]
mod tests {
    use std::ffi::{c_uint, CString};
    use crate::utils::{file_exist, file_size};
    use crate::utils::{parseoct, trim_rs};
    use crate::*;

    use std::fs::File;
    use std::io::Read;
    use std::path::PathBuf;
    use std::slice::from_raw_parts;
    use std::sync::{Mutex, OnceLock};
    use tar::Archive;
    use crate::sins::{transfer_cms};
    use crate::ta::{process_trim_area, TrimArea};
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

    #[test]
    fn test_xml() {
        let base = PathBuf::from("../../XQ-EC72_Customized_HK_69.2.A.4.90/");

        println!("{:?}", std::env::current_dir());

        let partition_files = partition_delivery(base.join("partition/partition_delivery.xml")).unwrap();
        println!("partition files: {:#?}", partition_files);
        println!();
        let boot = boot_delivery(base.join("boot/boot_delivery.xml")).unwrap();
        println!("Boot Delivery: {:#?}", boot);
    }

    #[test]
    fn test_ta_files() {
        test_both_ta("../../XQ-EC72_Customized_HK_69.2.A.4.90/auto-boot.ta", TrimArea { partition: 2, unit: 0x907, data: ByteVec::from([0x0]) });
        test_both_ta("../../XQ-EC72_Customized_HK_69.2.A.4.90/CustomerID_S20000480_001_HK_c001526.ta", TrimArea { partition: 2, unit: 0x87B, data: ByteVec::from([0x63, 0x30, 0x30, 0x31, 0x35, 0x32, 0x36]) });
        test_both_ta("../../XQ-EC72_Customized_HK_69.2.A.4.90/osv-restriction.ta", TrimArea { partition: 2, unit: 0x91A, data: ByteVec::from([0x0]) });
        test_both_ta("../../XQ-EC72_Customized_HK_69.2.A.4.90/reset-kernel-cmd-debug.ta", TrimArea { partition: 2, unit: 0x9A9, data: ByteVec::from([0x0]) });
        test_both_ta("../../XQ-EC72_Customized_HK_69.2.A.4.90/reset-retail-demo-active-sts.ta", TrimArea { partition: 2, unit: 0xA1E, data: ByteVec::new() });
    }

    // https://android.googlesource.com/platform/system/core/+/master/fastboot/README.md
    #[test]
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
    }

    #[test]
    fn test_fastboot_flashmode() {
        let mut usb = get_device().lock().unwrap();

        usb.download(&[0u8]).unwrap();
        usb.command_expect("Write-TA:2:10100", FastbootHeader::Okay).unwrap();
    }

    #[test]
    fn test_sin_signature() {
        let mut usb = get_device().lock().unwrap();

        println!("{:?}", std::env::current_dir());

        let file = File::open("../../XQ-EC72_Customized_HK_69.2.A.4.90/partition/partition-image-LUN0_124936192_X-FLASH-ALL-88DF.sin").unwrap();
        let mut archive = Archive::new(Box::new(file) as Box<dyn Read>);

        let mut fst = archive.entries().unwrap().nth(0).unwrap().unwrap();

        transfer_cms(&mut usb, &mut fst, "partitionimage_0").unwrap();
    }

    // #[test]
    // fn test_firmware_history() {
    //     let mut usb = get_device().lock().unwrap();
    //
    //     usb.get_data("Read-TA:2:2475").unwrap();
    //
    //     println!("{:#?}", usb.reply);
    // }

    #[test]
    fn test_files() {
        let text = c"files/text".as_ptr();
        assert_eq!(file_exist(text), 1);
        assert_eq!(file_size(text), 26);
    }

    // #[test]
    // fn test_trim() {
    //     let str = "  this is\r a\n test\t string  .  ";
    //     assert_eq!(trim_rs(str), "thisisateststring.");
    //
    //     let str1 = "\r\n\t why would you type\r\r\r\r \n\n\n\n \t\t\t\t like this";
    //     assert_eq!(trim_rs(str1), "whywouldyoutypelikethis");
    // }

    #[test]
    fn test_parseoct() {
        assert_eq!(parseoct(c"123".as_ptr(), 3), 0b001010011);
        assert_eq!(parseoct(c"1000".as_ptr(), 4), 0b1000000000);
        assert_eq!(parseoct(c"777".as_ptr(), 3), 0b111111111);

        assert_eq!(parseoct(c"a777z".as_ptr(), 5), 0b111111111);
    }

    fn test_both_ta(file: &str, expected: TrimArea) {
        let path = PathBuf::from(file);
        let string = CString::new(path.to_str().unwrap()).unwrap();
        let cstr = string.as_ptr();

        let ta_rs = process_trim_area(&path).unwrap().unwrap();
        let ta_c = unsafe { process_ta_file_c(cstr) };

        assert_eq!(ta_rs, expected);
        assert_eq!(ta_c.partition, expected.partition);
        assert_eq!(ta_c.unit, expected.unit);
        assert_eq!(ta_c.size, expected.data.len());
        assert_eq!(unsafe { from_raw_parts(ta_c.data, ta_c.size) }, expected.data.as_slice());
    }

    fn reply_str<'a>(usb: &'a mut FastbootDevice, cmd: &str) -> &'a str {
        if let Err(e) = usb.command(cmd) {
            error!("{}", e);
            return "";
        };

        str::from_utf8(usb.reply.as_slice()).expect(&format!("Failed to parse {:?} as str", usb.reply.as_slice()))
    }
}