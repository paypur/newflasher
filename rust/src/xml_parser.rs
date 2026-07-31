use anyhow::Context;
use regex::regex;
use std::fs::File;
use std::io::BufReader;
use std::path::Path;
use xml::attribute::OwnedAttribute;
use xml::reader::XmlEvent;
use xml::ParserConfig;

#[derive(Debug, Default)]
pub struct BootDelivery {
    pub space_id: Option<String>,
    pub configurations: Vec<BootConfiguration>,
}
#[derive(Debug, Default)]
pub struct BootConfiguration {
    pub name: String,
    pub platform_id: String,
    pub plf_root_hash: String,
    pub hw_config_rev: String,
    pub boot_config: String,
    pub boot_images: Vec<String>,
}

impl BootConfiguration {
    fn builder() -> BootConfigurationBuilder {
        BootConfigurationBuilder::default()
    }
}

#[derive(Debug, Default)]
struct BootConfigurationBuilder {
    name: Option<String>,
    platform_id: String,
    plf_root_hash: String,
    hw_config_rev: Option<String>,
    boot_config: Option<String>,
    boot_images: Vec<String>,
}

impl BootConfigurationBuilder {
    pub fn build(self) -> Option<BootConfiguration> {
        Some(BootConfiguration {
            name: self.name?,
            platform_id: self.platform_id,
            plf_root_hash: self.plf_root_hash,
            hw_config_rev: self.hw_config_rev?,
            boot_config: self.boot_config?,
            boot_images: self.boot_images,
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

pub fn boot_delivery(path: impl AsRef<Path>) -> anyhow::Result<BootDelivery> {
    let path = path.as_ref();
    let boot_delivery_file = File::open(path).with_context(|| format!("failed to open {}", path.display()))?;
    let reader = ParserConfig::default().create_reader(BufReader::new(boot_delivery_file));

    let mut element_stack = Vec::<String>::new();
    let mut boot_delivery = BootDelivery::default();
    let mut builder = BootConfiguration::builder();

    for event in reader {
        let event = event?;
        match event {
            XmlEvent::StartElement { name, attributes: attrs, .. } => {
                if name.local_name == BOOT_DELIVERY_ELEMENT && element_stack.is_empty() && boot_delivery.space_id.is_none() {
                    boot_delivery.space_id = attribute_value(&attrs, SPACE_ID_ATTRIBUTE);
                }

                if name.local_name == CONFIGURATION_ELEMENT {
                    builder = BootConfiguration::builder();
                    builder.name = attribute_value(&attrs, NAME_ATTRIBUTE);
                }

                if element_stack.iter().any(|e| e == CONFIGURATION_ELEMENT) {
                    if name.local_name == ATTRIBUTES_ELEMENT {
                        let regex = regex!(r#"PLATFORM_ID="([0-9A-F]{8})";PLF_ROOT_HASH="([0-9A-F]{48}|[0-9A-F]{32})"#);

                        if let Some(val) = attribute_value(&attrs, VALUE_ATTRIBUTE) {
                            if let Some(cap) = regex.captures(val.as_str()) {
                                builder.platform_id = cap.get(1).unwrap().as_str().to_string();
                                builder.plf_root_hash = cap.get(2).unwrap().as_str().to_string();
                            }
                        }
                    }

                    if name.local_name == HWCONFIG_ELEMENT {
                        builder.hw_config_rev = builder.hw_config_rev.or_else(|| attribute_value(&attrs, REVISION_ATTRIBUTE));
                    }

                    if element_stack.last().is_some_and(|e| e == BOOT_CONFIG_ELEMENT) {
                        builder.boot_config = builder.boot_config.or_else(|| attribute_value(&attrs, PATH_ATTRIBUTE));
                    }

                    if element_stack.last().is_some_and(|e| e == BOOT_IMAGES_ELEMENT) {
                        if name.local_name == FILE_ELEMENT {
                            if let Some(path) = attribute_value(&attrs, PATH_ATTRIBUTE) {
                                builder.boot_images.push(path);
                            }
                        }
                    }
                }

                element_stack.push(name.local_name);
            }
            XmlEvent::EndElement { name } => {
                pop_element(&mut element_stack, &name.local_name)?;

                if name.local_name == CONFIGURATION_ELEMENT {
                    let option = builder.build();
                    if let Some(bd) = option {
                        boot_delivery.configurations.push(bd);
                        builder = BootConfiguration::builder();
                    } else {
                        panic!("boot configuration is missing required fields");
                    }
                }
            }
            _ => {}
        }
    }

    Ok(boot_delivery)
}

pub fn partition_delivery(path: impl AsRef<Path>) -> anyhow::Result<Vec<String>> {
    let path = path.as_ref();
    let partition_delivery_file = File::open(path).with_context(|| format!("failed to open {}", path.display()))?;
    let reader = ParserConfig::default().create_reader(BufReader::new(partition_delivery_file));

    let mut element_stack = Vec::<String>::new();
    let mut partition_files = Vec::<String>::new();

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
                        partition_files.push(path);
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

    Ok(partition_files)
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