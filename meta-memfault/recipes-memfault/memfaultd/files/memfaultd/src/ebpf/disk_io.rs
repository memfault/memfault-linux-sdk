//
// Copyright (c) Memfault, Inc.
// See License.txt for details
use std::{
    collections::HashMap,
    marker::PhantomData,
    path::{Path, PathBuf},
    str::FromStr,
};

use aya::{
    maps::{AsyncPerfEventArray, MapData},
    programs::TracePoint,
    util::online_cpus,
    Ebpf,
};
use bytes::BytesMut;
use eyre::{eyre, Context, Error, Result};
use ssf::{Service, TaskService};
use tokio::{
    fs::{read_link, read_to_string},
    sync::mpsc::{channel, Receiver, Sender},
    task::JoinHandle,
};

use crate::{
    ebpf_programs::DISK_IO,
    metrics::{KeyedMetricReading, MetricStringKey},
};
use crate::{metrics::MetricsMBox, util::system::ProcessNameMapper};

pub struct DiskIo<P: ProcessNameMapper> {
    _ebpf: Ebpf,
    metrics_mbox: MetricsMBox,
    perf_array: AsyncPerfEventArray<MapData>,
    event_tx: Sender<DiskIoEvent>,
    event_rx: Receiver<DiskIoEvent>,
    dev_name_cache: DevNameCache,
    _marker: PhantomData<P>,
}

impl<P: ProcessNameMapper> DiskIo<P> {
    const DISK_IO_CHANNEL_SIZE: usize = 1024;
    const EVENT_RX_MAX: usize = 8;

    pub fn load(metrics_mbox: MetricsMBox) -> Result<Self> {
        let mut ebpf = Ebpf::load(DISK_IO)?;
        let prog: &mut TracePoint = ebpf
            .program_mut("handle_block_io_start")
            .expect("Wrong program type")
            .try_into()?;
        prog.load()?;
        prog.attach("block", "block_io_start")?;

        let perf_array: AsyncPerfEventArray<_> = ebpf
            .take_map("DISK_EVENTS")
            .ok_or_else(|| eyre!("Failed to fetch perf map"))?
            .try_into()?;

        let (event_tx, event_rx) = channel(Self::DISK_IO_CHANNEL_SIZE);

        Ok(Self {
            _ebpf: ebpf,
            metrics_mbox,
            perf_array,
            event_tx,
            event_rx,
            dev_name_cache: DevNameCache::default(),
            _marker: PhantomData,
        })
    }

    pub async fn init(&mut self) -> Result<()> {
        let cpus = online_cpus().map_err(|e| eyre!("Failed to get number of cpus: {:?}", e))?;

        for cpu in cpus {
            let mut perf_array = self.perf_array.open(cpu, None)?;
            let event_tx = self.event_tx.clone();

            let _handle: JoinHandle<std::result::Result<(), Error>> = tokio::spawn(async move {
                // TODO: Experiment with buffer size/count. It's unclear exactly how we should size this
                let mut buffers = (0..16)
                    .map(|_| BytesMut::with_capacity(128))
                    .collect::<Vec<_>>();

                loop {
                    // TODO: Error handling is inadequate here. These futures will be cancelled currently
                    // leading to missed perf events
                    perf_array
                        .read_events(&mut buffers)
                        .await
                        .wrap_err("Failed to read events from perf buffer")?;

                    for buf in &mut buffers {
                        if buf.is_empty() {
                            continue;
                        }

                        let slice = &buf[0..size_of::<DiskIoEvent>()];

                        // SAFETY: Cast here is safe because the struct consists only of scalar values
                        let event: &DiskIoEvent =
                            unsafe { &*(slice.as_ptr() as *const DiskIoEvent) };

                        event_tx
                            .send(*event)
                            .await
                            .wrap_err("Failed to send perf event")?;

                        buf.clear();
                    }
                }
            });
        }

        Ok(())
    }

    pub async fn run_once(&mut self) -> Result<()> {
        let mut events = Vec::with_capacity(Self::EVENT_RX_MAX);
        let num_events = self
            .event_rx
            .recv_many(&mut events, Self::EVENT_RX_MAX)
            .await;
        if num_events == 0 {
            return Err(eyre!("Disk IO events channel dropped"));
        }

        let mut readings = Vec::with_capacity(num_events);
        for event in &events[..num_events] {
            if let Some(reading) = self.disk_io_event_to_metric(event).await {
                readings.push(reading);
            }
        }

        if !readings.is_empty() {
            self.metrics_mbox.send_and_forget(readings)?;
        }

        Ok(())
    }

