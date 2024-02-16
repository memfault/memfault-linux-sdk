#
# Copyright (c) Memfault, Inc.
# See License.txt for details
from time import sleep

from .memfault_service_tester import MemfaultServiceTester
from .qemu import QEMU


# Assumptions:
# - The machine/qemu is built with a valid project key of a project on app.memfault.com,
#   or whatever the underlying QEMU instance points at.
# - The MEMFAULT_E2E_* environment variables are set to match whatever the underlying
#   QEMU instance points at.
def test_metrics_sync(
    qemu: QEMU, memfault_service_tester: MemfaultServiceTester, qemu_device_id: str
) -> None:
    # Run collectd at 1hz
    qemu.exec_cmd("sed -ie 's/Interval .*/Interval 1/' /etc/collectd.conf")
    qemu.exec_cmd("systemctl restart collectd")

    # Wait for collectd to start (it starts after memfaultd)
    qemu.systemd_wait_for_service_state("collectd.service", "active")

    def _check() -> None:
        # Force a sync - We do this in the _check loop so we attempt more than once
        qemu.exec_cmd("memfaultctl sync")
        sleep(1)

        reports = memfault_service_tester.list_reports(
            {"device_serial": qemu_device_id},
            ignore_errors=True,
        )
        assert reports
        # Note: sometimes the first heartbeat is an empty dict:
        assert any(report["metrics"] for report in reports)

    memfault_service_tester.poll_until_not_raising(
        _check, timeout_seconds=60, poll_interval_seconds=1
    )


def test_write_on_exit(qemu: QEMU) -> None:
    # Run collectd at 1hz
    qemu.exec_cmd("sed -ie 's/Interval .*/Interval 1/' /etc/collectd.conf")
    qemu.exec_cmd("systemctl restart collectd")

    # Wait for collectd to start (it starts after memfaultd)
    qemu.systemd_wait_for_service_state("collectd.service", "active")

    # Wait to capture some metrics
    sleep(4)

    # Shutdown
    qemu.exec_cmd("systemctl stop memfaultd")

    # Make sure a MAR entry was written
    qemu.exec_cmd("grep -l linux-metric-report /media/memfault/mar/*/*")
    qemu.child().expect("manifest.json", timeout=3)
