use anyhow::{anyhow, Context, ensure, bail};
use regex::{Regex};
use std::fs::File;
use std::io::BufReader;
use std::path::{Path, PathBuf};
use log::{info, trace};
use xml::attribute::OwnedAttribute;
use xml::reader::XmlEvent;
use xml::ParserConfig;
use crate::sin;
use crate::sin::Sin;
use crate::ta::TrimArea;

pub struct BootConfiguration {
    pub _space_id: String,
    pub _name: String,
    pub _platform_id: Option<String>,
    pub plf_root_hash: String,
    pub hw_config_rev: String,
    pub boot_config: TrimArea,
    pub boot_images: Vec<Sin>,
}

impl BootConfiguration {
    fn builder() -> BootConfigurationBuilder {
        BootConfigurationBuilder::default()
    }
}

#[derive(Debug, Default)]
struct BootConfigurationBuilder {
    space_id: Option<String>,
    name: Option<String>,
    platform_id: Option<String>,
    plf_root_hash: Option<String>,
    hw_config_rev: Option<String>,
    boot_config: Option<PathBuf>,
    boot_images: Vec<PathBuf>,
}

impl BootConfigurationBuilder {
    pub fn build(self) -> anyhow::Result<BootConfiguration> {
        Ok(BootConfiguration {
            _space_id: self.space_id.ok_or_else(|| anyhow!("Missing space_id"))?,
            _name: self.name.ok_or_else(|| anyhow!("Missing name"))?,
            _platform_id: self.platform_id,
            plf_root_hash: self.plf_root_hash.ok_or_else(|| anyhow!("Missing plf_root_hash"))?,
            hw_config_rev: self.hw_config_rev.ok_or_else(|| anyhow!("Missing hw_config_rev"))?,
            boot_config: TrimArea::parse(self.boot_config.ok_or_else(|| anyhow!("Missing boot_config"))?)?,
            boot_images: sin::parse_paths(self.boot_images)?,
        })
    }
}

const BOOT_DELIVERY_ELEMENT: &str = "BOOT_DELIVERY";
const CONFIGURATION_ELEMENT: &str = "CONFIGURATION";
const ATTRIBUTES_ELEMENT: &str = "ATTRIBUTES";
const HWCONFIG_ELEMENT: &str = "HWCONFIG";
const BOOT_CONFIG_ELEMENT: &str = "BOOT_CONFIG";
const BOOT_IMAGES_ELEMENT: &str = "BOOT_IMAGES";
const PARTITION_IMAGES_ELEMENT: &str = "PARTITION_IMAGES";
const FILE_ELEMENT: &str = "FILE";

const SPACE_ID_ATTRIBUTE: &str = "SPACE_ID";
const NAME_ATTRIBUTE: &str = "NAME";
const VALUE_ATTRIBUTE: &str = "VALUE";
const REVISION_ATTRIBUTE: &str = "REVISION";
const PATH_ATTRIBUTE: &str = "PATH";

pub fn boot_delivery(root_key_hash: Option<&str>) -> anyhow::Result<BootConfiguration> {
    boot_delivery_in_dir(std::env::current_dir()?, root_key_hash)
}

