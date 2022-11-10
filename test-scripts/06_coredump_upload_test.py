#
# Copyright (c) Memfault, Inc.
# See License.txt for details
import time

from memfault_service_tester import MemfaultServiceTester
from qemu import QEMU


# Assumptions:
# - The machine/qemu is built with a valid project key of a project on app.memfault.com,
#   or whatever the underlying QEMU instance points at.
# - The MEMFAULT_E2E_* environment variables are set to match whatever the underlying
#   QEMU instance points at.
def test(
    qemu: QEMU, memfault_service_tester: MemfaultServiceTester, qemu_device_id: str
):
    # Enable data collection, activating the coredump functionality
    qemu.exec_cmd("memfaultd --enable-data-collection")
    qemu.systemd_wait_for_service_state("memfaultd.service", "active")

    # Give memfaultd a moment to start the socket thread
    time.sleep(1)

    # Stream memfaultd's log
    qemu.exec_cmd("journalctl --follow --unit=memfaultd.service &")

    # Generate corefile from killing 'sleep' process
    qemu.exec_cmd("sleep 5s &")
    qemu.exec_cmd("SLEEP_PID=$!")

    # Give the sleep program a bit of time to start and run:
    qemu.exec_cmd("sleep 0.1s")

    # Now send SIGQUIT to trigger a coredump!
    qemu.exec_cmd("kill -SIGQUIT $SLEEP_PID")

    # Ensure memfaultd has received the core
    qemu.child().expect("coredump:: Received corefile for PID")

    # Trigger memfaultd to parse TX queue
    qemu.exec_cmd(
        "kill -SIGUSR1 $(systemctl show --property MainPID --value memfaultd.service)"
    )

    # Ensure memfaultd has transmitted the corefile
    qemu.child().expect("network:: Successfully transmitted file")

    # Check that the backend created the coredump:
    memfault_service_tester.poll_elf_coredumps_until_count(
        1, device_serial=qemu_device_id, timeout_secs=60
    )

    # TODO: upload symbol files, so we can assert that the processing was w/o errors here and an issue got created.
