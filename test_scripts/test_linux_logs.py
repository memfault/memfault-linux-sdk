#
# Copyright (c) Memfault, Inc.
# See License.txt for details
import re
import time
import uuid

from .memfault_service_tester import MemfaultServiceTester
from .qemu import QEMU


# Assumptions:
# - The machine/qemu is built with a valid project key of a project on app.memfault.com,
#   or whatever the underlying QEMU instance points at.
# - The MEMFAULT_E2E_* environment variables are set to match whatever the underlying
#   QEMU instance points at.
def test_logs(
    qemu: QEMU,
    memfault_service_tester: MemfaultServiceTester,
    qemu_device_id: str,
) -> None:
    # Push a custom message to the systemd journal
    test_msg = f"test-{uuid.uuid4()}"
    qemu.exec_cmd(f"echo {test_msg} | systemd-cat")

    time.sleep(5)

    # Poke memfaultd to upload now
    qemu.exec_cmd("memfaultctl sync")

    # Wait until the logfile has been uploaded to the cloud
    def _check() -> None:
        logs = memfault_service_tester.log_files_get_list(device_serial=qemu_device_id)
        assert len(logs) > 0

    memfault_service_tester.poll_until_not_raising(_check, poll_interval_seconds=1)

    # Now download the log file and check the content
    logs = memfault_service_tester.log_files_get_list(device_serial=qemu_device_id)
    cid = logs[0]["cid"]
    assert isinstance(cid, str)
    log = memfault_service_tester.log_file_download(device_serial=qemu_device_id, cid=cid)

    # First kernel log message
    assert "Booting Linux on physical CPU 0x0" in log, log
    # Last kernel log message
    assert "Run /sbin/init as init process" in log, log

    # First systemd message
    assert re.search(r"systemd .* running in system mode", log), log

    # Some log from memfaultd
    assert "Base configuration (/etc/memfaultd.conf)" in log, log
    # Our test message
    assert test_msg in log, log
