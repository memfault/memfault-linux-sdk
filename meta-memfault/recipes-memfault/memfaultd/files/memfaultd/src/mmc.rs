//
// Copyright (c) Memfault, Inc.
// See License.txt for details
use std::path::PathBuf;
use std::{path::Path, str::FromStr};

use eyre::{eyre, Report, Result};
use fs_extra::file::read_to_string;
use nix::fcntl::{open, OFlag};
use nix::ioctl_readwrite;
use nix::sys::stat::Mode;
use nix::unistd::close;

// All values fetched from kernel source:
// https://elixir.bootlin.com/linux/v6.13.7/source/include/linux/mmc/core.h#L35
const MMC_SEND_EXT_CSD: u32 = 8;
const MMC_RSP_PRESENT: u32 = 1 << 0;
const MMC_RSP_CRC: u32 = 1 << 2;
const MMC_RSP_OPCODE: u32 = 1 << 4;
const MMC_RSP_R1: u32 = MMC_RSP_PRESENT | MMC_RSP_CRC | MMC_RSP_OPCODE;
const MMC_RSP_SPI_R1: u32 = 1 << 7;
const MMC_CMD_ADTC: u32 = 1 << 5;

const EXT_CSD_SIZE: usize = 512;

pub trait Mmc {
    fn read_lifetime(&self) -> Result<Option<MmcLifeTime>>;
    fn product_name(&self) -> Result<String>;
    fn manufacturer_id(&self) -> Result<String>;
    fn disk_name(&self) -> &str;
    fn disk_sector_count(&self) -> Result<u64>;
    fn manufacture_date(&self) -> Result<String>;
    fn revision(&self) -> Result<String>;
    fn serial(&self) -> Result<String>;
    fn disk_type(&self) -> MmcType;
}

pub struct MmcImpl {
    device_path: PathBuf,
    disk_name: String,
    mmc_type: MmcType,
    sysfs_path: PathBuf,
    sysfs_device_path: PathBuf,
}

impl MmcImpl {
    pub fn new(device_path: PathBuf) -> Result<Self> {
        let disk_name = device_path
            .file_name()
            .ok_or_else(|| eyre!("Invalid device path, must point to a file"))?
            .to_str()
            .ok_or_else(|| eyre!("Invalid disk name"))?
            .to_string();

        let sysfs_path_string = format!("/sys/block/{}", &disk_name);
        let sysfs_path = PathBuf::from(sysfs_path_string);
        let sysfs_device_path = sysfs_path.join("device");

        if !sysfs_device_path.exists() {
            return Err(eyre!("{} is not a valid block device", &disk_name));
        }

        let type_path = sysfs_device_path.join("type");
        let mmc_type = read_to_string(type_path)
            .map_err(|e| eyre!("Failed to read MMC type string: {}", e))
            .and_then(|type_string| MmcType::from_str(type_string.trim()))?;

        Ok(Self {
            device_path,
            disk_name,
            mmc_type,
            sysfs_path,
            sysfs_device_path,
        })
    }
}

impl Mmc for MmcImpl {
    fn read_lifetime(&self) -> Result<Option<MmcLifeTime>> {
        if self.mmc_type != MmcType::Mmc {
            return Ok(None);
        }

        let bytes = read_extcsd(&self.device_path)?;

        MmcLifeTime::try_from(bytes).map(Some)
    }

    fn product_name(&self) -> Result<String> {
        let product_name_path = self.sysfs_device_path.join("name");
        Ok(read_to_string(product_name_path)?.trim().to_string())
    }

    fn manufacturer_id(&self) -> Result<String> {
        let product_name_path = self.sysfs_device_path.join("manfid");
        Ok(read_to_string(product_name_path)?.trim().to_string())
    }

    fn disk_name(&self) -> &str {
        &self.disk_name
    }

    fn disk_type(&self) -> MmcType {
        self.mmc_type
    }

    fn disk_sector_count(&self) -> Result<u64> {
        let size_path = self.sysfs_path.join("size");
        let sector_count_string = read_to_string(size_path)?;

        Ok(sector_count_string.trim().parse::<u64>()?)
    }

    fn manufacture_date(&self) -> Result<String> {
        let manf_date = self.sysfs_device_path.join("date");
        let manf_date_string = read_to_string(manf_date)?;

        Ok(manf_date_string.trim().to_string())
    }

    fn revision(&self) -> Result<String> {
        let revision_path = self.sysfs_device_path.join("rev");
        let revision_string = read_to_string(revision_path)?;

        Ok(revision_string.trim().to_string())
    }

    fn serial(&self) -> Result<String> {
        let serial_path = self.sysfs_device_path.join("serial");
        let serial_string = read_to_string(serial_path)?;

        Ok(serial_string.trim().to_string())
    }
}

#[derive(Debug, Clone)]
pub struct MmcLifeTime {
    pub lifetime_a_pct: u8,
    pub lifetime_b_pct: u8,
}

impl MmcLifeTime {
    const LIFETIME_A_OFFSET: usize = 268;
    const LIFETIME_B_OFFSET: usize = 269;
}

impl TryFrom<[u8; EXT_CSD_SIZE]> for MmcLifeTime {
    type Error = Report;

