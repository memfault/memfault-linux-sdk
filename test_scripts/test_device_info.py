#
# Copyright (c) Memfault, Inc.
# See License.txt for details
import os
import pathlib
import time

import pytest

from .qemu import QEMU


@pytest.fixture()
def memfault_device_info(tmpdir: pathlib.Path) -> pathlib.Path:
    fn = tmpdir / "memfault-device-info"
    with open(fn, "w") as f:
        # write an empty file so that it fails when being called simulating no `memfault-device-info`
        f.write("")
    os.chmod(fn, 0o755)  # noqa: S103
    return fn


def test_defaults(qemu: QEMU) -> None:
    """
    Test that the defaults are set without any error
    """
    # Call memfaultctl show-settings to verify the device info is set
    qemu.exec_cmd("memfaultctl show-settings")

    time.sleep(1)

    qemu.child().expect("Device configuration from memfault-device-info:")
    qemu.child().expect("  MEMFAULT_DEVICE_ID=")
