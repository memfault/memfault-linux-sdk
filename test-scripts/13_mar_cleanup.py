#
# Copyright (c) Memfault, Inc.
# See License.txt for details
import json
import re
import time

from qemu import QEMU


def test_mar_staging_is_cleaned_upon_log_rotation(qemu: QEMU):
    storage_max_usage_kib = 5
    runtime_conf = {
        "mar": {"storage_max_usage_kib": storage_max_usage_kib},
        "logs": {"rotate_size_kib": 1},
    }

    qemu.exec_cmd(f"echo '{json.dumps(runtime_conf)}' > /media/memfault/runtime.conf")
    qemu.exec_cmd("memfaultctl enable-data-collection")

    qemu.systemd_wait_for_service_state("memfaultd.service", "active")

    # Push 10 * 1K lines to the systemd journal
    for _ in range(10):
        test_msg = "A" * 1024
        qemu.exec_cmd(f"echo {test_msg} | systemd-cat")

    # Wait a bit for logs to be processed by memfaultd:
    time.sleep(2)

    # The MAR staging area should not exceed storage_max_usage_kib:
    qemu.exec_cmd("du -s /media/memfault/mar_staging")
    qemu.child().expect(re.compile(rb"(\d+)\s+/media/memfault/mar_staging"), timeout=1)
    match = qemu.child().match
    assert match
    assert int(match.group(1)) <= storage_max_usage_kib, match
