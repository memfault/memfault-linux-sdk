#
# Copyright (c) Memfault, Inc.
# See License.txt for details
#!/usr/bin/python3

from qemu import QEMU


def get_rootfs_partition(qemu):
    qemu.child().sendline("cat /proc/cmdline")
    qemu.child().expect(".*root=(.+) .*")
    (rootfs,) = qemu.child().match.groups()
    qemu.child().expect("#")
    return rootfs.decode("utf-8", "ignore")


# Assumptions:
# - The machine/qemu is built with a valid project key of a project on app.memfault.com,
#   or whatever the underlying QEMU instance points at.
# - The project has a valid Release and OTA payload available for the machine.
def test_start():
    qemu = QEMU()
    qemu.child().expect(" login:")
    qemu.child().sendline("root")
    qemu.child().expect("#")

    # update will install in the background. we can monitor it!
    qemu.child().sendline("journalctl -u swupdate.service -f")
    qemu.child().expect("Installation in progress")
    qemu.child().expect("SWUPDATE successful !")
    qemu.child().expect("Update successful, executing post-update actions")

    # after the update installs the device will reboot
    qemu.child().expect("reboot: Restarting system")
    qemu.child().expect(" login:")


if __name__ == "__main__":
    test_start()
