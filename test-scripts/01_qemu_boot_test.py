#
# Copyright (c) Memfault, Inc.
# See License.txt for details
from qemu import QEMU


def test_start(qemu: QEMU):
    qemu.child().expect(" login:")