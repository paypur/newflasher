use anyhow::{ensure, Context, Result};
use derive_more::{AsRef, Deref, DerefMut};
use log::error;
use nusb::io::{EndpointRead, EndpointWrite};
use nusb::transfer::{Bulk, In, Out};
use nusb::{Device, Interface};
use std::fmt::{Debug, Display, Formatter};
use std::io::{Read, Write};
use std::time::Duration;
use std::{mem, ptr};
use tar::Entry;

use crate::utils::{print_hex_ascii, u8_ascii};

const IN: u8 = 0x81;
const OUT: u8 = 0x01;

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Slot {
    A,
    B
}

impl Slot {
    pub fn other(&self) -> Self {
        match self {
            Slot::A => Slot::B,
            Slot::B => Slot::A
        }
    }

    pub fn as_str(&self) -> &'static str {
        match self {
            Slot::A => "a",
            Slot::B => "b"
        }
    }
}

impl From<&str> for Slot {
    fn from(value: &str) -> Self {
        match value {
            "a" | "A" => Slot::A,
            "b" | "B" => Slot::B,
            _ => panic!("Invalid slot: {}", value)
        }
    }
}

impl Display for Slot {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

#[derive(AsRef, Deref, DerefMut, Default, PartialEq)]
#[repr(C)]
pub struct ByteVec {
    data: Vec<u8>
}

impl ByteVec {
    pub fn new() -> Self {
        Self { data: Vec::new() }
    }

    pub fn append(&mut self, vec: &Self) {
        self.data.extend(vec.as_ref());
    }

    pub fn extend_vec(&mut self, vec: Self) {
        self.data.extend(vec.as_ref());
    }

    pub fn from_len(len: usize) -> Self {
        ByteVec::from(format!("{:08x}", len))
    }

    pub fn as_decimal(&self) -> Result<u32> {
        let str = str::from_utf8(&self.data)?;
        let n = u32::from_str_radix(str, 10)?;
        Ok(n)
    }

    pub fn as_hexadecimal(&self) -> Result<u32> {
        let str = str::from_utf8(&self.data)?;
        let n = u32::from_str_radix(str, 16).with_context(|| format!("String: \"{str}\""))?;
        Ok(n)
    }
}

impl From<Vec<u8>> for ByteVec {
    fn from(data: Vec<u8>) -> Self {
        Self { data }
    }
}

impl<const N: usize> From<[u8; N]> for ByteVec {
    fn from(value: [u8; N]) -> Self {
        Vec::from(value).into()
    }
}

impl From<&[u8]> for ByteVec {
    fn from(value: &[u8]) -> Self {
        Vec::from(value).into()
    }
}

impl From<&str> for ByteVec {
    fn from(value: &str) -> Self {
        Vec::from(value).into()
    }
}

impl From<String> for ByteVec {
    fn from(value: String) -> Self {
        value.into_bytes().into()
    }
}

impl From<CVec> for ByteVec {
    fn from(value: CVec) -> Self {
        unsafe {
            Vec::from_raw_parts(value.ptr, value.len, value.cap).into()
        }
    }
}

impl FromIterator<u8> for ByteVec {
    fn from_iter<T: IntoIterator<Item = u8>>(iter: T) -> Self {
        iter.into_iter().collect::<Vec<u8>>().into()
    }
}

impl Debug for ByteVec {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", &self.data.iter().map(|&b| b as char).collect::<String>() )
    }
}

impl Display for ByteVec {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", &self.data.iter().map(|&b| b as char).collect::<String>() )
    }
}

impl<T> PartialEq<T> for ByteVec
    where
        T: AsRef<[u8]>,
{
    fn eq(&self, other: &T) -> bool {
        self.data == other.as_ref()
    }
}

#[repr(C)]
pub struct CVec {
    ptr: *mut u8,
    len: usize,
    cap: usize,
}

impl From<ByteVec> for CVec {
    fn from(vec: ByteVec) -> Self {
        let (ptr, len, cap) = vec.data.into_raw_parts();
        Self { ptr, len, cap }
    }
}

#[repr(C)]
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

#[repr(C)]
pub struct FastbootDeviceFFI {
    _device: Device,
    _interface: Interface,
    _reader: EndpointRead<Bulk>,
    _writer: EndpointWrite<Bulk>,
    pub reply: CVec,
}

impl From<FastbootDevice> for FastbootDeviceFFI {
    fn from(dev: FastbootDevice) -> Self {
        let cvec = CVec::from(dev.reply);
        Self { _device: dev.device, _interface: dev.interface, _reader: dev.reader, _writer: dev.writer, reply: cvec }
    }
}

pub struct FastbootDevice {
    device: Device,
    interface: Interface,
    pub reader: EndpointRead<Bulk>,
    pub writer: EndpointWrite<Bulk>,
    pub reply: ByteVec,
}

const BUFFER_SIZE: usize = 1024;
const NUM_TRANSFERS: usize = 16;
const TEN_SEC: Duration = Duration::from_secs(10);

impl FastbootDevice {
    pub fn new(device: Device, interface: Interface) -> Self {
        let reader = interface.endpoint::<Bulk, In>(IN).unwrap().reader(BUFFER_SIZE).with_num_transfers(NUM_TRANSFERS).with_read_timeout(TEN_SEC);
        let writer = interface.endpoint::<Bulk, Out>(OUT).unwrap().writer(BUFFER_SIZE).with_num_transfers(NUM_TRANSFERS).with_write_timeout(TEN_SEC);
        let vec = Vec::<u8>::with_capacity(256);
        Self { device, interface, reader, writer, reply: vec.into() }
    }

