use crate::types::{ByteVec, Slot};
use crate::utils::trace_formatted_hex;
use anyhow::{Context, ensure};
use log::{debug, info};
use nusb::io::{EndpointRead, EndpointWrite};
use nusb::transfer::{Bulk, In, Out};
use nusb::{Device, Interface, MaybeFuture};
use std::io::{ErrorKind, Read, Write};
use std::time::Duration;

pub struct Response {
    pub header: Header,
    pub data: ByteVec,
}

impl Response {
    fn new(header: Header, data: ByteVec) -> Response {
        Response { header, data }
    }

    pub fn expect_header(self, expected: Header) -> anyhow::Result<ByteVec> {
        ensure!(expected == self.header, format!("Expected {expected} header, received {}: {}", self.header, self.data));
        Ok(self.data)
    }
}


#[derive(PartialEq, Eq, Debug, derive_more::Display)]
pub enum Header {
    Error = 0,
    Okay,
    Data,
    Fail,
    NoHeader,
}

impl From<&[u8]> for Header {
    fn from(value: &[u8]) -> Self {
        match value {
            b"OKAY" => Header::Okay,
            b"DATA" => Header::Data,
            b"FAIL" => Header::Fail,
            _ => Header::NoHeader,
        }
    }
}


const VID: u16 = 0x0FCE;
const PID: u16 = 0xB00B;

const IN: u8 = 0x81;
const OUT: u8 = 0x01;

const TEN_SEC: Duration = Duration::from_secs(10);

pub struct FastbootDevice {
    _device: Device,
    _interface: Interface,
    pub reader: EndpointRead<Bulk>,
    pub writer: EndpointWrite<Bulk>
}

impl FastbootDevice {
    pub fn open() -> Self {
        let info = nusb::list_devices()
            .wait()
            .unwrap()
            .find(|d| d.vendor_id() == VID && d.product_id() == PID)
            .expect("Failed to find device! Device should be connected via flash mode (green *)");

        info!("USB device connected at {:?} speed", info.speed());

        let _device: Device = info.open().wait().unwrap_or_else(|e| panic!("Failed to open device (VID: {VID}, PID: {PID})\n{e}"));
        let _interface: Interface = _device.claim_interface(0).wait().unwrap();

        let reader = _interface.endpoint::<Bulk, In>(IN).unwrap().reader(256).with_num_transfers(1).with_read_timeout(TEN_SEC);
        let writer = _interface.endpoint::<Bulk, Out>(OUT).unwrap().writer(0x20000).with_num_transfers(4).with_write_timeout(TEN_SEC);

        Self { _device, _interface, reader, writer }
    }

    /* utility commands */
    pub fn print_firmware_history(&mut self) -> anyhow::Result<()> {
        let data = self.command_expect_header("Read-TA:2:2475", Header::Data).context("Failed to read firmware history")?;
        let len = data.as_hexadecimal()?;

        let history = self.read_and_parse()?;

        ensure!(history.data.len() == len as usize);

        println!("{:─<97}", "Firmware History");
        println!("{}", history.data);

        ensure!(self.read_and_parse()?.header == Header::Okay);

        Ok(())
    }

    pub fn reboot_system(&mut self) {
        self.command_expect_header("continue", Header::Okay).expect("Failed to reboot to system!");
        println!("Rebooting to system");
    }

    pub fn enter_flash_mode(&mut self) {
        self.download(&[1u8]).unwrap();
        self.command_expect_header("Write-TA:2:10100", Header::Okay).expect("Failed to enter flash mode");
        info!("Entered flash mode");
    }

    pub fn exit_flash_mode(&mut self) {
        self.download(&[0u8]).unwrap();
        self.command_expect_header("Write-TA:2:10100", Header::Okay).expect("Failed to exit flash mode");
        info!("Exited flash mode");
    }

    pub fn sync(&mut self) {
        self.command_expect_header("Sync", Header::Okay).unwrap();
    }

    pub fn set_active_slot(&mut self, slot: Slot) {
        let cmd = format!("set_active:{slot}");
        self.command_expect_header(cmd, Header::Okay).expect("Failed to set active slot");
        println!("Set active slot to {slot}");
    }

    /// For commands that require multiple reads to receive the actual data
    /// used by, Get-root-key-hash, Get-ufs-info, Read-TA:2:2475, and more
    pub fn get_data(&mut self, cmd: &str) -> anyhow::Result<ByteVec> {
        // outside of body, to avoid duplicate error message
        let data = self.command_expect_header(cmd, Header::Data)?;

        let mut body = || -> anyhow::Result<ByteVec> {
            let len = data.as_hexadecimal()?;

            // second read for actual data
            let bytes = self.read()?;
            ensure!(len as usize == bytes.len(), "Expected {len} bytes, received {}!", bytes.len());
            if Header::from(bytes.as_slice()) == Header::Okay {
                return Ok(bytes.into());
            }

            // if not OKAY in second read, should read a 3rd time to acknowledge
            let res = self.read_and_parse()?;
            let _ = res.expect_header(Header::Okay)?;

            Ok(bytes.into())
        };
        body().with_context(|| format!("Fastboot command {cmd} failed!"))
    }

