#
# Copyright (c) Memfault, Inc.
# See License.txt for details
from .memfault_service_tester import MemfaultServiceTester
from .qemu import QEMU


# Assumptions:
# - The machine/qemu is built with a valid project key of a project on app.memfault.com,
#   or whatever the underlying QEMU instance points at.
# - The MEMFAULT_E2E_* environment variables are set to match whatever the underlying
#   QEMU instance points at.
#
# If you want to see the crashes on the server, remember to upload the symbols:
# qemu$ memfault --org $MEMFAULT_E2E_ORGANIZATION_SLUG --org-token $MEMFAULT_E2E_ORG_TOKEN \
#   --project $MEMFAULT_E2E_PROJECT_SLUG upload-yocto-symbols
#   --image tmp/deploy/images/qemuarm64/base-image-qemuarm64.tar.bz2
def test(qemu: QEMU, memfault_service_tester: MemfaultServiceTester, qemu_device_id: str) -> None:
    # Stream memfaultd's log and wait for memfaultd to start
    qemu.exec_cmd("journalctl --follow --unit=memfaultd.service &")
    qemu.child().expect("Started memfaultd daemon")

    # Stream logs from kernel: memfault-core-handler logs to dmsg
    qemu.exec_cmd("journalctl --follow -t kernel &")

    # Trigger the coredump
    qemu.exec_cmd("memfaultctl trigger-coredump")

    # Wait for coredump to be captured by memfault-core-handler:
    qemu.child().expect("Successfully captured coredump")

    # Ensure memfaultd has transmitted the corefile
    qemu.exec_cmd("memfaultctl sync")

    # Check that the backend created the coredump:
    memfault_service_tester.poll_elf_coredumps_until_count(
        1, device_serial=qemu_device_id, timeout_secs=90
    )

    # TODO: upload symbol files, so we can assert that the processing was w/o errors here and an issue got created.
