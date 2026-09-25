#
# Copyright (c) Memfault, Inc.
# See License.txt for details
"""Fixtures for the zchunk delta OTA E2E test.

The image under test is built against the mock DDI server rather than repointed
at it afterwards, so MEMFAULT_BASE_URL and MEMFAULT_DEVICE_ID here have to be
the ones the image was built with; see the README.
"""

import contextlib
import os
import pathlib
import shutil
import threading
import time
from collections.abc import Callable, Generator, Iterator
from typing import TYPE_CHECKING, cast
from urllib.parse import urlparse

import pexpect
import pytest
from scripts import delta_ota_local_server
from test_scripts.qemu import QEMU

from . import artifacts
from .artifacts import InstalledSlot

if TYPE_CHECKING:
    from _pytest.terminal import TerminalReporter
    from pexpect.spawnbase import _InputRePattern

_INSTALL_OUTCOMES: "list[_InputRePattern]" = [
    "SWUPDATE successful !",
    "SWUPDATE failed",
    "fallback to full download",
]
_JOURNAL_BACKLOG_LINES = 1000
_INSTALL_STARTED = "Installation in progress"
_BOOTING_TARGET_SLOT = f"Booting rootfs from /dev/vda{artifacts.TARGET_PARTITION}"
_DEPLOYMENT_TIMEOUT_SECONDS = 300
_INSTALL_TIMEOUT_SECONDS = 1800
_REBOOT_TIMEOUT_SECONDS = 300
_CONFIRM_TIMEOUT_SECONDS = 300
_POLL_INTERVAL_SECONDS = 1.0
_DDI_POLL_SECONDS = 5
_SEARCH_WINDOW_BYTES = 4096

_MEASURED: list[str] = []


@pytest.fixture()
def measure(request: pytest.FixtureRequest) -> Callable[[str], None]:
    """Record one line about what this test read, for the end-of-run summary."""
    name = cast("str", request.node.name)  # pyright: ignore[reportUnknownMemberType]

    def record(line: str) -> None:
        _MEASURED.append(f"{name}: {line}")

    return record


def pytest_terminal_summary(terminalreporter: "TerminalReporter") -> None:
    if not _MEASURED:
        return
    terminalreporter.write_sep("=", "delta OTA measurements")
    for line in _MEASURED:
        terminalreporter.write_line(line)


@contextlib.contextmanager
def _explain_hangs(qemu: QEMU, what: str) -> Generator[None]:
    """Turn a pexpect timeout into a failure that says what was being waited for."""
    try:
        yield
    except (pexpect.TIMEOUT, pexpect.EOF) as error:
        raise AssertionError(f"{what}\n\n--- console tail ---\n{qemu.console_tail()}") from error


@pytest.fixture(scope="session")
def delta_artifacts() -> artifacts.DeltaArtifacts:
    return artifacts.collect()


@pytest.fixture(scope="session")
def mock_ddi_endpoint() -> tuple[str, int]:
    hint = (
        "MEMFAULT_BASE_URL has to name the mock DDI server's host and port, for example "
        "http://10.0.2.2:18080, and the image under test has to be built with it"
    )
    parsed = urlparse(os.environ.get("MEMFAULT_BASE_URL", ""))
    assert parsed.hostname, hint
    assert parsed.port, hint
    return parsed.hostname, parsed.port


@pytest.fixture(scope="session")
def target_software_version() -> str:
    version = os.environ.get("MEMFAULT_DELTA_OTA_TARGET_VERSION", "")
    assert version, (
        "MEMFAULT_DELTA_OTA_TARGET_VERSION has to be the MEMFAULT_SOFTWARE_VERSION the "
        "second build was run with, for example 0.0.2"
    )
    return version


@pytest.fixture(scope="session")
def qemu_device_id() -> str:
    device_id = os.environ.get("MEMFAULT_DEVICE_ID", "")
    assert device_id, (
        "MEMFAULT_DEVICE_ID has to be the controller id the image under test was built with; "
        "a different one is offered no deployment and times out"
    )
    return device_id


@pytest.fixture(scope="session")
def working_wic(
    delta_artifacts: artifacts.DeltaArtifacts, tmp_path_factory: pytest.TempPathFactory
) -> pathlib.Path:
    # QEMU opens the wic read-write with no snapshot, so the install, the U-Boot
    # environment and the kernel command line all land in the file it boots.
    dest = tmp_path_factory.mktemp("image") / artifacts.SOURCE_WIC_FILENAME
    _ = shutil.copyfile(delta_artifacts.source_wic, dest)
    return dest


