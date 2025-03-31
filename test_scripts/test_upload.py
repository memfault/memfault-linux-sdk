#
# Copyright (c) Memfault, Inc.
# See License.txt for details
import time

import pytest

from .memfault_service_tester import MemfaultServiceTester
from .qemu import QEMU


@pytest.fixture()
def memfault_extra_config() -> object:
    return {
        "enable_data_collection": True,
    }


def test_upload_does_not_dump_data(qemu: QEMU) -> None:
    # Force an upload
    # This should only upload data and not serialize any additional
    # data
    qemu.exec_cmd("memfaultctl upload")

    # Make sure a MAR entry was NOT written
    qemu.exec_cmd("grep -l linux-metric-report /media/memfault/mar/*/*")
    qemu.child().expect("No such file or directory", timeout=3)


def test_upload_still_uploads_mar_entries(
    qemu: QEMU, memfault_service_tester: MemfaultServiceTester, qemu_device_id: str
) -> None:
    # Stream memfaultd's log and wait for memfaultd to start
    qemu.exec_cmd("journalctl --follow --unit=memfaultd.service &")
    qemu.child().expect("Started memfaultd daemon")

    # Stream logs from kernel: memfault-core-handler logs to dmsg
    qemu.exec_cmd("journalctl --follow -t kernel &")

    # Trigger the coredump
    qemu.exec_cmd("memfaultctl trigger-coredump")

    # Wait for coredump to be captured by memfault-core-handler:
    qemu.child().expect("Successfully captured coredump")

    # Wait for MAR to be written
    time.sleep(5)

    # Ensure memfaultd has transmitted the corefile
    qemu.exec_cmd("memfaultctl upload")

    # Check that the backend created the coredump:
    memfault_service_tester.poll_elf_coredumps_until_count(1, device_serial=qemu_device_id)
