#[cfg(test)]
mod tests {
    use std::ffi::{c_char, CStr};
    use arrayvec::ArrayVec;
    use crate::*;

    unsafe extern "C" {
        pub fn trim(ptr: *mut c_char);

        pub fn parseoct(p: *const c_char, n: usize) -> i32;
        pub fn is_end_of_archive(p: *const u8) -> i32;
    }

    const VID: u16 = 0x0FCE;
    const PID: u16 = 0xB00B;

    #[test]
    fn test_() {
        let mut message = [0u8; 4096];
        let mut reply = ArrayVec::<u8, 4096>::new();

        let mut usb = get_flash_mode_rs(VID, PID);

        let str = b"getvar:max-download-size";
        message[..str.len()].clone_from_slice(str);

        assert!(transfer_bulk_rs(&mut usb, Direction::Out, &mut message.as_mut()[..str.len()], true).is_ok());
        assert!(get_reply_rs(&mut usb, &mut reply, false));
        assert!(&reply[..4].ne(b"FAIL"));

        if reply.last().is_some_and(|b| *b == 0) {
            // remove null terminator
            reply.truncate(reply.len() - 1);
        }

        let max_download_size = str::from_utf8(&reply).expect(&format!("Failed to parse {:?} as str", reply.as_slice()))
                                            .parse::<i32>().expect(&format!("Failed to parse {:?} as i32", str));

        assert_eq!(max_download_size, 805_306_368);
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