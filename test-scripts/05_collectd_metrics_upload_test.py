#
# Copyright (c) Memfault, Inc.
# See License.txt for details
import pytest
from memfault_service_tester import MemfaultServiceTester
from qemu import QEMU


@pytest.fixture()
def memfault_extra_config() -> object:
    return {"enable_data_collection": True, "collectd_plugin": {"interval_seconds": 5}}


# Assumptions:
# - The machine/qemu is built with a valid project key of a project on app.memfault.com,
#   or whatever the underlying QEMU instance points at.
# - The MEMFAULT_E2E_* environment variables are set to match whatever the underlying
#   QEMU instance points at.
def test(
    qemu: QEMU, memfault_service_tester: MemfaultServiceTester, qemu_device_id: str
):
    qemu.systemd_wait_for_service_state("collectd.service", "active")

    def _check():
        reports = memfault_service_tester.list_reports(
            dict(device_serial=qemu_device_id),
            ignore_errors=True,
        )
        assert reports
        # Note: sometimes the first heartbeat is an empty dict:
        assert any((report["metrics"] for report in reports))

    memfault_service_tester.poll_until_not_raising(
        _check, timeout_seconds=60, poll_interval_seconds=1
    )
