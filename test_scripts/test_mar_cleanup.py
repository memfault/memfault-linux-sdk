#
# Copyright (c) Memfault, Inc.
# See License.txt for details
import re
import time

import pytest

from .qemu import QEMU

storage_max_usage_kib = 5


@pytest.fixture()
def memfault_extra_config() -> object:
    return {
        "enable_data_collection": True,
        "tmp_dir_max_usage_kib": storage_max_usage_kib,
        "logs": {"rotate_size_kib": 1},
    }


def test_mar_staging_is_cleaned_upon_log_rotation(qemu: QEMU) -> None:
    qemu.exec_cmd("for i in `seq 10`; do (printf %1024s |tr ' ' 'A' | systemd-cat); done")

    # Wait a bit for logs to be processed by memfaultd:
    time.sleep(2)

    # The MAR staging area should not exceed storage_max_usage_kib:
    qemu.exec_cmd(
        "ls -lAR /media/memfault/mar| grep -v '^d' | awk '{total += $5} END {print total}'"
    )
    qemu.child().expect(re.compile(rb"(\d+)"), timeout=1)
    match = qemu.child().match
    assert match
    assert int(match.group(1)) <= storage_max_usage_kib * 1024, match  # pyright: ignore[reportAttributeAccessIssue]
