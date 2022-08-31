#
# Copyright (c) Memfault, Inc.
# See License.txt for details
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
    qemu.child().expect(" login:")
    qemu.child().sendline("root")

    # enable data collection, so that the reboot event can get captured
    qemu.exec_cmd("memfaultd --enable-data-collection")

    qemu.systemd_wait_for_service_state("memfaultd.service", "active")
    qemu.systemd_wait_for_service_state("collectd.service", "active")

    # SIGUSR1 triggers a force-flush and as a result, an HTTP request is made:
    # Note it's not possible to force-read all plugins (https://github.com/collectd/collectd/issues/4039),
    # so the HTTP request will only contain a few metrics that happened to have written data to collectd's cache.
    qemu.exec_cmd(
        "kill -s SIGUSR1 $(systemctl show --property MainPID --value collectd)"
    )

    reports = memfault_service_tester.poll_reports_until_count(
        1, device_serial=qemu_device_id, timeout_secs=30
    )
    assert reports
    assert reports[0]["metrics"]
