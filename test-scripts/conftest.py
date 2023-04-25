#
# Copyright (c) Memfault, Inc.
# See License.txt for details
import json
import os
import pathlib
import textwrap
import uuid
from typing import Iterable

import pytest
import runqemu
from wic import WicImage

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


@pytest.fixture(autouse=True)
def set_test_id_device_attribute(request: pytest.FixtureRequest) -> Iterable[None]:
    """
    Set a device attribute called "test_id" to the current test's nodeid, to make it easy to find all devices that have
    run a particular test case via the device search.
    """
    fixture_names = ("memfault_service_tester", "qemu_device_id")
    if not all(f in request.fixturenames for f in fixture_names):
        yield
        return

    memfault_service_tester, qemu_device_id = (
        request.getfixturevalue(f) for f in fixture_names
    )
    yield
    # Patch the device attributes after the test has run, because the patch endpoint requires the device to exist.
    key = "test_id"
    try:
        memfault_service_tester.create_custom_metric(key=key, data_type="STRING")
    except AssertionError:
        pass  # Ignore 409 Conflict errors, which indicate the metric already exists.
    memfault_service_tester.patch_device_attributes(
        device_serial=qemu_device_id,
        patch={key: f"{request.node.path.name}::{request.node.name}"},
    )


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


CI_IMAGE_FILENAME = "ci-test-image.wic"


@pytest.fixture()
def memfault_extra_config(data_collection_enabled: bool) -> object:
    if not data_collection_enabled:
        return {}
    else:
        return {"enable_data_collection": True}


@pytest.fixture()
def qemu_image_wic_path(
    tmpdir, memfault_device_info, memfault_extra_config, swupdate_enabled
) -> WicImage:
    dest_wic = tmpdir / "ci-test-image.copy.wic"
    image = WicImage(
        runqemu.qemu_get_image_wic_path(CI_IMAGE_FILENAME),
        dest_wic,
        runqemu.qemu_get_system_partition_a_index(),
    )
    install_memfault_device_info(image, memfault_device_info)
    install_extra_config(tmpdir, image, memfault_extra_config)

    if swupdate_enabled is False:
        disable_swupdate(image)

    return dest_wic


def install_memfault_device_info(qemu_image: WicImage, memfault_device_info):
    qemu_image.rm("/usr/bin/memfault-device-info")
    qemu_image.add_file(memfault_device_info, "/usr/bin/")


def install_extra_config(tmpdir, qemu_image: WicImage, memfault_extra_config):
    config = tmpdir / "memfaultd.conf"
    config_file = "/etc/memfaultd.conf"
    qemu_image.extract_file(config_file, config)

    with open(config, "r") as f:
        json_data = json.load(f)

    json_data.update(memfault_extra_config)

    with open(config, "w") as f:
        json.dump(json_data, f)

    qemu_image.rm(config_file)
    qemu_image.add_file(config, config_file)


def disable_swupdate(qemu_image: WicImage):
    qemu_image.rm("/lib/systemd/system/swupdate.service")
    qemu_image.rm("/lib/systemd/system/swupdate.socket")


@pytest.fixture()
def qemu(qemu_image_wic_path) -> QEMU:
    return QEMU(qemu_image_wic_path)


@pytest.fixture()
def swupdate_enabled() -> bool:
    return False


@pytest.fixture()
def data_collection_enabled() -> bool:
    return True


@pytest.fixture()
def memfault_service_tester() -> Iterable[MemfaultServiceTester]:
    st = MemfaultServiceTester(
        base_url=os.environ["MEMFAULT_E2E_API_BASE_URL"],
        organization_slug=os.environ["MEMFAULT_E2E_ORGANIZATION_SLUG"],
        project_slug=os.environ["MEMFAULT_E2E_PROJECT_SLUG"],
        organization_token=os.environ["MEMFAULT_E2E_ORG_TOKEN"],
    )
    yield st


@pytest.fixture()
def wait_for_memfaultd_start() -> bool:
    return True


@pytest.fixture(autouse=True)
def do_wait_for_memfaultd(wait_for_memfaultd_start, qemu: QEMU):
    if wait_for_memfaultd_start:
        qemu.wait_for_memfaultd_start()