pub fn boot_delivery_in_dir(dir: impl AsRef<Path>, root_key_hash: Option<&str>) -> anyhow::Result<BootConfiguration> {
    let boot_delivery_file = File::open(dir.as_ref().join("boot/boot_delivery.xml")).context("Failed to open boot_delivery.xml")?;
    let reader = ParserConfig::default().create_reader(BufReader::new(boot_delivery_file));

    let mut element_stack = Vec::<String>::new();
    let mut config = BootConfiguration::builder();

    let mut skip = false;

    for event in reader {
        let event = event?;
        match event {
            XmlEvent::StartElement { name, attributes: attrs, .. } => {
                trace!("Start element {:#?}", name.local_name);

                if name.local_name == BOOT_DELIVERY_ELEMENT && element_stack.is_empty() {
                    config.space_id = attribute_value(&attrs, SPACE_ID_ATTRIBUTE);
                }

                if name.local_name == CONFIGURATION_ELEMENT {
                    let name_attr = attribute_value(&attrs, NAME_ATTRIBUTE);

                    skip = !&name_attr.as_deref().is_some_and(|s| s.starts_with("COMMERCIAL"));

                    if skip {
                        element_stack.push(CONFIGURATION_ELEMENT.to_owned());
                    } else {
                        config.name = name_attr;
                    }
                }

                if skip {
                    continue;
                }

                if element_stack.iter().any(|e| e == CONFIGURATION_ELEMENT) {
                    if name.local_name == ATTRIBUTES_ELEMENT {
                        if let Some(val) = attribute_value(&attrs, VALUE_ATTRIBUTE) {
                            // newer devices dont have plat id
                            if let Some(cap) = Regex::new(r#"(PLATFORM_ID="[0-9A-F]{8}";)?PLF_ROOT_HASH="([0-9A-F]{96}|[0-9A-F]{64}|[0-9A-F]{48})"#)?.captures(val.as_str()) {
                                if let Some(m) = cap.get(2) {
                                    let str = m.as_str();
                                    if let Some(rkh) = root_key_hash {
                                        ensure!(rkh.starts_with(str), "PLF_ROOT_HASH does not match! Expected {rkh}, got {str}.");
                                    }
                                    config.plf_root_hash = Some(str.to_owned());
                                }
                            } else {
                                bail!("Failed to match value attribute regex");
                            }
                        }
                    }

                    if name.local_name == HWCONFIG_ELEMENT {
                        config.hw_config_rev = config.hw_config_rev.or_else(|| attribute_value(&attrs, REVISION_ATTRIBUTE));
                    }

                    if element_stack.last().is_some_and(|e| e == BOOT_CONFIG_ELEMENT) {
                        if config.boot_config.is_none() && let Some(path) = attribute_value(&attrs, PATH_ATTRIBUTE) {
                            config.boot_config = Some(dir.as_ref().join("boot").join(path))
                        }
                    }

                    if element_stack.last().is_some_and(|e| e == BOOT_IMAGES_ELEMENT) {
                        if name.local_name == FILE_ELEMENT {
                            if let Some(path) = attribute_value(&attrs, PATH_ATTRIBUTE) {
                                config.boot_images.push(dir.as_ref().join("boot").join(path));
                            }
                        }
                    }
                }

                element_stack.push(name.local_name);
            }
            XmlEvent::EndElement { name } => {
                trace!("End element {:#?}", name.local_name);

                if skip {
                    if name.local_name == CONFIGURATION_ELEMENT {
                        pop_element(&mut element_stack, &name.local_name)?;
                        skip = false;
                    }
                    continue;
                }

                pop_element(&mut element_stack, &name.local_name)?;

                if name.local_name == CONFIGURATION_ELEMENT {
                    let bc = config.build().context("Boot Configuration is missing required fields!")?;

                    info!("Found {} configuration in boot.xml", bc._name);

                    return Ok(bc);
                }
            }
            _ => {}
        }
    }

    Err(anyhow!("No commercial boot configuration found!"))
}

pub fn partition_delivery(lun_size: Option<u64>) -> anyhow::Result<Vec<Sin>> {
    partition_delivery_in_dir(std::env::current_dir()?, lun_size)
}

