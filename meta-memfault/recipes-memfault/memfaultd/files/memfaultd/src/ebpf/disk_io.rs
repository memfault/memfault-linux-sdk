//
// Copyright (c) Memfault, Inc.
// See License.txt for details
use std::{
    collections::{HashMap, HashSet},
    marker::PhantomData,
    path::{Path, PathBuf},
    str::FromStr,
    time::Duration,
};

use aya::{
    maps::{MapData, PerCpuArray, PerCpuHashMap},
    programs::TracePoint,
    Ebpf, Pod,
};
use eyre::{eyre, Result};
use ssf::{Service, TaskService};
use tokio::{
    fs::{read_link, read_to_string},
    time::{interval, Interval, MissedTickBehavior},
};

use crate::{
    ebpf_programs::DISK_IO,
    metrics::{KeyedMetricReading, MetricStringKey, MetricsMBox},
    util::system::ProcessNameMapper,
};

const DISK_OP_READ: u32 = 0;
const DISK_OP_WRITE: u32 = 1;

// Bounded, to keep memory flat when many short-lived processes come and go
// in a single sampling interval. Normal growth is pruned every run_once via
// retain(), so this cap is a safety net rather than a steady-state target.
const PROC_NAME_CACHE_CAPACITY: usize = 256;
const MAP_FLUSH_DURATION: Duration = Duration::from_secs(10);

#[repr(C)]
#[derive(Debug, Default, Clone, Copy, Eq, PartialEq, Hash)]
struct DiskIoKey {
    tgid: u32,
    dev: u32,
    op: u32,
}

// SAFETY: DiskIoKey has no padding (three u32 fields) and is plain-old-data.
// Layout matches struct disk_io_key in ebpf/disk_io.c.
unsafe impl Pod for DiskIoKey {}

pub struct DiskIo<P: ProcessNameMapper> {
    _ebpf: Ebpf,
    metrics_mbox: MetricsMBox,
    stats: PerCpuHashMap<MapData, DiskIoKey, u64>,
    drops: PerCpuArray<MapData, u64>,
    last_drops: u64,
    dev_name_cache: DevNameCache,
    proc_name_cache: ProcNameCache,
    flush_interval: Option<Interval>,
    _marker: PhantomData<P>,
}

impl<P: ProcessNameMapper> DiskIo<P> {
    pub fn load(metrics_mbox: MetricsMBox) -> Result<Self> {
        let mut ebpf = Ebpf::load(DISK_IO)?;
        let prog: &mut TracePoint = ebpf
            .program_mut("handle_block_io_start")
            .expect("Wrong program type")
            .try_into()?;
        prog.load()?;
        prog.attach("block", "block_io_start")?;

        let stats: PerCpuHashMap<_, DiskIoKey, u64> = ebpf
            .take_map("DISK_IO_STATS")
            .ok_or_else(|| eyre!("Failed to fetch DISK_IO_STATS map"))?
            .try_into()?;

        let drops: PerCpuArray<_, u64> = ebpf
            .take_map("DISK_IO_DROPS")
            .ok_or_else(|| eyre!("Failed to fetch DISK_IO_DROPS map"))?
            .try_into()?;

        Ok(Self {
            _ebpf: ebpf,
            metrics_mbox,
            stats,
            drops,
            last_drops: 0,
            dev_name_cache: DevNameCache::default(),
            proc_name_cache: ProcNameCache::new(PROC_NAME_CACHE_CAPACITY),
            flush_interval: None,
            _marker: PhantomData,
        })
    }

