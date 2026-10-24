use anyhow::{Context, Result};
use derive_more::{AsRef, Deref, DerefMut};
use indicatif::{ProgressFinish, ProgressStyle};
use std::borrow::Cow;
use std::fmt::{Debug, Display, Formatter};

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

impl From<String> for Slot {
    fn from(value: String) -> Self {
        value.as_str().into()
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
pub struct ByteVec {
    data: Vec<u8>
}

impl ByteVec {
    pub(crate) fn new() -> Self {
        Vec::new().into()
    }

    pub fn from_len(len: u32) -> Self {
        ByteVec::from(format!("{len:08x}"))
    }

    pub fn as_decimal(&self) -> Result<u32> {
        let str = str::from_utf8(&self.data)?;
        let n = u32::from_str_radix(str, 10).with_context(|| format!("String: \"{str}\""))?;
        Ok(n)
    }

    pub fn as_hexadecimal(&self) -> Result<u32> {
        let str = str::from_utf8(&self.data)?;
        let n = u32::from_str_radix(str, 16).with_context(|| format!("String: \"{str}\""))?;
        Ok(n)
    }

    pub fn to_hexadecimal_string(&self) -> String {
        const NIBBLE_TO_HEX: fn(u8) -> char = |b: u8| -> char {
            let i = b.min(0x10);
            (if i < 10 {
                '0' as u8 + i
            } else {
                'A' as u8 + i - 10
            }) as char
        };

        let mut string = String::new();

        for b in self.data.iter() {
            string.push(NIBBLE_TO_HEX(b >> 4));
            string.push(NIBBLE_TO_HEX(b & 0xF));
        }

        string
    }

    pub fn into_string(self) -> Result<String> {
        Ok(String::from_utf8(self.data)?)
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

impl FromIterator<u8> for ByteVec {
    fn from_iter<T: IntoIterator<Item = u8>>(iter: T) -> Self {
        iter.into_iter().collect::<Vec<u8>>().into()
    }
}

impl Debug for ByteVec {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", &self.data.iter().map(|&b| b as char).collect::<String>())
    }
}

impl Display for ByteVec {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", &self.data.iter().map(|&b| b as char).collect::<String>())
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


const PREFIX_LEN: usize = 40;

pub struct ProgressBar {
    bar: indicatif::ProgressBar,
}

impl ProgressBar {
    pub fn new(parts: u64, text: impl Into<String>) -> Self {
        let template = format!("{{prefix:<{PREFIX_LEN}}} {{elapsed:>3}} [{{bar:40.cyan/blue}}] {{percent:>3}}% {{msg}}");

        let mut prefix = text.into();
        prefix.truncate(PREFIX_LEN);

        Self { bar: indicatif::ProgressBar::new(parts)
            .with_position(0)
            .with_style(ProgressStyle::with_template(template.as_str()).unwrap().progress_chars("#*-"))
            .with_prefix(prefix)
            .with_finish(ProgressFinish::AbandonWithMessage(Cow::Owned(console::style("FAIL").red().to_string())))
        }
    }

    pub fn inc(&mut self) {
        self.bar.inc(1)
    }

    pub fn set_position(&self, pos: u64) {
        self.bar.set_position(pos);
    }

    pub fn step<T, E>(&mut self, f: impl FnOnce() -> Result<T, E>) -> Result<T, E> {
        let value = f()?;
        self.inc();
        Ok(value)
    }
    
    pub fn okay(self) {
        self.bar.finish_with_message(console::style("OKAY").green().to_string())
    }

    pub fn fail(self) {
        self.bar.abandon();
    }
}