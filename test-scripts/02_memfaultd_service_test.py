#
# Copyright (c) Memfault, Inc.
# See License.txt for details
from qemu import QEMU


def test_start(qemu: QEMU):
    qemu.child().expect(" login:")
    qemu.child().sendline("root")
    qemu.child().expect("#")
    qemu.child().sendline("memfaultd -h")
    qemu.child().expect("Usage: memfaultd \\[OPTION\\]...")
