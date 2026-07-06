use anyhow::{anyhow, ensure, Context};
use std::fs::File;
use std::io::{BufRead, BufReader};
use std::path::PathBuf;
use std::str::FromStr;
use crate::types::{ByteVec, FastbootDevice};

#[derive(PartialEq, Debug)]
pub struct TrimArea {
    pub partition: u8,
    pub unit: usize,
    pub data: ByteVec
}

#[derive(PartialEq, Debug)]
enum TAParseState {
    Partition,
    UnitData,
    Extra,
    Complete
}

pub fn process_trim_area(ta_file: PathBuf) -> anyhow::Result<Option<TrimArea>> {
    let mut partition: u8 = 0;
    let mut unit: usize = 0;
    let mut unit_data = ByteVec::new();

    println!("Processing {}", ta_file.display());

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
                    partition = u8::from_str(trim).unwrap_or(0);
                    println!(" - Partition: {}", partition);
                    state = TAParseState::UnitData;
                }
            },
            TAParseState::UnitData => {
                if trim.len() < 8 {
                    return Err(anyhow!("Invalid unit!"));
                }

                let unit_hex: ByteVec = trim[0..8].as_bytes().into();
                unit = unit_hex.as_hexadecimal()? as usize;

                if is_blacklisted(unit) {
                    println!(" - Skipping unit 0x{unit:x}");
                    return Ok(None);
                }

                println!(" - Unit: {unit_hex} ({unit})");

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
                    println!(" - Found specific unit which doesn't contain data.");
                    state = TAParseState::Complete;
                    continue;
                }

                println!(" - Unit size: 0x{size:x}");

                unit_data = parse_hex_string(&line[offset..]).with_context(|| format!("Error parsing unit data: {}", &line[offset..]))?;

                if size < unit_data.len() {
                    return Err(anyhow!("Error: corrupted unit!"));
                };

                state = if size == unit_data.len() {
                    TAParseState::Complete
                } else {
                    TAParseState::Extra
                };
            },
            TAParseState::Extra => {
                unit_data.extend_vec(parse_hex_string(trim).with_context(|| format!("Error parsing unit data: {trim}"))?);
            },
            TAParseState::Complete => {
                return Err(anyhow!("Unit exceeds expected size!"));
            }
        }
    };

    ensure!(state == TAParseState::Complete, "Unexpected end of file!");

    Ok(Some(TrimArea{partition, unit, data: unit_data}))
}



/*        /*LOG("\n<<-------------------- Retrieval finished! Found unit: %s,"
            " Unit size: %04X, Unit data:%s\n",
             unit, unit_sz, unit_sz ? "" : " NULL");*/
        // TODO:
        unit_data = parse_hex_string(unit_data)?;

        command = format!("download:{:08x}", unit_size).into();
        println!("      {}", command);
        println!("      DATA: {unit_data}");*/
                            // TODO
                            /*                    if (transfer_bulk_ffi(dev, EP_OUT, command, strlen(command)) < 1) {
                                                    println!("      Error writing download command!\n");
                                                    ret = 0;
                                                    goto
                                                    finish_proced_ta;
                                                }

                                                if (!get_reply_ffi(dev)) {
                                                    println!("      Error, no download DATA reply!\n");
                                                    ret = 0;
                                                    goto
                                                    finish_proced_ta;
                                                }

                                                if (strlen(dev->vec.ptr) != 12) {
                                                    println!("      Error, download DATA reply size: %zu less than expected: 12!\n", strlen(dev->vec.ptr));
                                                    ret = 0;
                                                    goto
                                                    finish_proced_ta;
                                                }

                                                if (memcmp(dev->vec.ptr + 4, command + 9, 8) != 0) {
                                                    println!("      Error, download DATA reply string: %s is not equal to expected: DATA%s!\n", dev->vec.ptr, command + 9);
                                                    ret = 0;
                                                    goto
                                                    finish_proced_ta;
                                                }

                                                if (unit_sz > 0)
                                                {
                                                    if (transfer_bulk_ffi(dev, EP_OUT, unit_data, unit_sz) < 1) {
                                                        println!("      Error writing unit data!\n");
                                                        ret = 0;
                                                        goto
                                                        finish_proced_ta;
                                                    }
                                                }

                                                if (!get_reply_ffi(dev)) {
                                                    println!("      Error, no OKAY reply!\n");
                                                    ret = 0;
                                                    goto
                                                    finish_proced_ta;
                                                }

                                                if (strlen(dev->vec.ptr) < 4) {
                                                    println!("      Error, reply less than 4, got: %zu bytes!\n", strlen(dev->vec.ptr));
                                                    ret = 0;
                                                    goto
                                                    finish_proced_ta;
                                                }

                                                if (memcmp(dev->vec.ptr, "OKAY", 4) != 0) {
                                                    println!("      Error, didn't got OKAY reply! Got reply: %s\n", dev->vec.ptr);
                                                    ret = 0;
                                                    goto
                                                    finish_proced_ta;
                                                }

                                                println!("      OKAY.\n");

                                                snprintln!(command, sizeof(command), "Write-TA:%u:%u", partition, unit_dec);
                                                println!("      %s\n", command);

                                                if (transfer_bulk_ffi(dev, EP_OUT, command, strlen(command)) < 1) {
                                                    println!("      Error writing command WriteTA!\n");
                                                    ret = 0;
                                                    goto
                                                    finish_proced_ta;
                                                }

                                                if (!get_reply_ffi(dev)) {
                                                    println!("      Error, no OKAY reply!\n");
                                                    ret = 0;
                                                    goto
                                                    finish_proced_ta;
                                                }

                                                if (strlen(dev->vec.ptr) < 4) {
                                                    println!("      Error, reply less than 4, got: %zu bytes!\n", strlen(dev->vec.ptr));
                                                    ret = 0;
                                                    goto
                                                    finish_proced_ta;
                                                }

                                                if (memcmp(dev->vec.ptr, "OKAY", 4) != 0) {
                                                    println!("      Error, didn't got OKAY reply! Got reply: %s\n", dev->vec.ptr);
                                                    ret = 0;
                                                    goto
                                                    finish_proced_ta;
                                                }

                                                println!("      OKAY.\n");*/

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

pub fn parse_hex_string(hex: &str) -> anyhow::Result<ByteVec> {
    hex.as_bytes()
        .split(|&b| b == b' ')
        .map(|byte| {
            ensure!(byte.len() == 2, "Invalid hexadecimal string!");
            let str = str::from_utf8(byte).context("Invalid UTF8!")?;
            u8::from_str_radix(str, 16).context("Invalid hexadecimal string!")
        })
        .collect::<anyhow::Result<ByteVec>>()
}