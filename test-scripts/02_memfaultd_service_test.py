#
# Copyright (c) Memfault, Inc.
# See License.txt for details
#!/usr/bin/python3

from qemu import QEMU


def test_start():
    qemu = QEMU()
    qemu.child().expect(" login:")
    qemu.child().sendline("root")
    qemu.child().expect("#")
    qemu.child().sendline("memfaultd -h")
    qemu.child().expect("Usage: memfaultd \\[OPTION\\]...")


if __name__ == "__main__":
    test_start()
