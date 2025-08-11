//
// Copyright (c) Memfault, Inc.
// See License.txt for details
use std::fs::read_to_string;

use eyre::{eyre, Result};
use nom::bytes::complete::take_while1;
use nom::{
    character::complete::{digit1, line_ending, space1},
    combinator::{map, map_res, opt},
    multi::many0,
    sequence::{terminated, tuple},
    IResult,
};

use crate::metrics::{
    system_metrics::{SystemMetricFamilyCollector, MEMORY_METRIC_NAMESPACE},
    KeyedMetricReading, MetricStringKey,
};
use crate::util::time_measure::TimeMeasure;

const PROC_VMSTAT_PATH: &str = "/proc/vmstat";

#[derive(Debug, Copy, Clone)]
pub struct SwapStats<T>
where
    T: TimeMeasure + Copy,
{
    pub pswpin: u64,
    pub pswpout: u64,
    pub pgpgin: u64,
    pub pgpgout: u64,
    pub reading_time: T,
}

pub struct VmMetricsCollector<T>
where
    T: TimeMeasure + Copy,
{
    last_reading: Option<SwapStats<T>>,
}

impl<T> VmMetricsCollector<T>
where
    T: TimeMeasure + Copy,
{
    pub fn new() -> Self {
        Self { last_reading: None }
    }

    /// Parse a single vmstat line like "pswpin 12345"
    fn parse_vmstat_line(input: &str) -> IResult<&str, (&str, u64)> {
        let (input, (key, _, value)) = tuple((
            // Parse the key name
            take_while1(|c: char| c.is_alphanumeric() || c == '_'),
            // Skip whitespace
            space1,
            // Parse the numeric value
            map_res(digit1, |s: &str| s.parse::<u64>()),
        ))(input)?;

        Ok((input, (key, value)))
    }

    /// Parse a complete line including optional line ending
    fn parse_line(input: &str) -> IResult<&str, Option<(&str, u64)>> {
        map(terminated(Self::parse_vmstat_line, opt(line_ending)), Some)(input)
    }

    /// Parser that extracts swap stats from /proc/vmstat
    pub fn parse_swap_stats(input: &str) -> IResult<&str, SwapStats<T>> {
        let (input, lines) = many0(Self::parse_line)(input)?;

        let mut pswpin = 0u64;
        let mut pswpout = 0u64;
        let mut pgpgout = 0u64;
        let mut pgpgin = 0u64;

        for line in lines.into_iter().flatten() {
            match line.0 {
                "pswpin" => pswpin = line.1,
                "pswpout" => pswpout = line.1,
                "pgpgout" => pgpgout = line.1,
                "pgpgin" => pgpgin = line.1,
                _ => {}
            }
        }

        Ok((
            input,
            SwapStats {
                pswpin,
                pswpout,
                pgpgin,
                pgpgout,
                reading_time: T::now(),
            },
        ))
    }

    fn calculate_vm_metrics(&mut self, vmstat_content: &str) -> Result<Vec<KeyedMetricReading>> {
        let mut vm_metrics = vec![];
        let (_, current_stats) =
            Self::parse_swap_stats(vmstat_content).map_err(|e| eyre!("Parse error: {}", e))?;
        if let Some(prev) = self.last_reading.replace(current_stats) {
            let interval_secs =
                (current_stats.reading_time.since(&prev.reading_time)).as_secs_f64();

            if current_stats.pswpin >= prev.pswpin {
                let pswpin_rate = (current_stats.pswpin - prev.pswpin) as f64 / interval_secs;
                vm_metrics.push(KeyedMetricReading::new_histogram(
                    MetricStringKey::from("memory/vm/swaps_in_per_second"),
                    pswpin_rate,
                ));
            }
            if current_stats.pswpout >= prev.pswpout {
                let pswpout_rate = (current_stats.pswpout - prev.pswpout) as f64 / interval_secs;
                vm_metrics.push(KeyedMetricReading::new_histogram(
                    MetricStringKey::from("memory/vm/swaps_out_per_second"),
                    pswpout_rate,
                ));
            }

            if current_stats.pgpgin >= prev.pgpgin {
                let pgpgin_rate = (current_stats.pgpgin - prev.pgpgin) as f64 / interval_secs;
                vm_metrics.push(KeyedMetricReading::new_histogram(
                    MetricStringKey::from("memory/vm/pages_in_per_second"),
                    pgpgin_rate,
                ));
            }
            if current_stats.pgpgout >= prev.pgpgout {
                let pgpgout_rate = (current_stats.pgpgout - prev.pgpgout) as f64 / interval_secs;
                vm_metrics.push(KeyedMetricReading::new_histogram(
                    MetricStringKey::from("memory/vm/pages_out_per_second"),
                    pgpgout_rate,
                ));
            }
        }

        Ok(vm_metrics)
    }

    pub fn get_vm_metrics(&mut self) -> Result<Vec<KeyedMetricReading>> {
        let vmstat_content = read_to_string(PROC_VMSTAT_PATH)?;

        let vm_metrics = self.calculate_vm_metrics(&vmstat_content)?;

        Ok(vm_metrics)
    }
}

