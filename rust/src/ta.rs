use anyhow::{anyhow, ensure, Context};
use std::fs::File;
use std::io::{BufRead, BufReader};
use std::path::PathBuf;
use std::str::FromStr;
use log::debug;
use crate::types::{ByteVec, FastbootDevice};

#[derive(PartialEq, Debug)]
pub struct TrimArea {
    pub partition: u8,
    pub boot_config_units: Vec<BootConfigUnit>,
}

impl TrimArea {
    pub fn new(partition: u8, boot_config_units: Vec<BootConfigUnit>) -> Self {
        Self {
            partition,
            boot_config_units
        }
    }
}

#[derive(PartialEq, Debug)]
pub struct BootConfigUnit {
    pub unit: usize,
    pub data: ByteVec
}

#[derive(Default, Debug)]
struct BootConfigUnitBuilder {
    unit: Option<usize>,
    size: Option<usize>,
    data: ByteVec
}

impl BootConfigUnitBuilder {
    fn append(&mut self, data: ByteVec) {
        self.data.append(&data) ;
    }

    fn build(self) -> BootConfigUnit {
        BootConfigUnit {
            unit: self.unit.unwrap(),
            data: self.data
        }
    }

    fn clear(&mut self) {
        self.unit = None;
        self.size = None;
        self.data.clear();
    }
}


#[derive(PartialEq, Debug)]
enum TAParseState {
    Partition,
    UnitData,
    ExtraData
}

pub fn process_trim_area(ta_file: PathBuf) -> anyhow::Result<TrimArea> {
    println!("Processing {}", ta_file.display());

    let mut partition = None;
    let mut vec = Vec::<BootConfigUnit>::new();

    let mut builder = BootConfigUnitBuilder::default();
    let mut state = TAParseState::Partition;

    let file = File::open(ta_file).context("Unable to open file")?;
    let reader = BufReader::new(file);

    for res in reader.lines() {
        let line = res?;
        let trim =  line.trim();

        if trim.is_empty() || trim.starts_with("//") {
            continue;
        }

        match state {
            TAParseState::Partition => {
                if trim.len() != 2 {
                    return Err(anyhow!("Invalid partition!"));
                }

                let bytes = trim.as_bytes();
                if bytes[0].is_ascii_digit() && bytes[1].is_ascii_digit() {
                    partition = Some(u8::from_str(trim)?);
                    debug!("Partition: {}", partition.unwrap());
                    state = TAParseState::UnitData;
                }
            },
            TAParseState::UnitData => {
                if trim.len() < 8 {
                    return Err(anyhow!("Invalid unit!"));
                }

                let unit_hex: ByteVec = trim[0..8].as_bytes().into();
                let unit = unit_hex.as_hexadecimal()? as usize;

                let blacklisted = is_blacklisted(unit);

                if !blacklisted {
                    debug!("- Unit: 0x{unit_hex} ({unit})");
                }

                let (size, offset) = {
                    /*
                     * in case of 32 bit unit size!
                     * unit(8) + space(1) + unit size(8) + space(1)
                     */
                    let (size_str, offset) = if trim.len() >= 18 && trim.chars().nth(8).unwrap() == ' ' && trim.chars().nth(17).unwrap() == ' ' {
                        (&trim[9..17], 18)
                    } else {
                        (&trim[9..13], 14)
                    };
                    let s = usize::from_str_radix(size_str, 16).with_context(|| format!("Error parsing unit size: {size_str}"))?;
                    (s, offset)
                };

                if size == 0 {
                    debug!("- Found specific unit which doesn't contain data");
                    continue;
                }

                if !blacklisted {
                    debug!("  Unit size: 0x{size:x}");
                }

                builder.unit = Some(unit);
                builder.size = Some(size);

                builder.append(parse_hex_string(&line[offset..])?);

                if size == builder.data.len() {
                    vec.push(builder.build());
                    builder = BootConfigUnitBuilder::default();
                } else if size > builder.data.len() {
                    state = TAParseState::ExtraData
                } else {
                    return Err(anyhow!("Corrupted unit data!"));
                }
            },
            TAParseState::ExtraData => {
                builder.append(parse_hex_string(trim)?);

                if builder.size.context("Parsing reaching ExtraData without matching unit size!")? == builder.data.len() {
                    let unit = builder.unit.unwrap();
                    if is_blacklisted(unit) {
                        debug!("- Skipping unit 0x{unit:x}");
                        builder.clear();
                    } else {
                        vec.push(builder.build());
                        builder = BootConfigUnitBuilder::default();
                    }

                    state = TAParseState::UnitData
                };
            }
        }
    };

    println!();

    Ok(TrimArea::new(partition.context("Partition not found!")?, vec))
}

fn parse_hex_string(hex: &str) -> anyhow::Result<ByteVec> {
    hex.as_bytes()
       .split(|&b| b == b' ')
       .map(|byte| {
           ensure!(byte.len() == 2, format!("Invalid hexadecimal string! h:{hex}, b:{byte:?}"));
           let str = str::from_utf8(byte).context("Invalid UTF8!")?;
           u8::from_str_radix(str, 16).context("Invalid hexadecimal string!")
       })
       .collect::<anyhow::Result<ByteVec>>()
       .with_context(|| format!("Error parsing unit data: {hex}"))
}

pub fn flash_trim_area(usb: &mut FastbootDevice, ta: TrimArea) -> anyhow::Result<()> {
    for unit in &ta.boot_config_units {
        usb.download(unit.data.as_slice())?;

        let cmd = format!("Write-TA:{}:{}", ta.partition, unit.unit);
        usb.command(cmd.as_str())?;
    }

    Ok(())
}

pub fn is_blacklisted(unit: usize) -> bool {
    /*
        unit 0x7d3 (2003) hardware config
        unit 0x7da (2010) simlock
        unit 0x851 (2129) simlock signature
        unit 0x1324 (4900) device id
        unit 0x1046F (66671) google lock state ( allow bootloader unlock in dev settings )
        unit 0x9A9 (2473) value 1 for enable serial console or value 0 (default) to disable (https://forum.xda-developers.com/showpost.php?p=80212371&postcount=1125)
        unit 0x10471 (66673) protocol switch? Or keystore? What is this? Depend on existance of unit 0x36A (https://forum.xda-developers.com/showpost.php?p=80176195&postcount=1093)
    */
    // if /*memcmp(unit, "000008B2", 8) == 0 || unlock key */
    matches!(unit, 0x7D3 /* hardware config */ |
                   0x7DA /* simlock */ |
                   0x851 /* simlock signature */ |
                   0x8A2 /* device name */ |
                   0x1324 /* device id */ |
                   0x1046B /* drm key */)
}