    async fn disk_io_event_to_metric(&mut self, event: &DiskIoEvent) -> Option<KeyedMetricReading> {
        let proc_name = P::get_process_name(event.pid).ok()?;
        let dev_name = self.dev_name_cache.get(event.dev).await;
        let disk_op = DiskOp::try_from(event.rwbs.as_slice()).ok();

        build_metric_reading(dev_name, disk_op, &proc_name, event.bytes)
    }
}

impl<P: ProcessNameMapper> TaskService for DiskIo<P> {
    fn run_task(&mut self) -> futures::future::LocalBoxFuture<'_, std::result::Result<(), String>> {
        Box::pin(async {
            self.run_once()
                .await
                .map_err(|e| format!("Failed to fetch disk I/O metrics: {}", e))
        })
    }

    fn init(&mut self) -> futures::future::LocalBoxFuture<'_, std::result::Result<(), String>> {
        Box::pin(async {
            self.init()
                .await
                .map_err(|e| format!("Failed to initialize disk I/O metric reader: {}", e))
        })
    }
}

impl<P: ProcessNameMapper> Service for DiskIo<P> {
    fn name(&self) -> &str {
        "DiskIo"
    }
}

enum DiskOp {
    Read,
    Write,
}

impl TryFrom<&[u8]> for DiskOp {
    type Error = ();

    fn try_from(value: &[u8]) -> std::result::Result<Self, Self::Error> {
        let op_string = String::from_utf8_lossy(value);
        if op_string.contains('W') {
            Ok(Self::Write)
        } else if op_string.contains('R') {
            Ok(Self::Read)
        } else {
            Err(())
        }
    }
}

#[repr(C)]
#[derive(Debug, Default, Clone, Copy)]
struct DiskIoEvent {
    pid: u32,
    bytes: u32,
    dev: u32,
    rwbs: [u8; 8],
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

fn build_metric_reading(
    dev_name: Option<&str>,
    disk_op: Option<DiskOp>,
    proc_name: &str,
    bytes: u32,
) -> Option<KeyedMetricReading> {
    let metric_key_string = match (dev_name, disk_op) {
        (Some(dev_name), Some(DiskOp::Write)) => Some(format!(
            "diskstats/{}/{}/bytes_written",
            dev_name, proc_name
        )),

        (Some(dev_name), Some(DiskOp::Read)) => {
            Some(format!("diskstats/{}/{}/bytes_read", dev_name, proc_name))
        }

        (None, _) | (_, None) => None,
    }?;

    MetricStringKey::from_str(&metric_key_string)
        .ok()
        .map(|key| KeyedMetricReading::new_counter(key, bytes as f64))
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
        Some("sda"),
        Some(DiskOp::Write),
        "test_proc",
        1024,
        "diskstats/sda/test_proc/bytes_written",
        1024.0
    )]
    #[case(
        Some("sda"),
        Some(DiskOp::Read),
        "test_proc",
        2048,
        "diskstats/sda/test_proc/bytes_read",
        2048.0
    )]
    fn test_build_metric_reading(
        #[case] dev_name: Option<&str>,
        #[case] disk_op: Option<DiskOp>,
        #[case] proc_name: &str,
        #[case] bytes: u32,
        #[case] expected_str: &'static str,
        #[case] expected_val: f64,
    ) {
        let result = build_metric_reading(dev_name, disk_op, proc_name, bytes)
            .expect("Metric reading failed");
        assert_eq!(result.name.as_str(), expected_str);
        match result.value {
            crate::metrics::MetricReading::Counter { value, .. } => assert_eq!(value, expected_val),
            _ => panic!("Unexpected metric value type"),
        }
    }

    #[rstest]
    #[case(None, None, "test_proc", 1024)]
    #[case(Some("sda"), None, "test_proc", 1024)]
    #[case(None, Some(DiskOp::Write), "test_proc", 1024)]
    #[case(None, None, "test_proc", 0)]
    fn test_build_metric_reading_failure(
        #[case] dev_name: Option<&str>,
        #[case] disk_op: Option<DiskOp>,
        #[case] proc_name: &str,
        #[case] bytes: u32,
    ) {
        let result = build_metric_reading(dev_name, disk_op, proc_name, bytes);
        assert!(result.is_none());
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
