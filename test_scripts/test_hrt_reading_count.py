#
# Copyright (c) Memfault, Inc.
# See License.txt for details
from time import sleep
from typing import cast

from .memfault_service_tester import MemfaultServiceTester
from .qemu import QEMU

# The internal metric emitted by memfaultd (see internal_metrics.rs) that reports
# the number of readings in each flushed HRT report.
HRT_READING_COUNT_METRIC = "MemfaultSdkMetric_hrt_reading_count"


# Assumptions:
# - The machine/qemu is built with a valid project key of a project on app.memfault.com,
#   or whatever the underlying QEMU instance points at.
# - The MEMFAULT_E2E_* environment variables are set to match whatever the underlying
#   QEMU instance points at.
# - High resolution telemetry is enabled (it is enabled by default in builtin.conf).
#
# This exercises the full device -> MAR -> server path for the HRT reading-count
# metric. Note the metric is reported on the *heartbeat* report (a metric report,
# returned by list_reports), never inside the HRT payload itself - so seeing it here
# also confirms it lands in the aggregated report path rather than in HRT. The exact
# count and the exclusion-from-HRT property are covered by the Rust unit tests in
# metric_report_manager.rs.
def test(qemu: QEMU, memfault_service_tester: MemfaultServiceTester, qemu_device_id: str) -> None:
    # Run collectd at 1hz so HRT accumulates readings quickly.
    qemu.exec_cmd("sed -ie 's/Interval .*/Interval 1/' /etc/collectd.conf")
    qemu.exec_cmd("systemctl restart collectd")

    # Wait for collectd to start (it starts after memfaultd).
    qemu.systemd_wait_for_service_state("collectd.service", "active")

    def _check() -> None:
        # Force a sync inside the poll loop so we sync more than once. The HRT flush
        # stages the reading-count metric into the heartbeat *after* that same sync's
        # heartbeat dump, so it takes a subsequent sync for the count to be uploaded.
        qemu.exec_cmd("memfaultctl sync")
        sleep(1)

        reports = memfault_service_tester.list_reports(
            {"device_serial": qemu_device_id},
            ignore_errors=True,
        )
        assert reports
        # A heartbeat report should eventually carry a positive HRT reading count.
        assert any(
            (cast("dict[str, float]", report["metrics"]).get(HRT_READING_COUNT_METRIC) or 0) > 0
            for report in reports
        )

    memfault_service_tester.poll_until_not_raising(_check, poll_interval_seconds=1)
