use std::error::Error;
use std::io;
use std::io::{Read, Write};
use nusb::io::{EndpointRead, EndpointWrite};
use nusb::transfer::{Bulk, In, Out};
use nusb::{Device, Interface};
use std::time::Duration;
use log::error;
use tar::Entry;
use crate::u32_from_bytes;

const IN: u8 = 0x81;
const OUT: u8 = 0x01;

#[repr(C)]
pub struct CVec {
    pub ptr: *mut u8,
    pub len: usize,
    pub capacity: usize,
}

#[repr(C)]
pub struct FastbootDevice {
    device: Device,
    interface: Interface,
    pub reader: EndpointRead<Bulk>,
    pub writer: EndpointWrite<Bulk>,
    // TODO: move vector here
}

const BUFFER_SIZE: usize = 1024;
const NUM_TRANSFERS: usize = 4;
const TEN_SEC: Duration = Duration::from_secs(10);

impl FastbootDevice {
    pub fn new(device: Device, interface: Interface) -> Self {
        let reader = interface.endpoint::<Bulk, In>(IN).unwrap().reader(BUFFER_SIZE).with_num_transfers(NUM_TRANSFERS).with_read_timeout(TEN_SEC);
        let writer = interface.endpoint::<Bulk, Out>(OUT).unwrap().writer(BUFFER_SIZE).with_num_transfers(NUM_TRANSFERS).with_write_timeout(TEN_SEC);
        Self { device, interface, reader, writer }
    }

    pub fn fastboot_cmd(&mut self, buffer: &mut Vec<u8>, cmd: &[u8]) -> Result<(), Box<dyn Error>> {
        if cmd.starts_with(b"getvar:") {
            // normal if it fail
            self.output_input_bulk(buffer, cmd)?;
        }
        else if (cmd.starts_with(b"Get-")) {
            self.output_bulk(cmd)?;

            let header = self.get_reply(buffer)?;
            if header != FastbootHeader::Data || buffer.len() != 8 {
                return Err(io::Error::new(io::ErrorKind::InvalidData, format!("Expected DATA header, received {:?}", header)).into());
            }

            let len = usize::from_str_radix(str::from_utf8(&buffer).expect("Failed to parse buffer as str"), 16).expect("Failed to parse str as hexadecimal");

            // second read for actual data
            self.input_bulk(buffer)?;
            if FastbootHeader::from(buffer.as_slice()) == FastbootHeader::Okay { todo!("unimplemented OKAY after DATA") }
            assert_eq!(len, buffer.len());

            // if not OKAY in 2nd read, should read a 3rd time to acknowledge
            assert_eq!(self.get_reply(&mut vec![0, 0, 0, 0])?, FastbootHeader::Okay);
        }
        else if cmd.eq(b"Write-TA:2:10100") {
            let header = self.output_input_bulk(buffer, cmd)?;
            if header != FastbootHeader::Okay {
                return Err(io::Error::new(io::ErrorKind::InvalidData, format!("Expected OKAY header, received {:?}", header)).into());
            }
        }
        else {
            panic!("Fastboot command prefix '{}' not found", str::from_utf8(cmd).unwrap());
        }

        Ok(())
    }

    /// Tries to write data to device
    pub fn fastboot_download(&mut self, buffer: &mut Vec<u8>, data: &[u8]) -> Result<(), Box<dyn Error>> {
        let mut full_cmd = b"download:".to_vec();
        let string = format!("{:08X}", data.len());
        let hex_len = string.as_bytes();
        full_cmd.extend_from_slice(hex_len);

        let header = self.output_input_bulk(buffer, &mut full_cmd)?;
        if header != FastbootHeader::Data {
            return Err(io::Error::new(io::ErrorKind::InvalidData, format!("Expected DATA header, received {:?}", header)).into());
        }
        if buffer.ne(&hex_len) {
            return Err(io::Error::new(io::ErrorKind::InvalidData, format!("Expected {:?}, received {:?}", hex_len, buffer)).into());
        }

        let header = self.output_input_bulk(buffer, data)?;
        if header != FastbootHeader::Okay {
            return Err(io::Error::new(io::ErrorKind::InvalidData, format!("Expected OKAY header, received {:?}", header)).into());
        }

        Ok(())
    }