    pub fn getvar_u32(&mut self, cmd: &str) -> anyhow::Result<u32> {
        self.getvar(cmd)?.as_decimal()
    }

    pub fn getvar_string(&mut self, cmd: &str) -> anyhow::Result<String> {
        self.getvar(cmd)?.into_string()
    }

    fn getvar(&mut self, cmd: &str) -> anyhow::Result<ByteVec> {
        self.command_expect_header(format!("getvar:{cmd}"), Header::Okay)
    }

    pub fn command(&mut self, cmd: impl AsRef<str>) -> anyhow::Result<Response> {
        let cmd = cmd.as_ref();
        debug!("{cmd}");
        self.write_and_read_reply(cmd).with_context(|| format!("Fastboot command {cmd} failed!"))
    }

    pub fn command_expect(&mut self, cmd: impl AsRef<str>, header: Header, expected_data: ByteVec) -> anyhow::Result<ByteVec> {
        let res = self.command(cmd)?;
        let data = res.expect_header(header)?;
        ensure!(expected_data == data, "Expected {expected_data}, received {data}!");
        Ok(data)
    }

    pub fn command_expect_header(&mut self, cmd: impl AsRef<str>, expected: Header) -> anyhow::Result<ByteVec> {
        let res = self.command(cmd)?;
        res.expect_header(expected)
    }

    /// Attempts to write a buffer to device
    pub fn download(&mut self, data: impl AsRef<[u8]>) -> anyhow::Result<()> {
        let len = data.as_ref().len();
        let hex_len = ByteVec::from_len(len as u32);
        let cmd = format!("download:{hex_len}");

        let _ = self.command_expect(cmd, Header::Data, hex_len)?;
        self.write_and_expect_header(data, Header::Okay).with_context(|| format!("Failed to download {} bytes to device!", len))?;

        Ok(())
    }

    pub fn write_and_expect_header(&mut self, cmd: impl AsRef<[u8]>, expected: Header) -> anyhow::Result<ByteVec> {
        self.write_and_read_reply(cmd)?.expect_header(expected)
    }

    pub fn write_and_read_reply(&mut self, cmd: impl AsRef<[u8]>) -> anyhow::Result<Response> {
        self.write(cmd)?;
        self.read_and_parse()
    }

    fn write(&mut self, data: impl AsRef<[u8]>) -> anyhow::Result<usize> {
        let bytes = data.as_ref();
        trace_formatted_hex("WRITE", bytes);

        self.writer.write_all(bytes)?;

        if let Err(e) = self.writer.flush() {
            match e.kind() {
                ErrorKind::ConnectionAborted => panic!("{}", e),
                // can be caused by writing commands consecutively without reading in between
                ErrorKind::TimedOut => {
                    eprintln!("Flush timed out.");

                    // same as read but don't care about data
                    let mut short_reader = self.reader.until_short_packet();
                    std::io::copy(&mut short_reader, &mut std::io::sink())?;
                    short_reader.consume_end()?;
                },
                _ => return Err(e.into())
            }
        };

        Ok(bytes.len())
    }

    /// Read and parse fastboot header from response
    pub fn read_and_parse(&mut self) -> anyhow::Result<Response> {
        let mut vec = self.read()?;
        trace_formatted_hex("READ", vec.as_slice());

        if vec.len() < 4 { return Ok(Response::new(Header::NoHeader, vec)) }

        let prefix = Header::from(&vec[0..4]);
        match prefix {
            Header::Okay | Header::Fail if vec.len() == 4 => {
                vec.clear();
            },
            Header::Okay | Header::Fail if vec.len() > 4 => {
                // strip the prefix and copy the rest
                vec.drain(0..4);
            }
            Header::Data if vec.len() == 12 || vec.len() == 13 => {
                // xperia 10 mark 3 XQ-BT41 send 13 bytes where last byte is null termination
                vec.truncate(12);
                vec.drain(0..4);
            },
            _ => ()
        };

        Ok(Response::new(prefix, vec))
    }

    /// Reads data from device into self.reply
    pub fn read(&mut self) -> anyhow::Result<ByteVec> {
        let mut vec = ByteVec::new();

        let mut short_reader = self.reader.until_short_packet();
        short_reader.read_to_end(vec.as_mut())?;
        short_reader.consume_end()?;

        Ok(vec)
    }
}