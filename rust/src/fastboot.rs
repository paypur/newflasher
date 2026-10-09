use crate::types::{ByteVec, Slot};
use crate::utils::trace_formatted_hex;
use anyhow::{Context, ensure};
use log::{debug, info, trace};
use nusb::io::{EndpointRead, EndpointWrite};
use nusb::transfer::{Bulk, In, Out};
use nusb::{Device, Interface, MaybeFuture};
use std::io::{Read, Write};
use std::mem;
use std::time::Duration;
use tar::Entry;

#[derive(PartialEq, Eq, Debug, derive_more::Display)]
pub enum FastbootHeader {
    Error = 0,
    Okay,
    Data,
    Fail,
    NoHeader,
}

impl From<&[u8]> for FastbootHeader {
    fn from(value: &[u8]) -> Self {
        match value {
            b"OKAY" => FastbootHeader::Okay,
            b"DATA" => FastbootHeader::Data,
            b"FAIL" => FastbootHeader::Fail,
            _ => FastbootHeader::NoHeader,
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
    pub writer: EndpointWrite<Bulk>,
    pub reply: ByteVec,
}

impl FastbootDevice {
    pub fn new() -> Self {
        let info = nusb::list_devices()
            .wait()
            .unwrap()
            .find(|d| d.vendor_id() == VID && d.product_id() == PID)
            .expect("Failed to find device at /dev/bus/usb! Device should be connected via flash mode (green *)");

        let _device: Device = info.open().wait().unwrap_or_else(|e| panic!("Failed to open device (VID: {VID}, PID: {PID})\n{e}"));
        let _interface: Interface = _device.claim_interface(0).wait().unwrap();

        let reader = _interface.endpoint::<Bulk, In>(IN).unwrap().reader(256).with_num_transfers(1).with_read_timeout(TEN_SEC);
        let writer = _interface.endpoint::<Bulk, Out>(OUT).unwrap().writer(0x20000).with_num_transfers(4).with_write_timeout(TEN_SEC);
        let vec = Vec::<u8>::with_capacity(256);

        Self { _device, _interface, reader, writer, reply: vec.into() }
    }

    /* utility commands */
    pub fn print_firmware_history(&mut self) -> anyhow::Result<()> {
        self.command_expect("Read-TA:2:2475", FastbootHeader::Data).context("Failed to read firmware history")?;

        let len = self.reply.as_hexadecimal()?;
        self.read_reply().expect("Failed to read reply");

        ensure!(self.reply.len() == len as usize);

        println!("Firmware History ───────────────────────────────────────────────────────────────────────────────────\n{}", self.reply);

        ensure!(self.read_reply()? == FastbootHeader::Okay);

        Ok(())
    }

    pub fn reboot_system(&mut self) -> anyhow::Result<()> {
        self.command_expect("continue", FastbootHeader::Okay)?;
        println!("Rebooting to system");
        Ok(())
    }

    pub fn enter_flash_mode(&mut self) -> anyhow::Result<()> {
        self.download(&[1u8])?;
        self.command_expect("Write-TA:2:10100", FastbootHeader::Okay)?;
        info!("Entered flashmode");
        Ok(())
    }

    pub fn exit_flash_mode(&mut self) -> anyhow::Result<()> {
        self.download(&[0u8])?;
        self.command_expect("Write-TA:2:10100", FastbootHeader::Okay)?;
        info!("Exited flashmode");
        Ok(())
    }

    pub fn set_active_slot(&mut self, slot: Slot) -> anyhow::Result<()> {
        let cmd = format!("set_active:{slot}");
        self.command_expect(cmd.as_str(), FastbootHeader::Okay)
    }

    pub fn command(&mut self, cmd: &str) -> anyhow::Result<FastbootHeader> {
        self.write_and_read_reply(cmd).with_context(|| format!("Fastboot command {cmd} failed!"))
    }

    pub fn command_expect(&mut self, cmd: &str, expected: FastbootHeader) -> anyhow::Result<()> {
        self.write_and_expect_reply(cmd, expected).with_context(|| format!("Fastboot command {cmd} failed!"))
    }

    /// For commands that require multiple reads to receive the actual data
    /// used by, Get-root-key-hash, Get-ufs-info, Read-TA:2:2475, and more
    pub fn get_data(&mut self, cmd: &str) -> anyhow::Result<()> {
        let mut body = || -> anyhow::Result<()> {
            self.write(cmd.as_bytes())?;

            let header = self.read_reply()?;
            ensure!(header == FastbootHeader::Data, format!("Expected DATA header, received {header:?}!"));

            let len = self.reply.as_hexadecimal()?;

            // second read for actual data
            self.read()?;
            ensure!(len as usize == self.reply.len(), format!("Expected {len} bytes, received {:?}!", self.reply.len()));
            if FastbootHeader::from(self.reply.as_slice()) == FastbootHeader::Okay {
                return Ok(());
            }

            // if not OKAY in second read, should read a 3rd time to acknowledge
            let mut temp = vec![0u8; 4];
            mem::swap::<Vec<u8>>(self.reply.as_mut(), temp.as_mut()); // TODO: this is kinda bad
            let header = self.read_reply()?;
            ensure!(header == FastbootHeader::Okay, format!("Expected OKAY header, received {header:?}!"));
            mem::swap::<Vec<u8>>(self.reply.as_mut(), temp.as_mut());

            Ok(())
        };
        body().with_context(|| format!("Fastboot command {cmd} failed!"))
    }

    pub fn getvar_u32(&mut self, cmd: &str) -> anyhow::Result<u32> {
        self.getvar(cmd)?;
        self.reply.as_decimal()
    }

    pub fn getvar_string(&mut self, cmd: &str) -> anyhow::Result<String> {
        self.getvar(cmd)?;
        String::from_utf8(self.reply.clone()).with_context(|| format!("Failed to parse {} as str", self.reply))
    }

    fn getvar(&mut self, cmd: &str) -> anyhow::Result<FastbootHeader> {
        let var = format!("getvar:{cmd}");
        self.command(&var)
    }

    /// Tries to write a buffer to device
    pub fn download(&mut self, data: impl AsRef<[u8]>) -> anyhow::Result<()> {
        let hex_len = ByteVec::from_len(data.as_ref().len());
        let cmd = format!("download:{hex_len}");

        self.write_and_expect_reply(cmd, FastbootHeader::Data)?;
        ensure!(hex_len == self.reply, format!("Expected {hex_len}, received {}!", self.reply));

        self.write_and_expect_reply(data, FastbootHeader::Okay).context("Failed to download data to device!")?;

        Ok(())
    }

    /// Tries to write tar entry to device
    pub fn download_tar_entry(&mut self, entry: &mut Entry<Box<dyn Read>>) -> anyhow::Result<()> {
        const MiB_2: u64 = 0x200000;

        let hex_len = ByteVec::from_len(entry.size() as usize);
        let cmd = format!("download:{hex_len}");

        debug!("{cmd}");
        self.write_and_expect_reply(cmd, FastbootHeader::Data)?;
        ensure!(hex_len == self.reply, format!("Expected {hex_len}, received {}!", self.reply));

        let mut buffer = Vec::<u8>::new();
        let mut adapter = entry.take(MiB_2);

        loop {
            buffer.clear();
            adapter.read_to_end(&mut buffer)?;

            if buffer.is_empty() {
                break;
            }

            self.writer.write_all(&buffer).context("Failed to download tar entry to device")?;
            trace!("WRITE[0x{:X}]:skip", buffer.len());

            adapter.set_limit(MiB_2);
        }

        self.writer.flush()?;

        let header = self.read_reply()?;
        ensure!(header == FastbootHeader::Okay, format!("Expected OKAY header, received {header:?}!"));

        Ok(())
    }

    pub fn write_and_read_reply(&mut self, cmd: impl AsRef<[u8]>) -> anyhow::Result<FastbootHeader> {
        self.write(cmd)?;
        self.read_reply()
    }

    pub fn write_and_expect_reply(&mut self, cmd: impl AsRef<[u8]>, expected: FastbootHeader) -> anyhow::Result<()> {
        let received = self.write_and_read_reply(cmd)?;
        ensure!(received == expected, format!("Expected {expected} header, received {received}: {}", self.reply));
        Ok(())
    }

    pub fn write(&mut self, data: impl AsRef<[u8]>) -> anyhow::Result<usize> {
        let bytes = data.as_ref();
        trace_formatted_hex("WRITE", bytes);
        self.writer.write_all(bytes)?;
        self.writer.flush_end()?;
        Ok(bytes.len())
    }

    /// Strips reply header from self.reply
    pub fn read_reply(&mut self) -> anyhow::Result<FastbootHeader> {
        let len = self.read()?;
        trace_formatted_hex("READ", self.reply.as_slice());
        if len < 4 { return Ok(FastbootHeader::NoHeader); }

        let prefix = FastbootHeader::from(&self.reply[0..4]);
        match prefix {
            FastbootHeader::Okay | FastbootHeader::Fail if len == 4 => {
                self.reply.clear();
            },
            FastbootHeader::Okay | FastbootHeader::Fail if len > 4 => {
                // strip the prefix and copy the rest
                self.reply.drain(0..4);
            }
            FastbootHeader::Data if len == 12 || len == 13 => {
                // xperia 10 mark 3 XQ-BT41 send 13 bytes where last byte is null termination
                self.reply.truncate(12);
                self.reply.drain(0..4);
            },
            _ => ()
        };

        Ok(prefix)
    }

    /// Reads data from device into self.reply
    pub fn read(&mut self) -> anyhow::Result<usize> {
        self.reply.clear();
        let mut short_reader = self.reader.until_short_packet();
        let size = short_reader.read_to_end(self.reply.as_mut())?;
        short_reader.consume_end()?;
        Ok(size)
    }
}