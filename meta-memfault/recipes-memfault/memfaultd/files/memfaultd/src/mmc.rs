//
// Copyright (c) Memfault, Inc.
// See License.txt for details
use std::path::PathBuf;
use std::{path::Path, str::FromStr};

use eyre::{eyre, Report, Result};
use fs_extra::file::read_to_string;
use log::warn;
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
}

pub struct MmcImpl {
    device_path: PathBuf,
    disk_name: String,
    mmc_type: MmcType,
    sysfs_path: PathBuf,
    sysfs_device_path: PathBuf,
    lifetime_source: Option<LifetimeSource>,
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

        let lifetime_source = Self::is_lifetime_available(&sysfs_device_path);

        Ok(Self {
            device_path,
            disk_name,
            mmc_type,
            sysfs_path,
            sysfs_device_path,
            lifetime_source,
        })
    }

    /// Check if it's possible to get the lifetime for a given disk.
    ///
    /// This function tests two possible sources. For kernels above 4.19 we can look
    /// in sysfs. For anything else we can use the ioctl method to read the extcsd
    /// register.
    ///
    /// For the sysfs method, if we cannot find the file, we can assume that the
    /// JEDEC rev of the disk is too old to report lifetimes.
    fn is_lifetime_available(sysfs_device_path: &Path) -> Option<LifetimeSource> {
        if Self::kernel_supports_sysfs_lifetime() {
            Self::is_lifetime_available_sysfs(sysfs_device_path).then_some(LifetimeSource::Sysfs)
        } else {
            let jedec_rev = read_extcsd(sysfs_device_path)
                .ok()
                .map(|ext_csd| get_jedec_revision(&ext_csd));

            Self::is_lifetime_available_jedec(jedec_rev).then_some(LifetimeSource::ExtCsd)
        }
    }

    #[cfg(target_os = "linux")]
    fn kernel_supports_sysfs_lifetime() -> bool {
        use procfs::KernelVersion;

        let sysfs_lifetime_kernel_version = KernelVersion::new(4, 19, 0);
        let current_kernel_version = KernelVersion::current().ok();

        current_kernel_version.is_some_and(|current| current >= sysfs_lifetime_kernel_version)
    }

    #[cfg(not(target_os = "linux"))]
    fn kernel_supports_sysfs_lifetime() -> bool {
        false
    }

    fn is_lifetime_available_sysfs(sysfs_device_path: &Path) -> bool {
        let lifetime_path = sysfs_device_path.join("life_time");

        lifetime_path.exists()
    }

    fn is_lifetime_available_jedec(jedec_rev: Option<u8>) -> bool {
        match jedec_rev {
            Some(rev) => {
                // JEDEC revision 7 (5.0) is the minimum required for lifetime info
                if rev < 7 {
                    warn!("JEDEC spec before v5.0, lifetime values not available");
                    false
                } else {
                    true
                }
            }
            None => {
                warn!("Unable to read EXT_CSD register, lifetime values not available");
                false
            }
        }
    }
}

