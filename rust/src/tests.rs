#[cfg(test)]
mod tests {
    use crate::utils::{file_exist, file_size};
    use crate::utils::{parseoct, trim_rs};
    use crate::*;
    use std::ffi::c_char;
    use std::fs::File;
    use std::io::Read;
    use std::sync::{Mutex, OnceLock};
    use tar::Archive;
    use crate::sins::{transfer_cms};

    unsafe extern "C" {
        pub fn is_end_of_archive(p: *const u8) -> i32;

        pub fn gunziper(in_: *const c_char, out: *const c_char) -> i32;
    }

    const VID: u16 = 0x0FCE;
    const PID: u16 = 0xB00B;

    static DEVICE: OnceLock<Mutex<FastbootDevice>> = OnceLock::new();

    fn get_device() -> &'static Mutex<FastbootDevice> {
        DEVICE.get_or_init(|| Mutex::new(get_flash_mode_rs(VID, PID)))
    }

    // https://android.googlesource.com/platform/system/core/+/master/fastboot/README.md
    #[test]
    fn test_fastboot_vars() {
        let mut usb = get_device().lock().unwrap();

        assert_eq!(reply_str(&mut usb, b"getvar:max-download-size").parse::<u32>().unwrap(), 805306368);
        assert_eq!(reply_str(&mut usb, b"getvar:product"), "XQ-EC72");
        assert_eq!(reply_str(&mut usb, b"getvar:version"), "0.4");
        assert_eq!(reply_str(&mut usb, b"getvar:version-bootloader"), "8650-0001_X_Boot_SM8650_LA1.0_U_36");
        assert_eq!(reply_str(&mut usb, b"getvar:serialno"), std::env::var("SERIAL_NO").unwrap().as_str());
        assert_eq!(reply_str(&mut usb, b"getvar:secure"), "no");
        assert_eq!(reply_str(&mut usb, b"getvar:Sector-size"), "4096");
        assert_eq!(reply_str(&mut usb, b"getvar:Loader-version"), "8650-0001_X_Boot_SM8650_LA1.0_U_36");
        assert_eq!(reply_str(&mut usb, b"getvar:Phone-id"), std::env::var("PHONE_ID").unwrap().as_str());
        assert_eq!(reply_str(&mut usb, b"getvar:Device-id"), std::env::var("DEVICE_ID").unwrap().as_str());
        assert_eq!(reply_str(&mut usb, b"getvar:Platform-id"), "202270E1");
        assert_eq!(reply_str(&mut usb, b"getvar:Rooting-status"), "ROOTED");
        assert_eq!(reply_str(&mut usb, b"getvar:Ufs-info"), "KIOXIA,THGJFLT1E45BATPB,0100");
        assert_eq!(reply_str(&mut usb, b"getvar:Emmc-info"), "Emmc-info not supported");
        assert_eq!(reply_str(&mut usb, b"getvar:Default-security"), "ON");
        assert_eq!(reply_str(&mut usb, b"getvar:Keystore-counter"), "2");
        assert_eq!(reply_str(&mut usb, b"getvar:Security-state"), std::env::var("SECURITY_STATE").unwrap().as_str());
        assert_eq!(reply_str(&mut usb, b"getvar:S1-root"), "S1_Root_398d");
        assert_eq!(reply_str(&mut usb, b"getvar:Sake-root"), "5515");

        usb.command(b"Get-root-key-hash").unwrap();
        assert_eq!(usb.reply.len(), 48);
        assert_eq!(usb.reply.iter().map(|b| format!("{:02X}", b)).collect::<String>(), std::env::var("ROOT_KEY_HASH").unwrap());

        assert_eq!(reply_str(&mut usb, b"getvar:slot-count"), "2");
        assert_eq!(reply_str(&mut usb, b"getvar:current-slot"), "a");
        assert_eq!(reply_str(&mut usb, b"getvar:Battery"), "Battery not supported");
    }

    #[test]
    fn test_fastboot_flashmode() {
        let mut usb = get_device().lock().unwrap();

        usb.download(&[0u8]).unwrap();
        usb.command(b"Write-TA:2:10100").unwrap();
    }

    #[test]
    fn test_sin_signature() {
        let mut usb = get_device().lock().unwrap();

        let file = File::open("files/partition-image-LUN0_124936192_X-FLASH-ALL-88DF.sin").unwrap();
        let mut archive = Archive::new(Box::new(file) as Box<dyn Read>);

        let mut fst = archive.entries().unwrap().nth(0).unwrap().unwrap();

        transfer_cms(&mut usb, &mut fst, "partitionimage_0").unwrap();
    }

    #[test]
    fn test_files() {
        let text = c"files/text".as_ptr();
        assert_eq!(file_exist(text), 1);
        assert_eq!(file_size(text), 26);
    }

    #[test]
    fn test_trim() {
        let str = "  this is\r a\n test\t string  .  ";
        assert_eq!(trim_rs(str), "thisisateststring.");

        let str1 = "\r\n\t why would you type\r\r\r\r \n\n\n\n \t\t\t\t like this";
        assert_eq!(trim_rs(str1), "whywouldyoutypelikethis");
    }

    #[test]
    fn test_parseoct() {
        assert_eq!(parseoct(c"123".as_ptr(), 3), 0b001010011);
        assert_eq!(parseoct(c"1000".as_ptr(), 4), 0b1000000000);
        assert_eq!(parseoct(c"777".as_ptr(), 3), 0b111111111);

        assert_eq!(parseoct(c"a777z".as_ptr(), 5), 0b111111111);
    }

    fn reply_str<'a>(usb: &'a mut FastbootDevice, var: &[u8]) -> &'a str {
        if let Err(e) = usb.command(var) {
            error!("{}", e);
            return "";
        };

        str::from_utf8(usb.reply.as_slice()).expect(&format!("Failed to parse {:?} as str", usb.reply.as_slice()))
    }
}