    pub fn fastboot_download_entry(&mut self, entry: &mut Entry<Box<dyn Read>>) -> io::Result<()> {
        let mut command = b"download:".to_vec();
        let string = format!("{:08X}", entry.size());
        let hex_len = string.as_bytes();
        command.extend_from_slice(hex_len);

        let mut buffer = Vec::new();

        let header = self.output_input_bulk(buffer.as_mut(), &mut command)?;
        if header != FastbootHeader::Data {
            return Err(io::Error::new(io::ErrorKind::InvalidData, format!("Expected DATA header, received {:?}", header)).into());
        }
        if buffer.ne(&hex_len) {
            return Err(io::Error::new(io::ErrorKind::InvalidData, format!("Expected {:?}, received {:?}", hex_len, buffer)).into());
        }

        entry.take(16384);
        std::io::copy(entry, &mut self.writer)?;
        self.writer.flush()?;

        let header = self.get_reply(buffer.as_mut())?;
        if header != FastbootHeader::Okay {
            return Err(io::Error::new(io::ErrorKind::InvalidData, format!("Expected OKAY header, received {:?}", header)).into());
        }

        self.writer.flush_end().map(|_| ())
    }

    pub fn output_input_bulk(&mut self, buffer: &mut Vec<u8>, var: &[u8]) -> Result<FastbootHeader, io::Error> {
        self.output_bulk(var)?;
        self.get_reply(buffer)
    }

    pub fn bulk_transfer_expect(&mut self, buffer: &mut Vec<u8>, var: &[u8], expected: FastbootHeader) -> Result<(), Box<dyn Error>> {
        self.output_bulk(var)?;
        let header = self.get_reply(buffer)?;
        if header != expected {
            return Err(io::Error::new(io::ErrorKind::InvalidData, format!("Expected OKAY header, received {:?}", header)).into());
        }
        Ok(())
    }

    pub fn getvar_u32(&mut self, buffer: &mut Vec<u8>, var: &[u8], default: u32) -> u32 {
        match self.output_input_bulk(buffer, var) {
            Ok(header) => if header == FastbootHeader::Okay {
                return u32_from_bytes(buffer)
            }
            Err(e) => {
                error!("{}", e);
                error!("Failed to execute command: {}", str::from_utf8(var).unwrap());
            }
        }
        default
    }

    pub fn get_reply(&mut self, reply: &mut Vec<u8>) -> io::Result<FastbootHeader> {
        let len = self.input_bulk(reply)?;
        if len < 4 { return Ok(FastbootHeader::NoHeader); }

        let prefix = FastbootHeader::from(&reply[0..4]);
        match prefix {
            FastbootHeader::Okay | FastbootHeader::Fail if len == 4 => {
                reply.clear();
            },
            FastbootHeader::Okay | FastbootHeader::Fail if len > 4 => {
                // strip the prefix and copy the rest
                reply.drain(0..4);
            }
            // xperia 10 mark 3 XQ-BT41 send 13 bytes where last byte is null termination
            FastbootHeader::Data if len == 12 || len == 13 => {
                reply.truncate(12);
                reply.drain(0..4);
            },
            _ => ()
        };

        Ok(prefix)
    }

    pub fn output_bulk(
        &mut self,
        data: &[u8],
    ) -> std::io::Result<usize> {
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
        vec: &mut Vec<u8>,
    ) -> std::io::Result<usize> {
        vec.clear();
        let mut short_reader = self.reader.until_short_packet();
        let r_len = short_reader.read_to_end(vec);
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
