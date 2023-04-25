#
# Copyright (c) Memfault, Inc.
# See License.txt for details
import re

from qemu import QEMU


def test_version(qemu: QEMU):
    for cmd in [
        "memfaultctl --version",
        "memfaultctl -v",
        "memfaultd --version",
        "memfaultd -v",
    ]:
        qemu.exec_cmd(cmd)
        # Note: "dev" should fail the test:
        qemu.child().expect(re.compile(rb"VERSION=\d+\.\d+\.\d+.*\n"), timeout=1)
        # Note: "unknown" should fail the test:
        qemu.child().expect(re.compile(rb"GIT COMMIT=[0-9a-f]{6,9}\s*\n"), timeout=1)
        # Note: "unknown" should fail the test:
        qemu.child().expect(re.compile(rb"BUILD ID=[0-9]+\s*\n"), timeout=1)