impl Mmc for MmcImpl {
    fn read_lifetime(&self) -> Result<Option<MmcLifeTime>> {
        if self.mmc_type != MmcType::Mmc {
            return Ok(None);
        }

        match self.lifetime_source {
            Some(LifetimeSource::ExtCsd) => {
                let bytes = read_extcsd(&self.device_path)?;

                MmcLifeTime::try_from(bytes).map(Some)
            }
            Some(LifetimeSource::Sysfs) => {
                let lifetime_path = self.sysfs_device_path.join("life_time");
                let lifetime_string = read_to_string(lifetime_path)?;
                let lifetime = MmcLifeTime::try_from(lifetime_string)?;

                Ok(Some(lifetime))
            }
            None => Ok(None),
        }
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

enum LifetimeSource {
    Sysfs,
    ExtCsd,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MmcLifeTime {
    pub lifetime_a_pct: Option<u8>,
    pub lifetime_b_pct: Option<u8>,
}

impl MmcLifeTime {
    const LIFETIME_A_OFFSET: usize = 268;
    const LIFETIME_B_OFFSET: usize = 269;

    pub const fn new(lifetime_a_pct: Option<u8>, lifetime_b_pct: Option<u8>) -> Self {
        Self {
            lifetime_a_pct,
            lifetime_b_pct,
        }
    }
}

impl TryFrom<[u8; EXT_CSD_SIZE]> for MmcLifeTime {
    type Error = Report;

    fn try_from(bytes: [u8; EXT_CSD_SIZE]) -> Result<Self, Self::Error> {
        let raw_lifetime_a = bytes[Self::LIFETIME_A_OFFSET];
        let raw_lifetime_b = bytes[Self::LIFETIME_B_OFFSET];

        let lifetime_a_pct = raw_lifetime_to_pct(raw_lifetime_a);
        let lifetime_b_pct = raw_lifetime_to_pct(raw_lifetime_b);

        Ok(Self::new(lifetime_a_pct, lifetime_b_pct))
    }
}

impl TryFrom<String> for MmcLifeTime {
    type Error = Report;

    fn try_from(value: String) -> Result<Self, Self::Error> {
        let tokens = value.split_whitespace().collect::<Vec<_>>();

        if tokens.len() != 2 {
            return Err(eyre!("Invalid number of lifetime values"));
        }

        let stripped_lifetime_a = tokens[0].strip_prefix("0x").unwrap_or(tokens[0]);
        let stripped_lifetime_b = tokens[1].strip_prefix("0x").unwrap_or(tokens[1]);

        let raw_lifetime_a = u8::from_str_radix(stripped_lifetime_a, 16)?;
        let raw_lifetime_b = u8::from_str_radix(stripped_lifetime_b, 16)?;

        let lifetime_a_pct = raw_lifetime_to_pct(raw_lifetime_a);
        let lifetime_b_pct = raw_lifetime_to_pct(raw_lifetime_b);

        Ok(Self::new(lifetime_a_pct, lifetime_b_pct))
    }
}

fn raw_lifetime_to_pct(raw_lifetime: u8) -> Option<u8> {
    if raw_lifetime > 0 && raw_lifetime <= 0xb {
        let lifetime_pct = (raw_lifetime - 1) * 10;
        Some(lifetime_pct)
    } else {
        None
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

const JEDEC_REV_OFFSET: usize = 192;

fn get_jedec_revision(ext_csd: &[u8; EXT_CSD_SIZE]) -> u8 {
    ext_csd[JEDEC_REV_OFFSET]
}

#[cfg(test)]
mod test {

    use rstest::rstest;
    use tempfile::TempDir;

    use crate::test_utils::create_file_with_contents;

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

        assert_eq!(lifetime.lifetime_a_pct, Some(0));
        assert_eq!(lifetime.lifetime_b_pct, Some(0));
    }

    #[rstest]
    #[case(String::from("4 5"), Ok(MmcLifeTime::new(Some(30), Some(40))))]
    #[case(String::from("4 0"), Ok(MmcLifeTime::new(Some(30), None)))]
    #[case(String::from("0 4"), Ok(MmcLifeTime::new(None, Some(30))))]
    #[case(String::from("4"), Err(eyre!("")))]
    #[case(String::from("4 4 4"), Err(eyre!("")))]
    #[case(String::from("4  5"), Ok(MmcLifeTime::new(Some(30), Some(40))))]
    #[case(String::from("4\t5"), Ok(MmcLifeTime::new(Some(30), Some(40))))]
    #[case(String::from("4 5\n"), Ok(MmcLifeTime::new(Some(30), Some(40))))]
    #[case(String::from("\t4 5"), Ok(MmcLifeTime::new(Some(30), Some(40))))]
    #[case(String::from("4 5"), Ok(MmcLifeTime::new(Some(30), Some(40))))]
    #[case(String::from("0x4 0x5"), Ok(MmcLifeTime::new(Some(30), Some(40))))]
    fn test_lifetime_from_sysfs_string(
        #[case] input_string: String,
        #[case] expected: Result<MmcLifeTime>,
    ) {
        let actual = MmcLifeTime::try_from(input_string);

        match (&actual, &expected) {
            (Ok(actual), Ok(expected)) => assert_eq!(actual, expected),
            (Ok(actual), Err(_)) => {
                panic!("Expected error, but conversion succeeded: {:?}", actual)
            }
            (Err(_), Ok(_)) => panic!("Expected success, but conversion failed"),
            (Err(_), Err(_)) => {}
        }
    }

    #[test]
    fn test_read_jedec_revision() {
        let jedec_rev = get_jedec_revision(&EXT_CSD_TEST_BUFFER);

        assert_eq!(jedec_rev, 8);
    }

    #[rstest]
    fn test_lifetime_fetch_type() {
        let mmc = MmcImpl {
            device_path: PathBuf::new(),
            disk_name: "disk".to_string(),
            mmc_type: MmcType::Sd,
            sysfs_path: PathBuf::new(),
            sysfs_device_path: PathBuf::new(),
            lifetime_source: Some(LifetimeSource::ExtCsd),
        };

        let lifetime = mmc.read_lifetime().unwrap();
        assert!(lifetime.is_none());
    }

    #[rstest]
    #[case(Some(6), false)]
    #[case(None, false)]
    #[case(Some(7), true)]
    #[case(Some(8), true)]
    fn test_lifetime_available_jedec(#[case] jedec_rev: Option<u8>, #[case] expected: bool) {
        let lifetime_available = MmcImpl::is_lifetime_available_jedec(jedec_rev);

        assert_eq!(lifetime_available, expected);
    }

    #[rstest]
    #[case(true)]
    #[case(false)]
    fn test_lifetime_available_sysfs(#[case] create_lifetime: bool) {
        let content = create_lifetime.then_some("");
        let tmp_dir = create_lifetime_dir(content);

        let actual = MmcImpl::is_lifetime_available_sysfs(tmp_dir.path());

        assert_eq!(actual, create_lifetime);
    }

    #[rstest]
    #[case(0, None)]
    #[case(1, Some(0))]
    #[case(0xb, Some(100))]
    #[case(0xc, None)]
    fn test_lifetime_raw_to_pct(#[case] raw_lifetime: u8, #[case] expected_pct: Option<u8>) {
        let lifetime_pct = raw_lifetime_to_pct(raw_lifetime);

        assert_eq!(lifetime_pct, expected_pct);
    }

    fn create_lifetime_dir(contents: Option<&str>) -> TempDir {
        let tmp_dir = TempDir::new().unwrap();
        let tmp_path = tmp_dir.path();
        let lifetime_path = tmp_path.join("life_time");
        if let Some(contents) = contents {
            create_file_with_contents(&lifetime_path, contents.as_bytes()).unwrap();
        }

        tmp_dir
    }
}
