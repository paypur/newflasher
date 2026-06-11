#[cfg(test)]
mod tests {
    use crate::utils::parseoct;
use crate::utils::{file_exist, file_size};
use std::ffi::{c_char, CStr, CString};
    use std::path::PathBuf;
    use std::sync::{Mutex, OnceLock};
    use flate2::read::{GzDecoder, MultiGzDecoder, ZlibDecoder};
    use tar::Archive;
    use crate::*;

    unsafe extern "C" {
        // pub fn trim(ptr: *mut c_char);

        pub fn is_end_of_archive(p: *const u8) -> i32;

        pub fn gunziper(in_: *const c_char, out: *const c_char) -> i32;
    }

    const VID: u16 = 0x0FCE;
    const PID: u16 = 0xB00B;

    static DEVICE: OnceLock<Mutex<UsbInterfaces>> = OnceLock::new();

    fn get_device() -> &'static Mutex<UsbInterfaces> {
        DEVICE.get_or_init(|| Mutex::new(get_flash_mode_rs(VID, PID)))
    }

    // https://android.googlesource.com/platform/system/core/+/master/fastboot/README.md
    #[test]
    fn test_fastboot_vars() {
        let mut usb = get_device().lock().unwrap();
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

    #[test]
    fn test_fastboot_flashmode() {
        let mut usb = get_device().lock().unwrap();
        let mut vec = Vec::<u8>::new();

        assert!(fastboot_download(&mut usb, &mut vec, &[0u8]).is_ok());
        assert!(fastboot_cmd(&mut usb, &mut vec, b"Write-TA:2:10100").is_ok());
    }

    #[test]
    fn test_files() {
        let text = c"files/text".as_ptr();
        assert_eq!(file_exist(text), 1);
        assert_eq!(file_size(text), 26);
    }

    #[test]
    fn test_parseoct() {
        assert_eq!(parseoct(c"123".as_ptr(), 3), 0b001010011);
        assert_eq!(parseoct(c"1000".as_ptr(), 4), 0b1000000000);
        assert_eq!(parseoct(c"777".as_ptr(), 3), 0b111111111);

        assert_eq!(parseoct(c"a777z".as_ptr(), 5), 0b111111111);
    }

    fn reply_str<'a>(buffer: &'a mut Vec<u8>, usb: &mut UsbInterfaces, var: &[u8]) -> &'a str {
        if let Err(e) = fastboot_cmd(usb, buffer, var) {
            error!("{}", e);
            return "";
        };

        str::from_utf8(buffer).expect(&format!("Failed to parse {:?} as str", buffer.as_slice()))
    }

}