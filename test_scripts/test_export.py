#
# Copyright (c) Memfault, Inc.
# See License.txt for details
import os
from typing import Any

from .memfault_service_tester import MemfaultServiceTester
from .qemu import QEMU


def test_export_zip(
    qemu: QEMU, memfault_service_tester: MemfaultServiceTester, qemu_device_id: str
) -> None:
    qemu.exec_cmd("memfaultctl export -o test.zip")
    qemu.child().expect("Nothing to export right now.")

    # add something to mar staging
    qemu.exec_cmd("memfaultctl write-attributes testdevice=true")

    # now data should have been exported
    qemu.exec_cmd("memfaultctl export -o test.zip")
    qemu.child().expect("Export saved and data cleared")

    qemu.exec_cmd("unzip -l test.zip")
    qemu.child().expect("manifest.json")

    # exporting again should not generate any data
    qemu.exec_cmd("memfaultctl export -o test.zip")
    qemu.child().expect("Nothing to export right now.")


def test_export_chunk(
    qemu: QEMU, memfault_service_tester: MemfaultServiceTester, qemu_device_id: str
) -> None:
    qemu.exec_cmd("memfaultctl write-attributes export_works=true")

    qemu.exec_cmd("memfaultctl export -o test.bin -f chunk")
    qemu.child().expect("Export saved and data cleared")

    qemu.exec_cmd(
        f"curl -v -X POST https://chunks.memfault.com/api/v0/chunks/{ qemu_device_id } -H 'Memfault-Project-Key: {os.environ['MEMFAULT_PROJECT_KEY']}' -H 'Content-Type: application/octet-stream' --data-binary @test.bin"
    )
    qemu.child().expect("Accepted")

    # Wait until we have received attributes
    def _check() -> None:
        attributes: Any = memfault_service_tester.list_attributes(device_serial=qemu_device_id)

        assert attributes

        d = {
            a["custom_metric"]["string_key"]: a["state"]["value"]
            for a in attributes
            if a["state"] is not None
        }

        assert d["export_works"] is True

    memfault_service_tester.poll_until_not_raising(
        _check, timeout_seconds=5 * 60, poll_interval_seconds=1
    )
