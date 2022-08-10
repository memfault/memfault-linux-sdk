#
# Copyright (c) Memfault, Inc.
# See License.txt for details
#!/usr/bin/python3

import pexpect
from qemu import QEMU


def test_start():
    qemu = QEMU()
    qemu.child().expect(" login:")
    qemu.child().sendline("root")
    qemu.child().expect("#")

    qemu.child().sendline("memfaultd --enable-data-collection")
    qemu.child().expect("Enabling data collection")

    qemu.child().sendline("memfaultd --enable-data-collection")
    qemu.child().expect("Data collection state already set")

    qemu.child().sendline("memfaultd --disable-data-collection")
    qemu.child().expect("Disabling data collection")

    qemu.child().sendline("memfaultd --disable-data-collection")
    qemu.child().expect("Data collection state already set")

    # Check that the service restarted:
    qemu.child().sendline("journalctl -u memfaultd.service")
    qemu.child().expect("Stopped memfaultd daemon")
    qemu.child().expect("Started memfaultd daemon")

    # Check that the service did not fail:
    # FIXME: pexpect does not have a "not expecting" API :/
    qemu.child().sendline("journalctl -u memfaultd.service")
    output = b""
    while True:
        try:
            output += qemu.child().read_nonblocking(size=1024, timeout=0.5)
        except pexpect.TIMEOUT:
            break
    assert b"memfaultd.service: Failed" not in output


if __name__ == "__main__":
    test_start()
