use std::error::Error;
use std::fmt::{Display, Formatter};
use std::{io, mem};
use std::io::{Read, Write};
use nusb::io::{EndpointRead, EndpointWrite};
use nusb::transfer::{Bulk, In, Out};
use nusb::{Device, Interface};
use std::time::Duration;
use derive_more::{AsRef, Deref, DerefMut};
use log::error;
use tar::Entry;
use crate::u32_from_bytes;

const IN: u8 = 0x81;
const OUT: u8 = 0x01;

#[derive(AsRef, Debug, Deref, DerefMut, PartialEq)]
#[repr(C)]
pub struct ByteVec {
    data: Vec<u8>
}

impl ByteVec {
    fn from_len(len: usize) -> Self {
        ByteVec::from(format!("{:08X}", len))
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
where T: AsRef<[u8]> {
    fn eq(&self, other: &T) -> bool {
        self == other
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

    pub fn command(&mut self, cmd: &[u8]) -> Result<(), Box<dyn Error>> {
        if cmd.starts_with(b"getvar:") {
            // normal if it fail
            self.output_input_bulk(cmd)?;
        }
        else if (cmd.starts_with(b"Get-")) {
            self.output_bulk(cmd)?;

            let header = self.get_reply()?;
            if header != FastbootHeader::Data || self.reply.len() != 8 {
                return Err(io::Error::new(io::ErrorKind::InvalidData, format!("Expected DATA header, received {:?}", header)).into());
            }

            let len = usize::from_str_radix(str::from_utf8(&self.reply).expect("Failed to parse self.reply as str"), 16).expect("Failed to parse str as hexadecimal");

            // second read for actual data
            self.input_bulk()?;
            if FastbootHeader::from(self.reply.as_slice()) == FastbootHeader::Okay { todo!("unimplemented OKAY after DATA") }
            assert_eq!(len, self.reply.len());

            // if not OKAY in 2nd read, should read a 3rd time to acknowledge
            let mut temp = vec![0u8; 4];
            mem::swap::<Vec<u8>>(self.reply.as_mut(), temp.as_mut()); // TODO: this is kinda bad
            assert_eq!(self.get_reply()?, FastbootHeader::Okay);
            mem::swap::<Vec<u8>>(self.reply.as_mut(), temp.as_mut());
        }
        else if cmd.eq(b"Write-TA:2:10100") {
            let header = self.output_input_bulk(cmd)?;
            if header != FastbootHeader::Okay {
                return Err(io::Error::new(io::ErrorKind::InvalidData, format!("Expected OKAY header, received {:?}", header)).into());
            }
        }
        else {
            panic!("Fastboot command prefix '{}' not found", str::from_utf8(cmd).unwrap());
        }

        Ok(())
    }

    /// Tries to write a buffer to device
    pub fn download(&mut self, data: &[u8]) -> Result<(), Box<dyn Error>> {
        let mut full_cmd = b"download:".to_vec();
        let hex_len = ByteVec::from_len(data.len());
        full_cmd.extend_from_slice(hex_len.as_slice());

        let header = self.output_input_bulk(&mut full_cmd)?;
        if header != FastbootHeader::Data {
            return Err(io::Error::new(io::ErrorKind::InvalidData, format!("Expected DATA header, received {:?}", header)).into());
        }
        if hex_len != self.reply {
            return Err(io::Error::new(io::ErrorKind::InvalidData, format!("Expected {:?}, received {}", hex_len, self.reply)).into());
        }

        let header = self.output_input_bulk(data)?;
        if header != FastbootHeader::Okay {
            return Err(io::Error::new(io::ErrorKind::InvalidData, format!("Expected OKAY header, received {:?}", header)).into());
        }

        Ok(())
    }

    /// Tries to write tar entry to device
    /// data is chunked in 16KiB parts
    pub fn download_tar_entry(&mut self, entry: &mut Entry<Box<dyn Read>>) -> io::Result<()> {
        let mut command = b"download:".to_vec();
        let hex_len = ByteVec::from_len(entry.size() as usize);
        command.extend_from_slice(hex_len.as_slice());

        let header = self.output_input_bulk(&mut command)?;
        if header != FastbootHeader::Data {
            return Err(io::Error::new(io::ErrorKind::InvalidData, format!("Expected DATA header, received {:?}", header)).into());
        }
        if hex_len.ne(self.reply.as_ref()) {
            return Err(io::Error::new(io::ErrorKind::InvalidData, format!("Expected {:?}, received {}", hex_len, self.reply)).into());
        }

        entry.take(16384);
        std::io::copy(entry, &mut self.writer)?;
        self.writer.flush()?;

        let header = self.get_reply()?;
        if header != FastbootHeader::Okay {
            return Err(io::Error::new(io::ErrorKind::InvalidData, format!("Expected OKAY header, received {:?}", header)).into());
        }

        self.writer.flush_end().map(|_| ())
    }

    pub fn output_input_bulk(&mut self, var: &[u8]) -> Result<FastbootHeader, io::Error> {
        self.output_bulk(var)?;
        self.get_reply()
    }

    pub fn bulk_transfer_expect(&mut self, var: &[u8], expected: FastbootHeader) -> Result<(), Box<dyn Error>> {
        self.output_bulk(var)?;
        let header = self.get_reply()?;
        if header != expected {
            return Err(io::Error::new(io::ErrorKind::InvalidData, format!("Expected OKAY header, received {:?}", header)).into());
        }
        Ok(())
    }

    pub fn getvar_u32(&mut self, var: &[u8], default: u32) -> u32 {
        match self.output_input_bulk(var) {
            Ok(header) => if header == FastbootHeader::Okay {
                return u32_from_bytes(self.reply.as_ref())
            }
            Err(e) => {
                error!("{}", e);
                error!("Failed to execute command: {}", str::from_utf8(var).unwrap());
            }
        }
        default
    }

    pub fn get_reply(&mut self) -> io::Result<FastbootHeader> {
        let len = self.input_bulk()?;
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
            // xperia 10 mark 3 XQ-BT41 send 13 bytes where last byte is null termination
            FastbootHeader::Data if len == 12 || len == 13 => {
                self.reply.truncate(12);
                self.reply.drain(0..4);
            },
            _ => ()
        };

        Ok(prefix)
    }

    pub fn output_bulk(
        &mut self,
        data: &[u8],
    ) -> io::Result<usize> {
        if let Err(e) = self.writer.write_all(data) {
            return Err(e);
        }

        match self.writer.flush_end() {
            Ok(_) => Ok(data.len()),
            Err(e) => Err(e)
        }
    }

    pub fn input_bulk(
        &mut self,
    ) -> io::Result<usize> {
        self.reply.clear();
        let mut short_reader = self.reader.until_short_packet();
        let r_len = short_reader.read_to_end(self.reply.as_mut());
        if let Ok(len) = r_len && let Err(e) =short_reader.consume_end() {
            println!("{}", e);
        }
        r_len
    }
}

#[repr(C)]
#[derive(PartialEq, Eq, Debug)]
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