    pub async fn run_once(&mut self) -> Result<()> {
        let keys: Vec<DiskIoKey> = self.stats.keys().filter_map(|k| k.ok()).collect();

        let mut readings = Vec::with_capacity(keys.len() + 1);
        let mut seen_tgids = HashSet::with_capacity(keys.len());

        for key in &keys {
            let total: u64 = match self.stats.get(key, 0) {
                Ok(values) => values.iter().sum(),
                Err(_) => continue,
            };
            // Delete so the next interval restarts at zero for this key.
            // Any bytes counted between get() and remove() are lost, an
            // acceptable microsecond-wide undercount.
            let _ = self.stats.remove(key);

            if total == 0 {
                continue;
            }

            seen_tgids.insert(key.tgid);

            let Some(proc_name) = self.proc_name_cache.get_or_resolve::<P>(key.tgid) else {
                continue;
            };
            let Some(dev_name) = self.dev_name_cache.get(key.dev).await.map(str::to_string) else {
                continue;
            };

            if let Some(reading) = build_metric_reading(&dev_name, key.op, &proc_name, total) {
                readings.push(reading);
            }
        }

        // Drops are cumulative on the kernel side; emit the delta since our
        // last read so the metric behaves like a normal counter increment.
        let drops_sum: u64 = self.drops.get(&0, 0).map(|v| v.iter().sum()).unwrap_or(0);
        let drop_delta = drops_sum.saturating_sub(self.last_drops);
        self.last_drops = drops_sum;
        if drop_delta > 0 {
            if let Ok(key) = MetricStringKey::from_str("diskstats/dropped_events") {
                readings.push(KeyedMetricReading::new_counter(key, drop_delta as f64));
            }
        }

        self.proc_name_cache.retain(&seen_tgids);

        if !readings.is_empty() {
            self.metrics_mbox.send_and_forget(readings)?;
        }

        Ok(())
    }
}

impl<P: ProcessNameMapper> TaskService for DiskIo<P> {
    fn run_task(&mut self) -> futures::future::LocalBoxFuture<'_, std::result::Result<(), String>> {
        Box::pin(async {
            // Only run task on fixed interval
            self.flush_interval
                .as_mut()
                .expect("Disk IO flush interval not present")
                .tick()
                .await;

            self.run_once()
                .await
                .map_err(|e| format!("Failed to collect disk I/O metrics: {}", e))
        })
    }

    fn init(&mut self) -> futures::future::LocalBoxFuture<'_, std::result::Result<(), String>> {
        Box::pin(async {
            let mut flush_interval = interval(MAP_FLUSH_DURATION);
            flush_interval.set_missed_tick_behavior(MissedTickBehavior::Skip);
            self.flush_interval = Some(flush_interval);

            Ok(())
        })
    }
}

impl<P: ProcessNameMapper> Service for DiskIo<P> {
    fn name(&self) -> &str {
        "DiskIo"
    }
}

fn build_metric_reading(
    dev_name: &str,
    op: u32,
    proc_name: &str,
    bytes: u64,
) -> Option<KeyedMetricReading> {
    let metric_key_string = match op {
        DISK_OP_READ => format!("diskstats/{}/{}/bytes_read", dev_name, proc_name),
        DISK_OP_WRITE => format!("diskstats/{}/{}/bytes_written", dev_name, proc_name),
        _ => return None,
    };

    MetricStringKey::from_str(&metric_key_string)
        .ok()
        .map(|key| KeyedMetricReading::new_counter(key, bytes as f64))
}

#[derive(Debug, Default)]
struct DevNameCache {
    map: HashMap<u32, String>,
}

impl DevNameCache {
    async fn get(&mut self, dev: u32) -> Option<&str> {
        if self.map.contains_key(&dev) {
            return self.map.get(&dev).map(|s| s.as_str());
        }

        if let Some(device_string) =
            base_block_device_name(dev_major(dev), dev_minor(dev), "/sys/dev/block").await
        {
            self.map.insert(dev, device_string);
            self.map.get(&dev).map(|s| s.as_str())
        } else {
            None
        }
    }
}

struct ProcNameCache {
    entries: HashMap<u32, String>,
    capacity: usize,
}

impl ProcNameCache {
    fn new(capacity: usize) -> Self {
        Self {
            entries: HashMap::new(),
            capacity,
        }
    }