    fn try_from(bytes: [u8; EXT_CSD_SIZE]) -> Result<Self, Self::Error> {
        let raw_lifetime_a = bytes[Self::LIFETIME_A_OFFSET];
        let raw_lifetime_b = bytes[Self::LIFETIME_B_OFFSET];

        if raw_lifetime_a == 0 || raw_lifetime_b == 0 {
            return Err(eyre!("Invalid lifetime value read"));
        }

        let lifetime_a_pct = (raw_lifetime_a - 1) * 10;
        let lifetime_b_pct = (raw_lifetime_b - 1) * 10;

        Ok(Self {
            lifetime_a_pct,
            lifetime_b_pct,
        })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MmcType {
    Mmc,
    Sd,
}

impl FromStr for MmcType {
    type Err = Report;

    fn from_str(s: &str) -> std::result::Result<Self, Self::Err> {
        match s {
            "MMC" => Ok(Self::Mmc),
            "SD" => Ok(Self::Sd),
            _ => Err(eyre!("Invalid MMC type string")),
        }
    }
}

#[repr(C)]
pub struct MmcIocCmd {
    write_flag: i32,
    is_acmd: i32,
    opcode: u32,
    arg: u32,
    response: [u32; 4],
    flags: u32,
    blksz: u32,
    blocks: u32,
    postsleep_min_us: u32,
    postsleep_max_us: u32,
    data_timeout_ns: u32,
    cmd_timeout_ms: u32,
    pad: u32,
    data_ptr: u64,
}

const MMC_IOC_MAGIC: u8 = 0xB3;
const MMC_IOC_CMD: u8 = 0x0;

ioctl_readwrite!(mmc_ioc_cmd_read, MMC_IOC_MAGIC, MMC_IOC_CMD, MmcIocCmd);

fn read_extcsd(device_path: &Path) -> Result<[u8; EXT_CSD_SIZE]> {
    let fd = match open(device_path, OFlag::O_RDWR, Mode::empty()) {
        Ok(fd) => fd,
        Err(e) => {
            return Err(eyre!("Failed to open device: {}", e));
        }
    };

    let mut buf = [0u8; EXT_CSD_SIZE];
    let mut cmd = MmcIocCmd {
        write_flag: 0,
        is_acmd: 0,
        opcode: MMC_SEND_EXT_CSD,
        arg: 0,
        response: [0; 4],
        flags: MMC_RSP_SPI_R1 | MMC_RSP_R1 | MMC_CMD_ADTC,
        blksz: EXT_CSD_SIZE as u32,
        blocks: 1,
        postsleep_min_us: 0,
        postsleep_max_us: 0,
        data_timeout_ns: 0,
        cmd_timeout_ms: 0,
        pad: 0,
        data_ptr: buf.as_mut_ptr() as u64,
    };

    match unsafe { mmc_ioc_cmd_read(fd, &mut cmd) } {
        Ok(_) => {
            let _ = close(fd);

            Ok(buf)
        }
        Err(e) => {
            let _ = close(fd);
            Err(eyre!("ioctl failed: {}", e))
        }
    }
}

#[cfg(test)]
mod test {

    use rstest::rstest;

    use super::*;

    // Captured return of EXT_CSD register from real MMC
    const EXT_CSD_TEST_BUFFER: [u8; EXT_CSD_SIZE] = [
        0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 8, 3, 0, 0, 103, 7, 0, 0, 0, 0, 0, 0, 0, 0,
        0, 0, 0, 1, 1, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
        10, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 3, 0, 0, 0, 0, 0, 0, 0, 0, 0,
        0, 0, 0, 1, 0, 2, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
        2, 0, 0, 1, 0, 0, 0, 0, 0, 0, 1, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
        0, 0, 0, 0, 0, 0, 0, 71, 14, 0, 7, 1, 1, 2, 0, 0, 21, 31, 32, 0, 0, 0, 0, 0, 0, 1, 0, 0, 0,
        72, 0, 0, 0, 0, 1, 3, 0, 13, 0, 0, 0, 0, 8, 0, 2, 0, 87, 31, 10, 3, 221, 221, 0, 0, 0, 10,
        10, 10, 10, 10, 10, 1, 0, 0, 103, 7, 23, 18, 23, 7, 8, 16, 1, 3, 1, 8, 32, 0, 7, 166, 166,
        85, 3, 0, 0, 0, 0, 221, 221, 0, 1, 90, 8, 0, 0, 0, 0, 25, 25, 0, 16, 0, 0, 221, 49, 56, 50,
        48, 51, 98, 49, 57, 37, 80, 8, 8, 8, 1, 1, 1, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
        0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 31, 1, 0, 0, 0, 0, 0, 0,
        0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
        0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
        0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
        0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
        0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
        0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 1, 0, 0, 0, 0, 16, 0, 3, 3,
        0, 5, 3, 3, 1, 63, 63, 1, 1, 1, 0, 0, 0, 0, 0, 0, 0,
    ];

    #[test]
    fn test_lifetime_from_extcsd() {
        let lifetime = MmcLifeTime::try_from(EXT_CSD_TEST_BUFFER).unwrap();

        assert_eq!(lifetime.lifetime_a_pct, 0);
        assert_eq!(lifetime.lifetime_b_pct, 0);
    }

    #[rstest]
    fn test_lifetime_fetch_type() {
        let mmc = MmcImpl {
            device_path: PathBuf::new(),
            disk_name: "disk".to_string(),
            mmc_type: MmcType::Sd,
            sysfs_path: PathBuf::new(),
            sysfs_device_path: PathBuf::new(),
        };

        let lifetime = mmc.read_lifetime().unwrap();
        assert!(lifetime.is_none());
    }
}
