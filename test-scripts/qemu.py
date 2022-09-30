#
# Copyright (c) Memfault, Inc.
# See License.txt for details
import os
import sys
import time
from typing import List, Literal

import pexpect
import runqemu

SystemdState = Literal[
    "inactive",
    "active",
    "deactivating",
    "activating",
    "reloading",
    "failed",
    "maintenance",
]


class QEMU:
    def __init__(self, image_wic_path: os.PathLike):
        command, *args = runqemu.qemu_build_command(image_wic_path)
        self.pid = pexpect.spawn(command, args, timeout=120, logfile=sys.stdout.buffer)
        self._prepare()

    def __del__(self):
        self.pid.close()
        self.pid.wait()

    def _prepare(self):
        self.pid.expect(" login:")
        self.pid.sendline("root")
        self._set_env()

    def _set_env(self):
        # Setting this environment variable to an empty string or the value "cat" is equivalent to passing --no-pager.
        # A pager (e.g. "less") could prevent E2E tests that check on journalctl output from passing.
        self.exec_cmd("export PAGER=cat")
        # Similarly, use "cat" as a pager for other programs that may honor $PAGER (e.g. systemd does).
        self.exec_cmd("export SYSTEMD_PAGER=cat")

    def child(self):
        return self.pid

    def exec_cmd(self, cmd):
        self.pid.sendline("")
        self.pid.expect("#")
        self.pid.sendline(cmd)
        self.pid.expect("\n")

    def systemd_wait_for_service_state(
        self,
        service: str,
        expected_state: SystemdState,
        *,
        timeout_seconds: float = 10.0,
    ):
        states: List[SystemdState] = [
            "inactive",
            "active",
            "deactivating",
            "activating",
            "reloading",
            "failed",
            "maintenance",
        ]
        expected_state_idx = states.index(expected_state)
        timeout = time.time() + timeout_seconds
        last_state_idx = -1
        while time.time() < timeout:
            self.exec_cmd(f"systemctl is-active {service}")
            last_state_idx = self.child().expect(states)
            if last_state_idx == expected_state_idx:
                return
            time.sleep(0.1)
        raise TimeoutError(
            f"Timed out waiting for service {service} to get state {expected_state}. Last state: {states[last_state_idx]}"
        )
