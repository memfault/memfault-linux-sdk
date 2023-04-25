#!/usr/bin/python3
#
# Copyright (c) Memfault, Inc.
# See License.txt for details
import dataclasses
import os
import pathlib
import platform
import shlex
from typing import Dict, List, Union


def get_build_dir() -> pathlib.Path:
    bitbake_path = os.getenv("BUILDDIR")
    assert bitbake_path, "Missing BUILDDIR environment variable"
    return pathlib.Path(bitbake_path)


def get_machine() -> str:
    machine = os.getenv("MACHINE")
    assert machine, "Missing MACHINE environment variable"
    return machine


def get_host_arch() -> str:
    return platform.machine()


BASE_IMAGE_FILENAME = f"base-image-{get_machine()}.wic"

SYSTEM_PARTITION_A_INDEX = 2


def qemu_get_image_wic_path(filename: str) -> pathlib.Path:
    return get_build_dir() / "tmp" / "deploy" / "images" / get_machine() / filename


def qemu_get_system_partition_a_index() -> int:
    return SYSTEM_PARTITION_A_INDEX


_DEFAULT_IMAGE_WIC_PATH = qemu_get_image_wic_path(BASE_IMAGE_FILENAME)


@dataclasses.dataclass(frozen=True)
class QemuInfo:
    executable_name: str
    cpu_name: str


def qemu_build_command(
    image_wic_path: os.PathLike = _DEFAULT_IMAGE_WIC_PATH,
) -> List[str]:
    machine_to_qemu_info: Dict[str, QemuInfo] = {
        "qemuarm": QemuInfo(executable_name="qemu-system-arm", cpu_name="cortex-a15"),
        "qemuarm64": QemuInfo(
            executable_name="qemu-system-aarch64", cpu_name="cortex-a57"
        ),
    }

    bitbake_path = get_build_dir()
    machine = get_machine()
    qemu_info = machine_to_qemu_info[machine]

    build_output_path = bitbake_path / "tmp/deploy/images" / machine

    command_parts: List[Union[str, pathlib.Path]] = []
    command_parts.append(
        bitbake_path
        / "tmp/work"
        / f"{get_host_arch()}-linux"
        / "qemu-helper-native/1.0-r1/recipe-sysroot-native/usr/bin"
        / qemu_info.executable_name
    )
    command_parts.append(
        "-device virtio-net-pci,netdev=net0,mac=52:54:00:12:35:02 -netdev user,id=net0"
    )
    command_parts.append(
        "-object rng-random,filename=/dev/urandom,id=rng0 -device virtio-rng-pci,rng=rng0"
    )
    command_parts.append(
        f"-drive id=disk0,file={image_wic_path},if=none,format=raw -device virtio-blk-device,drive=disk0"
    )
    command_parts.append(
        "-device qemu-xhci -device usb-tablet -device usb-kbd -device virtio-gpu-pci -nographic"
    )
    command_parts.append(f"-machine virt -cpu {qemu_info.cpu_name} -smp 4 -m 512M")
    command_parts.append("-serial mon:stdio -serial null")
    command_parts.append("-bios " + str(build_output_path / "u-boot.bin"))

    return shlex.split(" ".join((str(part) for part in command_parts)))


if __name__ == "__main__":
    executable, *args = qemu_build_command()
    os.execv(executable, [executable] + args)