@pytest.fixture(scope="session")
def ddi_server(
    delta_artifacts: artifacts.DeltaArtifacts,
    mock_ddi_endpoint: tuple[str, int],
    qemu_device_id: str,
) -> Iterator[delta_ota_local_server.State]:
    host, port = mock_ddi_endpoint
    state = delta_ota_local_server.build_ddi_state(
        deploy=delta_artifacts.deploy,
        host=host,
        port=port,
        device_id=qemu_device_id,
        poll=_DDI_POLL_SECONDS,
    )
    httpd = delta_ota_local_server.make_server(state, port)
    threading.Thread(target=httpd.serve_forever, daemon=True).start()
    try:
        yield state
    finally:
        httpd.shutdown()
        httpd.server_close()


def _wait_for_confirmation(state: delta_ota_local_server.State, timeout: float) -> None:
    """Wait for the device to close the deployment as a success.

    09-swupdate-args passes -c 2 on the boot after an install, which is what
    sends that feedback.
    """
    deadline = time.monotonic() + timeout
    while not state.done:
        assert time.monotonic() < deadline, (
            f"the device did not report a successful install in {timeout:.0f}s; "
            f"last feedback: {state.last_feedback}"
        )
        time.sleep(_POLL_INTERVAL_SECONDS)


@pytest.fixture(scope="session")
def installed_slot(
    working_wic: pathlib.Path,
    ddi_server: delta_ota_local_server.State,
) -> InstalledSlot:
    """Install the delta update, then read the target slot back from the file.

    The hash has to be taken with QEMU gone and before anything mounts the slot,
    since ext4 writes to the journal even on a read-only mount. swupdate flips
    rootpart as part of the install and memfault_boot reads it back, so the
    reboot lands in the target slot and mounts it read-write; U-Boot announcing
    that slot is the last point at which the partition is still untouched.
    """
    qemu = QEMU(working_wic)
    # journald here does not forward to the console, so the install has to be
    # streamed to the serial line to be watched at all.
    qemu.exec_cmd(f"journalctl -u swupdate.service -f -n {_JOURNAL_BACKLOG_LINES}")

    with _explain_hangs(qemu, "the deployment was never offered or installed"):
        _ = qemu.child().expect(  # pyright: ignore[reportUnknownMemberType]
            _INSTALL_STARTED,
            timeout=_DEPLOYMENT_TIMEOUT_SECONDS,
            searchwindowsize=_SEARCH_WINDOW_BYTES,
        )
    with _explain_hangs(qemu, "the install started but never finished"):
        outcome = qemu.child().expect(  # pyright: ignore[reportUnknownMemberType]
            _INSTALL_OUTCOMES,
            timeout=_INSTALL_TIMEOUT_SECONDS,
            searchwindowsize=_SEARCH_WINDOW_BYTES,
        )
    assert outcome == 0, f"the delta install did not succeed: {_INSTALL_OUTCOMES[outcome]!r}"
    with _explain_hangs(
        qemu, f"the install finished but U-Boot never booted /dev/vda{artifacts.TARGET_PARTITION}"
    ):
        _ = qemu.child().expect(  # pyright: ignore[reportUnknownMemberType]
            _BOOTING_TARGET_SLOT,
            timeout=_REBOOT_TIMEOUT_SECONDS,
            searchwindowsize=_SEARCH_WINDOW_BYTES,
        )
    qemu.terminate()

    return InstalledSlot(
        summary=ddi_server.summary(),
        partition_sha256=artifacts.sha256_of_partition(working_wic, artifacts.TARGET_PARTITION),
    )


@pytest.fixture(scope="session")
def new_slot_qemu(
    installed_slot: InstalledSlot,
    working_wic: pathlib.Path,
    ddi_server: delta_ota_local_server.State,
) -> Iterator[QEMU]:
    """Boot the slot the update was written to, which is the boot that confirms it."""
    qemu = QEMU(working_wic)
    qemu.exec_cmd_ok(f'test "$(fw_printenv -n rootpart)" = {artifacts.TARGET_PARTITION}')
    _wait_for_confirmation(ddi_server, timeout=_CONFIRM_TIMEOUT_SECONDS)
    yield qemu
    qemu.poweroff()