impl<T> SystemMetricFamilyCollector for VmMetricsCollector<T>
where
    T: TimeMeasure + Copy,
{
    fn collect_metrics(&mut self) -> Result<Vec<KeyedMetricReading>> {
        self.get_vm_metrics()
    }

    fn family_name(&self) -> &'static str {
        MEMORY_METRIC_NAMESPACE
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use insta::assert_json_snapshot;

    use super::*;
    use crate::test_utils::TestInstant;

    #[test]
    fn test_parse_vmstat_line() {
        let input = "pswpin 12345";
        let result = VmMetricsCollector::<TestInstant>::parse_vmstat_line(input);
        assert_eq!(result, Ok(("", ("pswpin", 12345))));

        let input2 = "pswpout 67890";
        let result2 = VmMetricsCollector::<TestInstant>::parse_vmstat_line(input2);
        assert_eq!(result2, Ok(("", ("pswpout", 67890))));

        let input3 = "nr_alloc_batch 67890";
        let result3 = VmMetricsCollector::<TestInstant>::parse_vmstat_line(input3);
        assert_eq!(result3, Ok(("", ("nr_alloc_batch", 67890))));
    }

    #[test]
    fn test_parse_swap_stats() {
        let vmstat_content = r#"nr_free_pages 123456
nr_alloc_batch 789
nr_inactive_anon 1000
nr_active_anon 2000
pswpin 42
pswpout 84
pgpgin 5000
pgpgout 6000
other_stat 999"#;

        let result = VmMetricsCollector::<TestInstant>::parse_swap_stats(vmstat_content);
        assert!(result.is_ok());

        let (_, stats) = result.unwrap();
        assert_eq!(stats.pswpin, 42);
        assert_eq!(stats.pswpout, 84);
    }

    #[test]
    fn test_real_vmstat_format() {
        // Test with actual /proc/vmstat format (no guaranteed order)
        let vmstat_content = "nr_free_pages 245760\nnr_zone_inactive_anon 12345\npswpout 156\nnr_zone_active_anon 67890\npswpin 78\npgpgin 123456\n";

        let result = VmMetricsCollector::<TestInstant>::parse_swap_stats(vmstat_content);
        assert!(result.is_ok());

        let (_, stats) = result.unwrap();
        assert_eq!(stats.pswpin, 78);
        assert_eq!(stats.pswpout, 156);
    }

    #[test]
    fn test_vm_metrics_calculation() {
        let mut vm_metrics_collector = VmMetricsCollector::<TestInstant>::new();

        let vmstat_content1 = r#"nr_free_pages 123456
nr_alloc_batch 789
nr_inactive_anon 1000
nr_active_anon 2000
pswpin 42
pswpout 84
pgpgin 5000
pgpgout 6000
other_stat 999"#;

        let metrics1 = vm_metrics_collector
            .calculate_vm_metrics(vmstat_content1)
            .unwrap();

        // No metrics should be calculated from first reading
        assert!(metrics1.is_empty());

        TestInstant::sleep(Duration::from_secs(10));

        let vmstat_content2 = r#"nr_free_pages 123456
nr_alloc_batch 789
nr_inactive_anon 1000
nr_active_anon 2000
pswpin 47
pswpout 94
pgpgin 5100
pgpgout 6200
other_stat 999"#;
        let metrics2 = vm_metrics_collector
            .calculate_vm_metrics(vmstat_content2)
            .unwrap();
        assert_json_snapshot!(metrics2,
                              {"[].value.**.timestamp" => "[timestamp]"});

        TestInstant::sleep(Duration::from_secs(10));

        let vmstat_content3 = r#"nr_free_pages 123456
nr_alloc_batch 789
nr_inactive_anon 1000
nr_active_anon 2000
pswpin 100
pswpout 120
pgpgin 5000
pgpgout 6000
other_stat 999"#;
        let metrics3 = vm_metrics_collector
            .calculate_vm_metrics(vmstat_content3)
            .unwrap();
        assert_json_snapshot!(metrics3,
                              {"[].value.**.timestamp" => "[timestamp]"});

        TestInstant::sleep(Duration::from_secs(10));

        let vmstat_content4 = r#"nr_free_pages 123456
nr_alloc_batch 789
nr_inactive_anon 1000
nr_active_anon 2000
pswpin 100
pswpout 120
pgpgin 5000
pgpgout 6000
other_stat 999"#;
        let metrics4 = vm_metrics_collector
            .calculate_vm_metrics(vmstat_content4)
            .unwrap();
        assert_json_snapshot!(metrics4,
                              {"[].value.**.timestamp" => "[timestamp]"});
    }
}
