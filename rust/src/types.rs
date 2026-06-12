use crate::u32_from_bytes;
use anyhow::{ensure, Context, Result};
use derive_more::{AsRef, Deref, DerefMut};
use log::error;
use nusb::io::{EndpointRead, EndpointWrite};
use nusb::transfer::{Bulk, In, Out};
use nusb::{Device, Interface};
use std::fmt::{Display, Formatter};
use std::io::{Read, Write};
use std::time::Duration;
use std::{mem};
use tar::Entry;

const IN: u8 = 0x81;
const OUT: u8 = 0x01;

#[derive(AsRef, Debug, Deref, DerefMut, PartialEq)]
#[repr(C)]
pub struct ByteVec {
    data: Vec<u8>
}

impl ByteVec {
    pub fn from_len(len: usize) -> Self {
        ByteVec::from(format!("{:08x}", len))
    }

    pub fn as_hexadecimal(&self) -> Result<u32> {
        let str = str::from_utf8(&self.data)?;
        let n = u32::from_str_radix(str, 16)?;
        Ok(n)
    }
}

impl From<Vec<u8>> for ByteVec {
    fn from(data: Vec<u8>) -> ByteVec {
        ByteVec { data }
    }
}

impl From<String> for ByteVec {
    fn from(value: String) -> Self {
        ByteVec::from(value.into_bytes())
    }
}

impl Display for ByteVec {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        unsafe {
            write!(f, "{}", &self.data.iter().map(|&b| char::from_u32_unchecked(b as u32)).collect::<String>())
        }
    }
}

impl<T> PartialEq<T> for ByteVec
    where
        T: AsRef<[u8]>
{
    fn eq(&self, other: &T) -> bool {
        self == other
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
pub struct FastbootDevice {
    device: Device,
    interface: Interface,
    pub reader: EndpointRead<Bulk>,
    pub writer: EndpointWrite<Bulk>,
    pub reply: ByteVec,
}

const BUFFER_SIZE: usize = 1024;
const NUM_TRANSFERS: usize = 4;
const TEN_SEC: Duration = Duration::from_secs(10);

impl FastbootDevice {
    pub fn new(device: Device, interface: Interface) -> Self {
        let reader = interface.endpoint::<Bulk, In>(IN).unwrap().reader(BUFFER_SIZE).with_num_transfers(NUM_TRANSFERS).with_read_timeout(TEN_SEC);
        let writer = interface.endpoint::<Bulk, Out>(OUT).unwrap().writer(BUFFER_SIZE).with_num_transfers(NUM_TRANSFERS).with_write_timeout(TEN_SEC);
        let vec = Vec::<u8>::with_capacity(32);
        Self { device, interface, reader, writer, reply: ByteVec::from(vec) }
    }

    pub fn command(&mut self, cmd: &[u8]) -> Result<()> {
        self.write_and_read_reply(cmd).with_context(|| format!("Fastboot command {} failed!", String::from_utf8_lossy(cmd)))?;
        Ok(())
    }

    pub fn command_expect(&mut self, cmd: &[u8], expected: FastbootHeader) -> Result<()> {
        self.write_and_expect_reply(cmd, expected).with_context(|| format!("Fastboot command {} failed!", String::from_utf8_lossy(cmd)))
    }

    /// For commands that require multiple reads to receive the actual data
    pub fn get_data(&mut self, cmd: &[u8]) -> Result<()> {
        let mut body = || -> Result<()> {
            self.write(cmd)?;

            let header = self.read_reply()?;
            ensure!(header == FastbootHeader::Data, format!("Expected DATA header, received {header:?}!"));

            let len = self.reply.as_hexadecimal()?;

            // second read for actual data
            self.read()?;
            if FastbootHeader::from(self.reply.as_slice()) == FastbootHeader::Okay { todo!("unimplemented OKAY after DATA") }
            ensure!(len as usize == self.reply.len(), format!("Expected {len} bytes, received {:?}!", self.reply.len()));

            // if not OKAY in second read, should read a 3rd time to acknowledge
            let mut temp = vec![0u8; 4];
            mem::swap::<Vec<u8>>(self.reply.as_mut(), temp.as_mut()); // TODO: this is kinda bad
            let header = self.read_reply()?;
            ensure!(header == FastbootHeader::Okay, format!("Expected OKAY header, received {header:?}!"));
            mem::swap::<Vec<u8>>(self.reply.as_mut(), temp.as_mut());

            Ok(())
        };
        body().with_context(|| format!("Fastboot command {} failed!", String::from_utf8_lossy(cmd)))
    }

    pub fn getvar_u32(&mut self, var: &[u8], default: u32) -> u32 {
        match self.write_and_read_reply(var) {
            Ok(header) => if header == FastbootHeader::Okay {
                return u32_from_bytes(self.reply.as_ref())
            }
            Err(e) => {
                error!("Failed to execute command: {}! {e}", str::from_utf8(var).unwrap());
            }
        }
        default
    }

    /// Tries to write a buffer to device
    pub fn download(&mut self, data: &[u8]) -> Result<()> {
        let hex_len = ByteVec::from_len(data.len());
        let cmd = [b"download:", hex_len.as_slice()].concat();

        self.write_and_expect_reply(&cmd, FastbootHeader::Data)?;
        ensure!(hex_len == self.reply, format!("Expected {hex_len}, received {}!", self.reply));

        self.write_and_expect_reply(data, FastbootHeader::Okay).with_context(|| format!("Failed to write to device:\n {:?}", data))?;

        Ok(())
    }

    /// Tries to write tar entry to device
    /// data is chunked in 16KiB parts
    pub fn download_tar_entry(&mut self, entry: &mut Entry<Box<dyn Read>>) -> Result<()> {
        let hex_len = ByteVec::from_len(entry.size() as usize);
        let cmd = [b"download:", hex_len.as_slice()].concat();

        self.write_and_expect_reply(&cmd, FastbootHeader::Data)?;
        ensure!(hex_len == self.reply, format!("Expected {hex_len}, received {}!", self.reply));

        entry.take(16384);
        std::io::copy(entry, &mut self.writer).with_context(|| format!("Failed to write tar entry to device: {:?}", entry.header()))?;
        self.writer.flush_end().with_context(|| format!("Failed to flush tar entry: {:?}", entry.header()))?;

        let header = self.read_reply()?;
        ensure!(header == FastbootHeader::Okay, format!("Expected OKAY header, received {header:?}!"));

        Ok(())
    }

    pub fn write_and_read_reply(&mut self, cmd: &[u8]) -> Result<FastbootHeader> {
        self.write(cmd)?;
        self.read_reply()
    }

    pub fn write_and_expect_reply(&mut self, cmd: &[u8], expected: FastbootHeader) -> Result<()> {
        let received= self.write_and_read_reply(cmd)?;
        ensure!(received == expected, format!("Expected {expected} header, received {received}!"));
        Ok(())
    }

    pub fn write(&mut self, data: &[u8]) -> Result<usize> {
        self.writer.write_all(data)?;
        self.writer.flush_end()?;
        Ok(data.len())
    }

    /// Strips reply header from self.reply
    pub fn read_reply(&mut self) -> Result<FastbootHeader> {
        let len = self.read()?;
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