use anyhow::{anyhow, Context};
use std::fs::File;
use std::io::{BufRead, BufReader};
use std::path::PathBuf;
use std::str::FromStr;
use crate::types::{ByteVec, FastbootDevice};
use crate::utils::trim_rs;

#[derive(PartialEq, Debug)]
pub struct TrimArea {
    pub partition: u8,
    pub unit: u32,
    pub data: ByteVec
}

pub fn process_ta_file(ta_file: PathBuf/*, dev: FastbootDevice*/) -> anyhow::Result<TrimArea> {
    let mut unit = ByteVec::new();
    let mut command = ByteVec::new();
    let mut unit_size: u32 = 0;
    let mut unit_data = ByteVec::new();
    let mut i = 0;
    let mut unit_dec = 0;
    let mut the_rest = false;
    let mut partition: u8 = 0;
    let mut finished = false;

    /* some devices have some units which exceeds sizeof uint16_t */
    let mut is_32bit = false;

    println!("Processing {}", ta_file.display());

    let file = File::open(ta_file).context("Unable to open file")?;

    let reader = BufReader::new(file);

    for res in reader.lines() {
        is_32bit = false;

        if let Ok(mut line) = res {
            match line.len() {
                0 => {
                    /*LOG("Skipped empty line.\n\n");*/
                },
                2 => {
                    let bytes = line.as_bytes();
                    if bytes[0] >= b'0' && bytes[0] <= b'9' && bytes[1] >= b'0' && bytes[1] <= b'9' {
                        partition = u8::from_str(line.as_str()).unwrap_or(0);
                        println!(" - Partition: {}", partition);
                    }
                },
                _ => {
                    if !line.starts_with("//") {
                        /*LOG("Retrieved line of lenght: %lu\n", read);*/

                        if check_valid_unit(&line) {
                            finished = true;

                            unit.extend_from_slice(&line[0..8].as_bytes());

                            unit_dec = unit.as_hexadecimal().unwrap_or(0);

                            println!(" - Unit: {} ({})", unit, unit_dec);

                            /*
                             * in case of 32 bit unit size!
                             * unit(8) + space(1) + unit size(8) + space(1)
                             */
                            if line.len() >= 18
                            {
                                if line.chars().nth(8).unwrap() == ' ' && line.chars().nth(17).unwrap() == ' '
                                {
                                    is_32bit = true;
                                }
                            }

                            trim_rs(&mut line);
                            /*LOG("Line lenght after trim: %lu\n", line.len());*/

                            if is_32bit
                            {
                                /* unit(8) + unit size(8) + at least one hex(2) */
                                if line.len() < 18
                                {
                                    if line.len() == 16
                                    {
                                        println!(" - Found specific unit which don't contain data.\n");
                                        the_rest = true;
                                        finished = true;
                                        unit_size = 0;
                                    } else {
                                        println!(" - Error: corrupted unit! Skipping this unit!\n\n");
                                        the_rest = false;
                                        continue;
                                    }
                                } else {
                                    let unit_size_str = &line[8..16];

                                    unit_size = u32::from_str_radix(unit_size_str, 16).with_context(|| format!("Error parsing unit size: {unit_size_str}"))?;

                                    println!(" - Unit size: 0x{}", unit_size_str);

                                    i = line.len();
                                    if i != 0 {
                                        unit_data.clear();
                                        unit_data.extend_from_slice(&line.as_bytes()[16..]);
                                    }
                                    // unit_data[i] = '\0';

                                    if line.len() - 16 < (unit_size * 2) as usize
                                    {
                                        /*LOG("Data probably continues in a new line (%u not match %u)!\n",
                                            (unsigned int)line.len()-16, unit_sz*2);*/
                                        the_rest = true;
                                    } else {
                                        the_rest = false;
                                    }

                                    if unit_data.len() == (unit_size * 2) as usize {
                                        finished = true;
                                    }
                                }
                            } else {
                                /* unit(8) + unit size(4) + at least one hex(2) */
                                if line.len() < 14
                                {
                                    if line.len() == 12
                                    {
                                        println!(" - Found specific unit which don't contain data.\n");
                                        the_rest = true;
                                        finished = true;
                                        unit_size = 0;
                                    } else {
                                        println!(" - Error: corrupted unit! Skipping this unit!\n\n");
                                        the_rest = false;
                                        continue;
                                    }
                                } else {
                                    let unit_sz_tmp = &line[8..12];

                                    unit_size = u32::from_str_radix(unit_sz_tmp, 16).with_context(|| format!("Error parsing unit size: {unit_sz_tmp}"))?;

                                    println!(" - Unit size: 0x{}", unit_sz_tmp);

                                    i = line.len();
                                    if i != 0 {
                                        unit_data.clear();
                                        unit_data.extend_from_slice(&line.as_bytes()[12..]);
                                    }
                                    // unit_data[i] = '\0';

                                    if line.len() - 12 < (unit_size * 2) as usize
                                    {
                                        /*LOG("Data probably continues in a new line (%u not match %u)!\n",
                                            (unsigned int)line.len()-12, unit_sz*2);*/
                                        the_rest = true;
                                    } else {
                                        the_rest = false;
                                    }

                                    if unit_data.len() == (unit_size * 2) as usize {
                                        finished = true;
                                    }
                                }
                            }
                        } else {
                            if the_rest
                            {
                                finished = false;
                                trim_rs(&mut line);
                                /*LOG("Line lenght after trim: %lu\n", line.len());
                                LOG("Found the rest ot the data!\n");*/
                                i = line.len();
                                if i != 0 {
                                    unit_data.extend_from_slice(&line.as_bytes()[0..]);
                                }
                                // unit_data[j + i] = '\0';

                                if unit_data.len() == (unit_size * 2) as usize
                                {
                                    the_rest = false;
                                    finished = true;
                                }
                            }
                        }

                        if finished {
                            let mut unit_total_temp: String;

                            /*
                                unit 0x7d3 (2003) hardware config
                                unit 0x7da (2010) simlock
                                unit 0x851 (2129) simlock signature
                                unit 0x1324 (4900) device id
                                unit 0x1046F (66671) google lock state ( allow bootloader unlock in dev settings )
                                unit 0x9A9 (2473) value 1 for enable serial console or value 0 (default) to disable (https://forum.xda-developers.com/showpost.php?p=80212371&postcount=1125)
                                unit 0x10471 (66673) protocol switch? Or keystore? What is this? Depend on existance of unit 0x36A (https://forum.xda-developers.com/showpost.php?p=80176195&postcount=1093)
                            */

                            if /*memcmp(unit, "000008B2", 8) == 0 || unlock key */
                            unit == b"000007D3" || /* hardware config */
                                unit == b"000007DA" || /* simlock */
                                unit == b"00000851" || /* simlock signature */
                                unit == b"000008A2" || /* device name */
                                unit == b"00001324" || /* device id */
                                unit == b"0001046B" {
                                /* drm key */
                                println!(" - Skipping unit {:x}", unit_dec);
                                continue;
                            }


                            finished = false;
                            /*LOG("\n<<-------------------- Retrieval finished! Found unit: %s,"
                                " Unit size: %04X, Unit data:%s\n",
                                 unit, unit_sz, unit_sz ? "" : " NULL");*/
                            // TODO:
                            unit_data = to_ascii_number(unit_data).into();

                            command = format!("download:{:08x}", unit_size).into();
                            println!("      {}", command);
                            println!("      DATA: {unit_data}");

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
                        }
                        /*LOG("\n");*/
                    }
                }
            }
        }
    };

    Ok(TrimArea{ partition, unit: unit_dec, data: unit_data })
}

pub fn check_valid_unit(line: &str) -> bool {
    if line.len() < 8 {
        return false;
    }

    line[0..8].chars().map(|c| c.is_ascii_alphanumeric()).all(|b| b)
}

pub fn to_ascii_number(hex: ByteVec) -> String {
    hex.as_hexadecimal().unwrap_or(0).to_string()
}