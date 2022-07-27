#
# Copyright (c) Memfault, Inc.
# See License.txt for details
#!/usr/bin/python3

from qemu import QEMU


def test_start():
    qemu = QEMU()
    qemu.child().expect(" login:")


if __name__ == "__main__":
    test_start()