pub fn partition_delivery_in_dir(dir: impl AsRef<Path>, lun_size: Option<u64>) -> anyhow::Result<Vec<Sin>> {
    let partition_delivery_file = File::open(dir.as_ref().join("partition/partition_delivery.xml")).context("Failed to open partition_delivery.xml")?;
    let reader = ParserConfig::default().create_reader(BufReader::new(partition_delivery_file));

    let regex = Regex::new(r"partition-image-LUN0_(\d{8,10})_X-FLASH-ALL-[0-9A-F]{4}")?;

    let mut element_stack = Vec::<String>::new();
    let mut partition_paths = Vec::<PathBuf>::new();

    for event in reader {
        let event = event?;

        match event {
            XmlEvent::StartElement { name, attributes, .. } => {
                let is_file = name.local_name == FILE_ELEMENT;
                let in_partition_images = element_stack
                    .iter()
                    .any(|name| name == PARTITION_IMAGES_ELEMENT);

                if in_partition_images && is_file {
                    if let Some(path) = attribute_value(&attributes, PATH_ATTRIBUTE) {
                        let skip = if let Some(lun_size) = lun_size {
                            regex.captures(&path)
                                .and_then(|cap| cap.get(1))
                                .map(|match_| match_.as_str().parse::<u64>())
                                .transpose()?
                                .is_some_and(|u| lun_size != u)
                        } else {
                            false
                        };

                        if skip {
                            info!("Skipping {path}");
                        } else {
                            partition_paths.push(dir.as_ref().join("partition").join(&path));
                        }
                    }
                }

                element_stack.push(name.local_name);
            }
            XmlEvent::EndElement { name } => {
                pop_element(&mut element_stack, &name.local_name)?;
            }
            _ => {}
        }
    }

    sin::parse_paths(partition_paths)
        .map(|sins| { info!("Found {} partition files", sins.len()); sins })
}

fn pop_element(element_stack: &mut Vec<String>, ended_name: &str) -> anyhow::Result<()> {
    let started_name = element_stack.pop().ok_or_else(|| {
        anyhow::anyhow!("XML parser stack underflow while ending {ended_name}")
    })?;

    anyhow::ensure!(
        started_name == ended_name,
        "XML parser stack got out of sync: expected end of {started_name}, got end of {ended_name}"
    );

    Ok(())
}

fn attribute_value(attributes: &[OwnedAttribute], name: &str) -> Option<String> {
    attributes.iter()
        .find(|attribute| attribute.name.local_name == name)
        .map(|attribute| attribute.value.clone())
}

#[cfg(test)]
mod tests {
    use crate::xml;

    #[test]
    fn boot() {
        xml::boot_delivery_in_dir("tests/data/H8314_Customized_US_52.1.A.3.137-R7C", None).unwrap();
        xml::boot_delivery_in_dir("tests/data/XQ-CQ72_Customized_HK_64.2.A.2.272", None).unwrap();
        let _1vi = xml::boot_delivery_in_dir("tests/data/XQ-EC72_Customized_HK_69.2.A.4.110", None).unwrap();
        xml::boot_delivery_in_dir("tests/data/XQ-GE44_Customized_JP_73.0.A.2.62", None).unwrap();
        xml::boot_delivery_in_dir("tests/data/XQ-GH44_Customized_JP_74.1.A.2.42", None).unwrap();

        assert_eq!(_1vi._space_id, "8650-0001");
        assert_eq!(_1vi._name, "COMMERCIAL_002270E1");
        assert!(_1vi.plf_root_hash.starts_with("2B9EB2535D4A1179B294D4D083F68D9D3DDC95164111B84157D8BE0169E35DE5C6F79EA6E1034FE3D7A684F3C5166250"));
        assert_eq!(_1vi.hw_config_rev, "HWC_XGeneric_001");
        assert_eq!(_1vi.boot_config.name, "PDX245_XBootConfig_MiscTA");
        assert_eq!(_1vi.boot_images.iter().map(|sin| sin.prefix.as_str()).collect::<Vec<&str>>(), vec!["bootloader"]);
    }

    #[test]
    fn partition() {
        xml::partition_delivery_in_dir("tests/data/H8314_Customized_US_52.1.A.3.137-R7C", None).unwrap();
        xml::partition_delivery_in_dir("tests/data/XQ-CQ72_Customized_HK_64.2.A.2.272", None).unwrap();
        xml::partition_delivery_in_dir("tests/data/XQ-EC72_Customized_HK_69.2.A.4.110", Some(249896960)).unwrap();
        xml::partition_delivery_in_dir("tests/data/XQ-GE44_Customized_JP_73.0.A.2.62", None).unwrap();
        xml::partition_delivery_in_dir("tests/data/XQ-GH44_Customized_JP_74.1.A.2.42", None).unwrap();
    }
}