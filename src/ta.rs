use crate::fastboot::FastbootDevice;
use crate::types::ProgressBar;
use crate::utils::{is_ta_file, noerase_in_updatexml};
use anyhow::{Context, bail, ensure};
use log::{debug, info};
use std::fs;
use std::path::Path;

#[derive(PartialEq, Debug)]
pub struct BootConfigUnit {
    pub unit: u32,
    pub data: Vec<u8>
}

#[derive(PartialEq, Debug)]
pub struct TrimArea {
    pub name: String,
    pub partition: u8,
    pub units: Vec<BootConfigUnit>,
}

impl TrimArea {
    pub fn new(name: &str, partition: u8, units: Vec<BootConfigUnit>) -> Self {
        Self {
            name: name.to_owned(),
            partition,
            units
        }
    }

    pub fn parse_file(path: impl AsRef<Path>) -> anyhow::Result<Self> {
        let path = path.as_ref();

        let name = path
            .file_name()
            .and_then(|n| n.to_str())
            .context("Invalid file name")?;

        debug!("Parsing {name}");

        let content = fs::read_to_string(path)?;

        let mut lines = content
            .lines()
            .map(str::trim)
            .filter(|line| !line.is_empty() && !line.starts_with("//"));

        let partition_str = lines.next().context("Partition not found!")?;
        ensure!(partition_str.len() == 2 && partition_str.bytes().all(|b| b.is_ascii_hexdigit()), "Invalid unit partition!");

        let partition = u8::from_str_radix(partition_str, 16)?;
        // debug!("  Partition: {partition}");

        let mut boot_config_units = Vec::new();

        while let Some(line) = lines.next() {
            ensure!(line.len() >= 8, "Invalid unit!");

            let unit = u32::from_str_radix(&line[..8], 16).with_context(|| format!("Invalid unit id: {}", &line[..8]))?;

            let (size_str, rest) = if line.len() >= 17 && line.as_bytes().get(8) == Some(&b' ') && line.as_bytes().get(13) != Some(&b' ') {
                (&line[9..17], line[17..].trim())
            } else if line.len() >= 13 && line.as_bytes().get(8) == Some(&b' ') {
                (&line[9..13], line[13..].trim())
            } else {
                bail!("Invalid unit format: {line}");
            };

            let size = usize::from_str_radix(size_str, 16).with_context(|| format!("Error parsing unit size: {size_str}"))?;

            if size == 0 {
                debug!("  Found unit which contains no data. Skipping");
                continue;
            }

            if !is_blacklisted(unit) {
                debug!("  Unit: 0x{unit:08X} ({unit})");
                debug!("  Size: 0x{size:X}");
            }

            let mut data = parse_hex_string(rest)?;

            while data.len() < size {
                let next_line = lines.next().context("Unexpected EOF in unit data")?;
                data.extend(&parse_hex_string(next_line)?);
            }

            ensure!(data.len() == size, "Unit data does not match expected length!");

            if is_blacklisted(unit) {
                debug!("  Skipping blacklisted unit: 0x{unit:08X} ({unit})");
            } else {
                boot_config_units.push(BootConfigUnit { unit, data });
            }
        }

        Ok(TrimArea::new(name, partition, boot_config_units))
    }

    pub fn flash(&self, usb: &mut FastbootDevice) -> anyhow::Result<()> {
        let mut progress = ProgressBar::new(self.units.len() as u64, format!("Flashing {}", self.name));

        for unit in &self.units {
            usb.download(&unit.data)?;
            usb.command(format!("Write-TA:{}:{}", self.partition, unit.unit))?;
            progress.inc();
        }

        progress.okay();
        Ok(())
    }
}

pub fn parse_all() -> anyhow::Result<Vec<TrimArea>> {
    let tas = fs::read_dir("./").unwrap()
                                .filter_map(is_ta_file)
                                .filter(|path| !noerase_in_updatexml(path.file_name().unwrap()))
                                .map(TrimArea::parse_file)
                                .collect::<Result<Vec<_>, _>>()
                                .context("All .ta files could not be successfully parsed!")?;

    ensure!(!tas.is_empty(), "No .ta files found!");
    info!("Found {} .ta files", tas.len());

    Ok(tas)
}