    fn get_or_resolve<P: ProcessNameMapper>(&mut self, pid: u32) -> Option<String> {
        if let Some(name) = self.entries.get(&pid) {
            return Some(name.clone());
        }

        let name = P::get_process_name(pid).ok()?;

        if self.entries.len() >= self.capacity {
            self.entries.clear();
        }

        self.entries.insert(pid, name.clone());
        Some(name)
    }

    fn retain(&mut self, seen: &HashSet<u32>) {
        self.entries.retain(|pid, _| seen.contains(pid));
    }
}

/// Gets the friendly device name from major/minor pair.
///
/// This function does a bit of magic to grab the friendly device name from
/// the sysfs path. If a partition is passed, we want to get the actual device
/// name. For the non-symlinked path, this is the parent directory. To get this
/// in those cases we need to do the following:
///     1. Resolve the symlink
///     2. If partition, go up one directory
///     3. Extract the device name from the uevent entry
async fn base_block_device_name(major_num: u32, minor_num: u32, root_dir: &str) -> Option<String> {
    let sys_block_link = PathBuf::from(format!("{}/{}:{}", root_dir, major_num, minor_num));

    if !sys_block_link.exists() {
        return None;
    }

    // Resolve the symlink to the real sysfs path
    let real_path = read_link(&sys_block_link)
        .await
        .ok()
        .map(|p| Path::new(root_dir).join(p))?;

    // If it's a partition, step up one level (from sda1 → sda)
    let target_path = if real_path.join("partition").exists() {
        real_path.parent()?.to_path_buf()
    } else {
        real_path
    };

    // Read uevent file
    let uevent_path = target_path.join("uevent");
    if let Ok(content) = read_to_string(uevent_path).await {
        for line in content.lines() {
            if let Some(name) = line.strip_prefix("DEVNAME=") {
                return Some(name.to_string());
            }
        }
    }

    None
}

fn dev_major(dev: u32) -> u32 {
    dev >> 20
}

fn dev_minor(dev: u32) -> u32 {
    dev & 0xFFFFF
}

#[cfg(test)]
mod test {
    use super::*;

    use rstest::rstest;
    use tempfile::TempDir;
    use tokio::fs::{create_dir_all, symlink, write};

    #[rstest]
    #[case(0x00A0000B, 10)]
    #[case(0x00A00003, 10)]
    #[case(0x00000000, 0)]
    #[case(0xFFF00000, 4095)]
    #[case(0x00000FFFFF, 0)]
    fn test_dev_major(#[case] dev: u32, #[case] expected: u32) {
        assert_eq!(dev_major(dev), expected);
    }

    #[rstest]
    #[case(0x00A0000B, 11)]
    #[case(0x00A00003, 3)]
    #[case(0x00000000, 0)]
    #[case(0xFFF00000, 0)]
    #[case(0x00000FFFFF, 1048575)]
    fn test_dev_minor(#[case] dev: u32, #[case] expected: u32) {
        assert_eq!(dev_minor(dev), expected);
    }

    #[rstest]
    #[tokio::test]
    async fn test_block_device_name() {
        let tmp_dir = build_test_dir().await;
        let dev_name =
            base_block_device_name(8, 0, tmp_dir.path().join("sys/dev/block").to_str().unwrap())
                .await;
        assert_eq!(dev_name, Some("sda".to_string()));
    }

    #[rstest]
    #[tokio::test]
    async fn test_block_device_name_partition() {
        let tmp_dir = build_test_dir().await;
        let dev_name =
            base_block_device_name(8, 1, tmp_dir.path().join("sys/dev/block").to_str().unwrap())
                .await;
        assert_eq!(dev_name, Some("sda".to_string()));
    }

    #[rstest]
    #[tokio::test]
    async fn test_block_device_not_found() {
        let tmp_dir = build_test_dir().await;
        let dev_name =
            base_block_device_name(8, 2, tmp_dir.path().join("sys/dev/block").to_str().unwrap())
                .await;
        assert_eq!(dev_name, None);
    }

