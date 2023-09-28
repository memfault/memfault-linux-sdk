#
# Copyright (c) Memfault, Inc.
# See License.txt for details
import time

import pytest
from memfault_service_tester import MemfaultServiceTester
from qemu import QEMU


# Assumptions:
# - The machine/qemu is built with a valid project key of a project on app.memfault.com,
#   or whatever the underlying QEMU instance points at.
# - The MEMFAULT_E2E_* environment variables are set to match whatever the underlying
#   QEMU instance points at.
def test(
    qemu: QEMU, memfault_service_tester: MemfaultServiceTester, qemu_device_id: str
):
    # Poke memfaultd to sync - This will force memfaultd to receive the device
    # config and send a 'device-config' mar entry to confirm the version.
    qemu.exec_cmd("memfaultctl sync")

    # Make sure device has updated the reported config revision
    def _check():
        device = memfault_service_tester.get_device(device_serial=qemu_device_id)
        assert device
        assert device["reported_config_revision"] == device["assigned_config_revision"]

    memfault_service_tester.poll_until_not_raising(
        _check, timeout_seconds=60, poll_interval_seconds=1
    )


@pytest.mark.parametrize("data_collection_enabled", [False])
def test_fails_with_data_collection_disabled(
    qemu: QEMU,
    data_collection_enabled: bool,
    memfault_service_tester: MemfaultServiceTester,
    qemu_device_id: str,
):
    # Poke memfaultd to sync - This will force memfaultd to receive the device
    # config and send a 'device-config' mar entry to confirm the version.
    qemu.exec_cmd("memfaultctl sync")

    time.sleep(5)

    try:
        memfault_service_tester.get_device(device_serial=qemu_device_id)
        # Device should not exist which makes get-device fail
        assert False
    except Exception as e:
        assert "404" in str(e)
