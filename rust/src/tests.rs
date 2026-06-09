#[cfg(test)]
mod tests {
    use std::ffi::{c_char, CStr};
    use crate::*;

    unsafe extern "C" {
        pub fn trim(ptr: *mut c_char);

        pub fn parseoct(p: *const c_char, n: usize) -> i32;
        pub fn is_end_of_archive(p: *const u8) -> i32;
    }

    const VID: u16 = 0x0FCE;
    const PID: u16 = 0xB00B;

    fn reply_str<'a>(buffer: &'a mut Vec<u8>, usb: &mut UsbInterfaces, var: &[u8]) -> &'a str {
        if let Err(e) = fastboot_cmd(usb, buffer, var) {
            error!("{}", e);
            return "";
        };

        str::from_utf8(buffer).expect(&format!("Failed to parse {:?} as str", buffer.as_slice()))
    }

    // https://android.googlesource.com/platform/system/core/+/master/fastboot/README.md
    #[test]
    fn test_fastboot_vars() {
        let mut usb = get_flash_mode_rs(VID, PID);
        let mut vec = Vec::<u8>::with_capacity(1024);

        assert_eq!(reply_str(&mut vec, &mut usb, b"getvar:max-download-size").parse::<u32>().unwrap(), 805306368);
        assert_eq!(reply_str(&mut vec, &mut usb, b"getvar:product"), "XQ-EC72");
        assert_eq!(reply_str(&mut vec, &mut usb, b"getvar:version"), "0.4");
        assert_eq!(reply_str(&mut vec, &mut usb, b"getvar:version-bootloader"), "8650-0001_X_Boot_SM8650_LA1.0_U_36");
        assert_eq!(reply_str(&mut vec, &mut usb, b"getvar:serialno"), std::env::var("SERIAL_NO").unwrap().as_str());
        assert_eq!(reply_str(&mut vec, &mut usb, b"getvar:secure"), "no");
        assert_eq!(reply_str(&mut vec, &mut usb, b"getvar:Sector-size"), "4096");
        assert_eq!(reply_str(&mut vec, &mut usb, b"getvar:Loader-version"), "8650-0001_X_Boot_SM8650_LA1.0_U_36");
        assert_eq!(reply_str(&mut vec, &mut usb, b"getvar:Phone-id"), std::env::var("PHONE_ID").unwrap().as_str());
        assert_eq!(reply_str(&mut vec, &mut usb, b"getvar:Device-id"), std::env::var("DEVICE_ID").unwrap().as_str());
        assert_eq!(reply_str(&mut vec, &mut usb, b"getvar:Platform-id"), "202270E1");
        assert_eq!(reply_str(&mut vec, &mut usb, b"getvar:Rooting-status"), "ROOTED");
        assert_eq!(reply_str(&mut vec, &mut usb, b"getvar:Ufs-info"), "KIOXIA,THGJFLT1E45BATPB,0100");
        assert_eq!(reply_str(&mut vec, &mut usb, b"getvar:Emmc-info"), "Emmc-info not supported");
        assert_eq!(reply_str(&mut vec, &mut usb, b"getvar:Default-security"), "ON");
        assert_eq!(reply_str(&mut vec, &mut usb, b"getvar:Keystore-counter"), "2");
        assert_eq!(reply_str(&mut vec, &mut usb, b"getvar:Security-state"), std::env::var("SECURITY_STATE").unwrap().as_str());
        assert_eq!(reply_str(&mut vec, &mut usb, b"getvar:S1-root"), "S1_Root_398d");
        assert_eq!(reply_str(&mut vec, &mut usb, b"getvar:Sake-root"), "5515");

        fastboot_cmd(&mut usb, &mut vec, b"Get-root-key-hash").unwrap();
        assert_eq!(vec.len(), 48);
        assert_eq!(vec.iter().map(|b| format!("{:02X}", b)).collect::<String>(), std::env::var("ROOT_KEY_HASH").unwrap());

        assert_eq!(reply_str(&mut vec, &mut usb, b"getvar:slot-count"), "2");
        assert_eq!(reply_str(&mut vec, &mut usb, b"getvar:current-slot"), "a");
        assert_eq!(reply_str(&mut vec, &mut usb, b"getvar:Battery"), "Battery not supported");
    }

    // #[test]
    // fn test_fastboot_download() {
    //     let mut vec = Vec::<u8>::new();
    //     let mut usb = get_flash_mode_rs(VID, PID);
    //
    //     assert_eq!(check_reply(&mut vec, &mut usb, b"getvar:max-download-size").parse::<u32>().unwrap(), 805306368);
    //
    //     output_bulk(&mut usb, b"download:00000001").unwrap();
    //
    //     // let mut buf = [0u8; 12];
    //     vec.resize(12, 0);
    //     usb.reader.read_exact(vec.as_mut()).unwrap();
    //
    //     assert_eq!(str::from_utf8(&vec).expect(&format!("Failed to parse {:?} as str", vec)), "DATA00000001");
    // }

    #[test]
    fn test_files() {
        let text = c"files/text".as_ptr();
        assert_eq!(file_exist(text), 1);
        assert_eq!(file_size(text), 26);
    }

    #[test]
    fn test_trim() {
        unsafe {
            let str = b"  this is\r a\n test\t string  .  \0".to_vec();
            trim(str.as_ptr() as *mut c_char);
            assert_eq!(CStr::from_ptr(str.as_ptr() as *const c_char), c"thisisateststring.");

            let str1 = b"\r\n\t why would you type\r\r\r\r \n\n\n\n \t\t\t\t like this\0".to_vec();
            trim(str1.as_ptr() as *mut c_char);
            assert_eq!(CStr::from_ptr(str1.as_ptr() as *const c_char), c"whywouldyoutypelikethis");
        }
    }

    // #[test]
    // fn test_display_buffer_hex_ascii() {
    //     unsafe {
    //         let m = c"message".as_ptr();
    //         let b = c"buffer".as_ptr();
    //         display_buffer_hex_ascii(m, b, 6);
    //     }
    // }

    #[test]
    fn test_parseoct() {
        unsafe {
            assert_eq!(parseoct(c"123".as_ptr(), 3), 0b001010011);
            assert_eq!(parseoct(c"1000".as_ptr(), 4), 0b1000000000);
            assert_eq!(parseoct(c"777".as_ptr(), 3), 0b111111111);

            assert_eq!(parseoct(c"a777z".as_ptr(), 5), 0b111111111);
        }
    }

    #[test]
    fn test_is_end_of_archive() {
        unsafe {
            let mut arr = [0u8; 512];
            assert_eq!(arr.len(), 512);

            assert_eq!(is_end_of_archive(arr.as_ptr()), 1);

            arr[0] = 0xFF;
            assert_eq!(is_end_of_archive(arr.as_ptr()), 0);

            arr[0] = 0x0;
            arr[511] = 0xFF;
            assert_eq!(is_end_of_archive(arr.as_ptr()), 0);
        }
    }
}