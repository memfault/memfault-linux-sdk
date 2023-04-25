#
# Copyright (c) Memfault, Inc.
# See License.txt for details
from memfault_service_tester import MemfaultServiceTester
from qemu import QEMU


# Assumptions:
# - The machine/qemu is built with a valid project key of a project on app.memfault.com,
#   or whatever the underlying QEMU instance points at.
# - The MEMFAULT_E2E_* environment variables are set to match whatever the underlying
#   QEMU instance points at.
def test_customer_reboot_reason_user_reset(
    qemu: QEMU, memfault_service_tester: MemfaultServiceTester, qemu_device_id: str
):
    # Stream memfaultd's log
    qemu.exec_cmd("journalctl --follow --unit=memfaultd.service &")

    # 6 == kMfltRebootReason_ButtonReset
    # /media/last_reboot_reason is the default reboot_plugin.last_reboot_reason_file file path
    qemu.exec_cmd("echo 6 > /media/last_reboot_reason")

    qemu.exec_cmd("reboot")
    qemu.child().expect("reboot: Restarting system")
    qemu.child().expect(" login:")

    events = memfault_service_tester.poll_reboot_events_until_count(
        2, device_serial=qemu_device_id, timeout_secs=60
    )
    assert events
    assert events[-1]["reason"] == 6  # Button Reset
