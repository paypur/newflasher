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
    pub fn append(&mut self, vec: &Self) {
        self.data.extend(vec.as_ref());
    }

    pub fn from_len(len: usize) -> Self {
        ByteVec::from(format!("{len:08x}"))
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
            .with_style(ProgressStyle::with_template(template.as_str()).unwrap().progress_chars("#*-"))
            .with_prefix(prefix)
            .with_finish(ProgressFinish::AbandonWithMessage(Cow::Owned(console::style("FAIL").red().to_string())))
        }
    }

    pub fn set_position(&self, pos: u64) {
        self.bar.set_position(pos);
    }

    pub fn okay(self) {
        self.bar.finish_with_message(console::style("OKAY").green().to_string())
    }

    pub fn fail(self) {
        self.bar.abandon();
    }
}