#
# Copyright (c) Memfault, Inc.
# See License.txt for details
import pytest
from qemu import QEMU


@pytest.fixture()
def qemu_device_id() -> str:
    # TODO: Remove this fixture once https://memfault.myjetbrains.com/youtrack/issue/MFLT-6943 has been deployed!
    return "qemu-tester"


# Assumptions:
# - The machine/qemu is built with a valid project key of a project on app.memfault.com,
#   or whatever the underlying QEMU instance points at.
# - The project has a valid Release and OTA payload available for the machine.
def test_start(qemu: QEMU):
    qemu.child().expect(" login:")
    qemu.child().sendline("root")
    qemu.child().expect("#")

    # update will install in the background. we can monitor it!
    qemu.child().sendline("journalctl -u swupdate.service -f")
    qemu.child().expect("Installation in progress", timeout=120)
    qemu.child().expect("SWUPDATE successful !")
    qemu.child().expect("Update successful, executing post-update actions")

    # after the update installs the device will reboot
    qemu.child().expect("reboot: Restarting system")
    qemu.child().expect(" login:")