fn parse_hex_string(hex: &str) -> anyhow::Result<Vec<u8>> {
    if hex.trim().is_empty() {
        return Ok(Vec::new());
    }

    hex.split_whitespace()
       .map(|byte| {
           ensure!(byte.len() == 2, format!("Invalid hexadecimal string! h:{hex}, b:{byte}"));
           u8::from_str_radix(byte, 16).context("Invalid hexadecimal string!")
       })
       .collect::<anyhow::Result<Vec<u8>>>()
       .with_context(|| format!("Error parsing unit data: {hex}"))
}

fn is_blacklisted(unit: u32) -> bool {
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

#[cfg(test)]
mod tests {
    use std::path::PathBuf;
    use super::*;

    #[test]
    fn test_ta_files() {
        let compare = |file: &str, partition: u8, units: Vec<BootConfigUnit>| {
            let result = TrimArea::parse_file(PathBuf::from(file)).expect(file);
            assert_eq!(result.partition, partition);
            assert_eq!(result.units, units);
        };

        // unsafe { std::env::set_var("RUST_LOG", "nfrs=debug"); }
        // env_logger::init();

        compare("/media/secondary/xperia/XQ-EC72_Customized_HK_69.2.A.4.90/auto-boot.ta", 2, vec![BootConfigUnit { unit: 0x907, data: vec![0] }]);
        compare("/media/secondary/xperia/XQ-EC72_Customized_HK_69.2.A.4.90/CustomerID_S20000480_001_HK_c001526.ta", 2, vec![BootConfigUnit { unit: 0x87B, data: vec![0x63, 0x30, 0x30, 0x31, 0x35, 0x32, 0x36] }]);
        compare("/media/secondary/xperia/XQ-EC72_Customized_HK_69.2.A.4.90/osv-restriction.ta", 2, vec![BootConfigUnit { unit: 0x91A, data: vec![0x0] }]);
        compare("/media/secondary/xperia/XQ-EC72_Customized_HK_69.2.A.4.90/reset-kernel-cmd-debug.ta", 2, vec![BootConfigUnit { unit: 0x9A9, data: vec![0x0] }]);
        compare("/media/secondary/xperia/XQ-EC72_Customized_HK_69.2.A.4.90/reset-retail-demo-active-sts.ta", 2, vec![]);

        compare("/media/secondary/xperia/H8314_O2_Pay_monthly_UK_52.1.A.3.49-R6C/auto-boot.ta", 2, vec![BootConfigUnit { unit: 0x90C, data: vec![0x0] }]);
        compare("/media/secondary/xperia/H8314_O2_Pay_monthly_UK_52.1.A.3.49-R6C/cust-reset.ta", 2, vec![BootConfigUnit { unit: 0x8A4, data: vec![0x0, 0x0, 0x0, 0x0, 0x0, 0x0, 0x0, 0x0, 0x0, 0x0, 0x0, 0x0, 0x0, 0x0] }]);
        compare("/media/secondary/xperia/H8314_O2_Pay_monthly_UK_52.1.A.3.49-R6C/master-reset.ta", 2, vec![BootConfigUnit { unit: 0x9F6, data: vec![0x1] }]);
        compare("/media/secondary/xperia/H8314_O2_Pay_monthly_UK_52.1.A.3.49-R6C/osv-restriction.ta", 2, vec![BootConfigUnit { unit: 0x91A, data: vec![0x0] }]);
        compare("/media/secondary/xperia/H8314_O2_Pay_monthly_UK_52.1.A.3.49-R6C/reset-kernel-cmd-debug.ta", 2, vec![BootConfigUnit { unit: 0x9A9, data: vec![0x0] }]);
        compare("/media/secondary/xperia/H8314_O2_Pay_monthly_UK_52.1.A.3.49-R6C/reset-non-secure-adb.ta", 2, vec![]);
        compare("/media/secondary/xperia/H8314_O2_Pay_monthly_UK_52.1.A.3.49-R6C/reset-wipe-reason.ta", 2, vec![]);
        compare("/media/secondary/xperia/H8314_O2_Pay_monthly_UK_52.1.A.3.49-R6C/simlock.ta", 2, vec![]);
    }
}