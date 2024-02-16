#
# Copyright (c) Memfault, Inc.
# See License.txt for details
import time
from typing import Any

import pytest

from .memfault_service_tester import MemfaultServiceTester
from .qemu import QEMU


# Assumptions:
# - The machine/qemu is built with a valid project key of a project on app.memfault.com,
#   or whatever the underlying QEMU instance points at.
# - The MEMFAULT_E2E_* environment variables are set to match whatever the underlying
#   QEMU instance points at.
def test(qemu: QEMU, memfault_service_tester: MemfaultServiceTester, qemu_device_id: str) -> None:
    # Write attributes
    qemu.exec_cmd(
        'memfaultctl write-attributes a_string=running a_bool=false a_boolish_string=\\"true\\" a_float=42.42'
    )

    # Wait for MAR to get written
    time.sleep(5)

    # Poke memfaultd to upload now
    qemu.exec_cmd("memfaultctl sync")

    # Wait until we have received attributes
    def _check() -> None:
        attributes: Any = memfault_service_tester.list_attributes(device_serial=qemu_device_id)

        assert attributes

        d = {
            a["custom_metric"]["string_key"]: a["state"]["value"]
            for a in attributes
            if a["state"] is not None
        }

        assert d["a_string"] == "running"
        assert d["a_bool"] is False
        assert d["a_boolish_string"] == "true"
        assert d["a_float"] == 42.42

    memfault_service_tester.poll_until_not_raising(
        _check, timeout_seconds=2 * 60, poll_interval_seconds=1
    )


@pytest.mark.parametrize("data_collection_enabled", [False])
def test_fails_with_data_collection_disabled(qemu: QEMU, data_collection_enabled: bool) -> None:
    qemu.exec_cmd("memfaultctl write-attributes foo=bar")
    qemu.child().expect("Cannot write attributes because data collection is disabled")
