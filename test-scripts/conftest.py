#
# Copyright (c) Memfault, Inc.
# See License.txt for details
import os
import pathlib
import shutil
import subprocess
import textwrap
import uuid
from typing import Iterable

import pytest

# Ensure pytest rewrites asserts for better debuggability.
# https://docs.pytest.org/en/stable/writing_plugins.html#assertion-rewriting
pytest.register_assert_rewrite(
    "memfault_service_tester",
)

from memfault_service_tester import MemfaultServiceTester  # noqa: E402 M900
from qemu import QEMU  # noqa: E402 M900


@pytest.fixture()
def qemu_device_id() -> str:
    device_id = str(uuid.uuid4())
    # Let's leave this here to make debugging failing tests in CI a bit easier:
    print(f"MEMFAULT_DEVICE_ID={device_id}")
    return device_id


@pytest.fixture()
def qemu_hardware_version() -> str:
    return os.environ.get("MEMFAULT_HARDWARE_VERSION", "qemuarm64")


@pytest.fixture()
def memfault_device_info(tmpdir, qemu_device_id, qemu_hardware_version) -> pathlib.Path:
    fn = tmpdir / "memfault-device-info"
    with open(fn, "w") as f:
        f.write(
            textwrap.dedent(
                f"""\
                #!/bin/sh
                echo MEMFAULT_DEVICE_ID={qemu_device_id}
                echo MEMFAULT_HARDWARE_VERSION={qemu_hardware_version}
                """
            )
        )
    os.chmod(fn, 0o755)
    return fn


@pytest.fixture()
def qemu_image_wic_path(tmpdir, memfault_device_info) -> pathlib.Path:
    bitbake_path = os.getenv("BUILDDIR")
    assert bitbake_path, "Missing BUILDDIR environment variable"

    src_wic = (
        pathlib.Path(bitbake_path)
        / "tmp"
        / "deploy"
        / "images"
        / "qemuarm64"
        / "ci-test-image.wic"
    )
    dest_wic = tmpdir / "ci-test-image.copy.wic"

    shutil.copyfile(src_wic, dest_wic)

    partition_num = 2
    # Remove /usr/bin/memfault-device-info:
    subprocess.check_output(
        ["wic", "rm", f"{dest_wic}:{partition_num}/usr/bin/memfault-device-info"]
    )
    # Copy in the new /usr/bin/memfault-device-info:
    subprocess.check_output(
        ["wic", "cp", memfault_device_info, f"{dest_wic}:{partition_num}/usr/bin/"]
    )
    return dest_wic


@pytest.fixture()
def qemu(qemu_image_wic_path) -> QEMU:
    return QEMU(qemu_image_wic_path)


@pytest.fixture()
def memfault_service_tester() -> Iterable[MemfaultServiceTester]:
    st = MemfaultServiceTester(
        base_url=os.environ["MEMFAULT_E2E_API_BASE_URL"],
        organization_slug=os.environ["MEMFAULT_E2E_ORGANIZATION_SLUG"],
        project_slug=os.environ["MEMFAULT_E2E_PROJECT_SLUG"],
        organization_token=os.environ["MEMFAULT_E2E_ORG_TOKEN"],
    )
    yield st
