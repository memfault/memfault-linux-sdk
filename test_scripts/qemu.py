#
# Copyright (c) Memfault, Inc.
# See License.txt for details
import pathlib
import sys
import time
import uuid
from typing import Literal, cast

import pexpect

from . import runqemu

_CONSOLE_TAIL_BYTES = 4000

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
    def __init__(self, image_wic_path: pathlib.Path) -> None:
        command, *args = runqemu.qemu_build_command(image_wic_path)
        self.pid = pexpect.spawn(
            command,
            args,
            timeout=120,
            logfile=sys.stdout.buffer,
            env=runqemu.qemu_build_env(),
        )
        self.login()

    def __del__(self) -> None:
        self.pid.close()
        self.pid.wait()

    def _set_env(self) -> None:
        # Setting this environment variable to an empty string or the value "cat" is equivalent to passing --no-pager.
        # A pager (e.g. "less") could prevent E2E tests that check on journalctl output from passing.
        self.exec_cmd("export PAGER=cat")
        # Similarly, use "cat" as a pager for other programs that may honor $PAGER (e.g. systemd does).
        self.exec_cmd("export SYSTEMD_PAGER=cat")

    def login(self, timeout: float = 120) -> None:
        self.pid.expect(" login:", timeout=cast("int", timeout))
        self.pid.sendline("root")
        self._set_env()

    def child(self) -> pexpect.spawn:
        return self.pid

    def exec_cmd(self, cmd: str) -> None:
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
    ) -> None:
        states: list[SystemdState] = [
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
            last_state_idx = self.child().expect(states)  # pyright: ignore[reportArgumentType]
            if last_state_idx == expected_state_idx:
                return
            time.sleep(0.1)
        raise TimeoutError(
            f"Timed out waiting for service {service} to get state {expected_state}. Last state: {states[last_state_idx]}"
        )

    def expect_journald_message(
        self, unit: str, message: str, timeout: float = 3, last_lines: int = 0
    ) -> None:
        """Wait for a specific message from a journald unit."""
        self.exec_cmd(f'(journalctl -f -u {unit} -n {last_lines} &) |grep -q "{message}"')
        self.pid.expect("#", timeout=cast("int", timeout))

    def console_tail(self, limit: int = _CONSOLE_TAIL_BYTES) -> str:
        """The console output pexpect read but did not match."""
        seen = cast("bytes", self.pid.before or b"")  # pyright: ignore[reportUnknownMemberType]
        return seen[-limit:].decode(errors="replace")

    def exec_cmd_ok(self, cmd: str, timeout: float = 120) -> None:
        """Run a command and assert that it exited zero.

        The console echoes the command back, so the patterns match on the
        marker's value, never on the marker alone.
        """
        marker = f"RC{uuid.uuid4().hex[:8]}"
        self.exec_cmd(f"{cmd}; echo {marker}=$?")
        patterns = [f"{marker}=0", f"{marker}=[1-9]"]
        try:
            index = self.pid.expect(patterns, timeout=cast("int", timeout))  # pyright: ignore[reportArgumentType, reportUnknownMemberType]
        except (pexpect.TIMEOUT, pexpect.EOF) as error:
            raise AssertionError(
                f"no exit status came back for: {cmd}\n\n"
                f"--- console tail ---\n{self.console_tail()}"
            ) from error
        assert index == 0, f"command exited non-zero on the device: {cmd}"

    def terminate(self) -> None:
        """Stop QEMU without running anything on the guest.

        SIGHUP first, so QEMU flushes and closes the disk image on its way out.
        """
        self.pid.terminate(force=True)  # pyright: ignore[reportUnknownMemberType]
        self.pid.wait()  # pyright: ignore[reportUnknownMemberType]

    def poweroff(self, timeout: float = 120) -> None:
        """Halt the guest and wait for QEMU to exit."""
        self.exec_cmd("poweroff")
        self.pid.expect(pexpect.EOF, timeout=cast("int", timeout))  # pyright: ignore[reportUnknownMemberType]
        self.pid.wait()  # pyright: ignore[reportUnknownMemberType]

    def wait_for_memfaultd_start(self, timeout: float = 3) -> None:
        """Wait for memfaultd to start - Note that this will return immediately if memfaultd was just started."""
        self.expect_journald_message(
            "memfaultd", "Started memfaultd daemon", last_lines=10, timeout=timeout
        )
