#
# Copyright (c) Memfault, Inc.
# See License.txt for details
import os
import time

from .memfault_service_tester import MemfaultServiceTester
from .qemu import QEMU


def test_export_zip(
    qemu: QEMU, memfault_service_tester: MemfaultServiceTester, qemu_device_id: str
) -> None:
    qemu.exec_cmd("memfaultctl export -o test.zip")
    qemu.child().expect("Nothing to export right now.")

    # add something to mar staging
    qemu.exec_cmd("memfaultctl trigger-coredump")

    # Wait for MAR to be written
    time.sleep(5)

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
    qemu.exec_cmd("memfaultctl trigger-coredump")

    # Wait for MAR to be written
    time.sleep(5)

    qemu.exec_cmd("memfaultctl export -o test.bin -f chunk")
    qemu.child().expect("Export saved and data cleared")

    qemu.exec_cmd(
        f"curl -v -X POST https://chunks.memfault.com/api/v0/chunks/{qemu_device_id} -H 'Memfault-Project-Key: {os.environ['MEMFAULT_PROJECT_KEY']}' -H 'Content-Type: application/octet-stream' --data-binary @test.bin"
    )
    qemu.child().expect("Accepted")

    # Wait until we have received a coredump
    memfault_service_tester.poll_elf_coredumps_until_count(1, device_serial=qemu_device_id)
