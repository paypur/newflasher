use std::ffi::c_char;

// #[link(name = "newflasher_c", kind = "static")]
unsafe extern "C" {
    pub fn trim(ptr: *mut c_char);
    pub fn parseoct(p: *const c_char, n: usize) -> i32;
    pub fn is_end_of_archive(p: *const u8) -> i32;
}

#[cfg(test)]
mod tests {
    use std::ffi::{c_char, CStr};
    use super::*;

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