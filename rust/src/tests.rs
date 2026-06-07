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

    #[test]
    fn test_fastboot_vars() {
        let mut vec = Vec::<u8>::new();
        let mut usb = get_flash_mode_rs(VID, PID);

        // let max_download_size = str::from_utf8(&reply).expect(&format!("Failed to parse {:?} as str", reply.as_slice()))
        //                                     .parse::<i32>().expect(&format!("Failed to parse {:?} as i32", str));

        // $ fastboot getvar all
        assert!(check_reply(&mut vec, &mut usb, b"getvar:max-download-size", "805306368"));
        assert!(check_reply(&mut vec, &mut usb, b"getvar:product", "XQ-EC72"));
        assert!(check_reply(&mut vec, &mut usb, b"getvar:version", "0.4"));
        assert!(check_reply(&mut vec, &mut usb, b"getvar:version-bootloader", "8650-0001_X_Boot_SM8650_LA1.0_U_36"));
        // test_var_string(message, &mut reply, &mut usb, b"getvar:serialno", "");
        assert!(check_reply(&mut vec, &mut usb, b"getvar:secure", "no"));
        assert!(check_reply(&mut vec, &mut usb, b"getvar:Sector-size", "4096"));
        assert!(check_reply(&mut vec, &mut usb, b"getvar:Loader-version", "8650-0001_X_Boot_SM8650_LA1.0_U_36"));
        // test_var_string(message, &mut reply, &mut usb, b"getvar:Phone-id", "");
        // test_var_string(message, &mut reply, &mut usb, b"getvar:Device-id", "");
        assert!(check_reply(&mut vec, &mut usb, b"getvar:Platform-id", "202270E1"));
        assert!(check_reply(&mut vec, &mut usb, b"getvar:Rooting-status", "ROOTED"));
        assert!(check_reply(&mut vec, &mut usb, b"getvar:Ufs-info", "KIOXIA,THGJFLT1E45BATPB,0100"));
        assert!(check_reply(&mut vec, &mut usb, b"getvar:Emmc-info", "Emmc-info not supported"));
        assert!(check_reply(&mut vec, &mut usb, b"getvar:Default-security", "ON"));
        assert!(check_reply(&mut vec, &mut usb, b"getvar:Keystore-counter", "2"));
        assert!(check_reply(&mut vec, &mut usb, b"getvar:Security-state", ""));
        assert!(check_reply(&mut vec, &mut usb, b"getvar:S1-root", "S1_Root_398d"));
        assert!(check_reply(&mut vec, &mut usb, b"getvar:Sake-root", "5515"));
        assert!(check_reply(&mut vec, &mut usb, b"getvar:Get-root-key-hash", "Get-root-key-hash not supported"));
        assert!(check_reply(&mut vec, &mut usb, b"getvar:slot-count", "2"));
        assert!(check_reply(&mut vec, &mut usb, b"getvar:current-slot", "a"));
        assert!(check_reply(&mut vec, &mut usb, b"getvar:Battery", "Battery not supported"));
    }

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