    #[rstest]
    #[case(
        "sda",
        DISK_OP_WRITE,
        "test_proc",
        1024,
        "diskstats/sda/test_proc/bytes_written",
        1024.0
    )]
    #[case(
        "sda",
        DISK_OP_READ,
        "test_proc",
        2048,
        "diskstats/sda/test_proc/bytes_read",
        2048.0
    )]
    fn test_build_metric_reading(
        #[case] dev_name: &str,
        #[case] op: u32,
        #[case] proc_name: &str,
        #[case] bytes: u64,
        #[case] expected_str: &'static str,
        #[case] expected_val: f64,
    ) {
        let result =
            build_metric_reading(dev_name, op, proc_name, bytes).expect("Metric reading failed");
        assert_eq!(result.name.as_str(), expected_str);
        match result.value {
            crate::metrics::MetricReading::Counter { value, .. } => assert_eq!(value, expected_val),
            _ => panic!("Unexpected metric value type"),
        }
    }

    #[rstest]
    #[case("sda", 99, "test_proc", 1024)]
    fn test_build_metric_reading_unknown_op(
        #[case] dev_name: &str,
        #[case] op: u32,
        #[case] proc_name: &str,
        #[case] bytes: u64,
    ) {
        assert!(build_metric_reading(dev_name, op, proc_name, bytes).is_none());
    }

    #[test]
    fn test_proc_name_cache_retain_drops_unseen_pids() {
        struct MockMapper;
        impl ProcessNameMapper for MockMapper {
            fn get_process_name(pid: u32) -> Result<String> {
                Ok(format!("proc_{pid}"))
            }
        }

        let mut cache = ProcNameCache::new(8);
        cache.get_or_resolve::<MockMapper>(10);
        cache.get_or_resolve::<MockMapper>(20);
        cache.get_or_resolve::<MockMapper>(30);
        assert_eq!(cache.entries.len(), 3);

        let seen: HashSet<u32> = [10, 30].into_iter().collect();
        cache.retain(&seen);
        assert_eq!(cache.entries.len(), 2);
        assert!(cache.entries.contains_key(&10));
        assert!(cache.entries.contains_key(&30));
        assert!(!cache.entries.contains_key(&20));
    }

    #[test]
    fn test_proc_name_cache_clears_at_capacity() {
        struct MockMapper;
        impl ProcessNameMapper for MockMapper {
            fn get_process_name(pid: u32) -> Result<String> {
                Ok(format!("proc_{pid}"))
            }
        }

        let mut cache = ProcNameCache::new(2);
        cache.get_or_resolve::<MockMapper>(1);
        cache.get_or_resolve::<MockMapper>(2);
        assert_eq!(cache.entries.len(), 2);
        cache.get_or_resolve::<MockMapper>(3);
        assert_eq!(cache.entries.len(), 1);
        assert!(cache.entries.contains_key(&3));
    }

    async fn build_test_dir() -> TempDir {
        // Create a temp dir with the following structure:
        // sys/block/sda
        // sys/block/sda1
        // sys/dev/block/8:0 -> ../../block/sda
        // sys/dev/block/8:1 -> ../../block/sda1

        let tmp_dir = tempfile::tempdir().unwrap();

        let sys_block_sda = tmp_dir.path().join("sys/block/sda");
        let uevent_path = sys_block_sda.join("uevent");
        create_dir_all(&sys_block_sda).await.unwrap();
        write(&uevent_path, b"DEVNAME=sda").await.unwrap();

        let sys_block_sda1 = sys_block_sda.join("sda1");
        let partition_path = sys_block_sda1.join("partition");
        create_dir_all(&sys_block_sda1).await.unwrap();
        write(&partition_path, b"1").await.unwrap();

        let sys_dev_block = tmp_dir.path().join("sys/dev/block");
        let symlink_partition_path = sys_dev_block.join("8:1");
        create_dir_all(&sys_dev_block).await.unwrap();
        symlink(&sys_block_sda1, &symlink_partition_path)
            .await
            .unwrap();

        let symlink_device_path = sys_dev_block.join("8:0");
        symlink(&sys_block_sda, &symlink_device_path).await.unwrap();

        tmp_dir
    }
}
