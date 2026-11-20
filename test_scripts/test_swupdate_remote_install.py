#
# Copyright (c) Memfault, Inc.
# See License.txt for details
import pytest

from . import runqemu
from .qemu import QEMU


@pytest.fixture()
def swupdate_enabled() -> bool:
    return True


# Assumptions:
# - The machine/qemu is built with a valid project key of a project on app.memfault.com,
#   or whatever the underlying QEMU instance points at.
# - The project has a valid Release and OTA payload available for the machine.
def test_start(qemu: QEMU) -> None:
    # update will install in the background. we can monitor it!
    qemu.child().sendline("journalctl -u swupdate.service -f")
    qemu.child().expect("Installation in progress", timeout=120)

    # after the update installs the device will reboot
    qemu.child().expect("reboot: Restarting system", timeout=240)
    qemu.login()
    qemu.exec_cmd_ok(
        f"grep -q root=/dev/vda{runqemu.qemu_get_system_partition_b_index()} /proc/cmdline"
    )