    pub fn command(&mut self, cmd: &str) -> Result<()> {
        self.write_and_read_reply(cmd.as_bytes()).with_context(|| format!("Fastboot command {cmd} failed!"))?;
        Ok(())
    }

    pub fn command_expect(&mut self, cmd: &str, expected: FastbootHeader) -> Result<()> {
        self.write_and_expect_reply(cmd.as_bytes(), expected).with_context(|| format!("Fastboot command {cmd} failed!"))
    }

    /// For commands that require multiple reads to receive the actual data
    /// used by, Get-root-key-hash, Get-ufs-info, Read-TA:2:2475, and more
    pub fn get_data(&mut self, cmd: &str) -> Result<()> {
        let mut body = || -> Result<()> {
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

    pub fn getvar_u32(&mut self, cmd: &str, default: u32) -> u32 {
        match self.write_and_read_reply(cmd.as_bytes()).with_context(|| format!("Fastboot command {cmd} failed!")) {
            Ok(header) => if header == FastbootHeader::Okay {
                return self.reply.as_decimal().unwrap_or_else(|e| {
                    error!("{}", e);
                    default
                });
            },
            Err(e) => error!("{}", e)
        }
        default
    }

    /// Tries to write a buffer to device
    pub fn download(&mut self, data: &[u8]) -> Result<()> {
        let hex_len = ByteVec::from_len(data.len());
        let cmd = [b"download:", hex_len.as_slice()].concat();

        self.write_and_expect_reply(&cmd, FastbootHeader::Data)?;
        ensure!(hex_len == self.reply, format!("Expected {hex_len}, received {}!", self.reply));

        self.write_and_expect_reply(data, FastbootHeader::Okay).with_context(|| format!("Failed to download data to device:\n {:?}", data))?;

        Ok(())
    }

    /// Tries to write tar entry to device
    pub fn download_tar_entry(&mut self, entry: &mut Entry<Box<dyn Read>>) -> Result<()> {
        let hex_len = ByteVec::from_len(entry.size() as usize);
        let mut cmd = ByteVec::from("download:");
        cmd.append(&hex_len);

        println!("    {}", cmd);
        self.write_and_expect_reply(&cmd, FastbootHeader::Data)?;
        ensure!(hex_len == self.reply, format!("Expected {hex_len}, received {}!", self.reply));

        // TODO: fix this for real
        if entry.size() >= 0x10 && entry.size() < 0x200000 {
            // copy doesn't need to be flushed
            std::io::copy(entry, &mut self.writer).with_context(|| format!("Failed to download tar entry to device: {:?}", entry.header()))?;
            println!("WRITE:\nskipped {} bytes", entry.size());
        } else {
            let mut buffer = Vec::<u8>::new();
            let mut adapter = entry.take(0x200000); // 2MiB

            loop {
                buffer.clear();
                adapter.read_to_end(&mut buffer)?;

                if buffer.is_empty() {
                    break;
                }

                self.writer.write_all(&buffer).with_context(|| "Failed to download tar entry to device".to_string())?;

                adapter.set_limit(0x200000);
            }
        }

        self.writer.flush()?;

        let header = self.read_reply()?;
        ensure!(header == FastbootHeader::Okay, format!("Expected OKAY header, received {header:?}!"));
        println!("    OKAY");

        Ok(())
    }

    pub fn write_and_read_reply(&mut self, cmd: &[u8]) -> Result<FastbootHeader> {
        self.write(cmd)?;
        self.read_reply()
    }

    pub fn write_and_expect_reply(&mut self, cmd: &[u8], expected: FastbootHeader) -> Result<()> {
        let received = self.write_and_read_reply(cmd)?;
        ensure!(received == expected, format!("Expected {expected} header, received {received}: {}", self.reply));
        Ok(())
    }

    pub fn write(&mut self, data: &[u8]) -> Result<usize> {
        print_hex_ascii("WRITE", data);
        self.writer.write_all(data)?;
        self.writer.flush_end()?;
        Ok(data.len())
    }

    pub fn getvar_string(&mut self, cmd: &str) -> Result<String> {
        self.command(cmd)?;
        Ok(String::from_utf8(self.reply.as_ref().clone()).expect(&format!("Failed to parse {} as str", self.reply)))
    }

    /// Strips reply header from self.reply
    pub fn read_reply(&mut self) -> Result<FastbootHeader> {
        let len = self.read()?;
        print_hex_ascii("READ", self.reply.as_slice());
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
    pub fn read(&mut self) -> Result<usize> {
        self.reply.clear();
        let mut short_reader = self.reader.until_short_packet();
        let size = short_reader.read_to_end(self.reply.as_mut())?;
        short_reader.consume_end()?;
        Ok(size)
    }
}

impl From<*mut FastbootDeviceFFI> for FastbootDevice {
    fn from(dev_ffi_ptr: *mut FastbootDeviceFFI) -> Self {
        unsafe {
            let dev_ffi = ptr::read(dev_ffi_ptr);
            Self { device: dev_ffi._device, interface: dev_ffi._interface, reader: dev_ffi._reader, writer: dev_ffi._writer, reply: dev_ffi.reply.into() }
        }
    }
}