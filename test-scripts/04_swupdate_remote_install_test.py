#
# Copyright (c) Memfault, Inc.
# See License.txt for details
import pytest
from qemu import QEMU


@pytest.fixture()
def swupdate_enabled() -> bool:
    return True


# Assumptions:
# - The machine/qemu is built with a valid project key of a project on app.memfault.com,
#   or whatever the underlying QEMU instance points at.
# - The project has a valid Release and OTA payload available for the machine.
def test_start(qemu: QEMU):
    # update will install in the background. we can monitor it!
    qemu.child().sendline("journalctl -u swupdate.service -f")
    qemu.child().expect("Installation in progress", timeout=120)
    qemu.child().expect("SWUPDATE successful !")
    qemu.child().expect("Update successful, executing post-update actions")

    # after the update installs the device will reboot
    qemu.child().expect("reboot: Restarting system")
    qemu.child().expect(" login:")
