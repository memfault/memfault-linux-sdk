#
# Copyright (c) Memfault, Inc.
# See License.txt for details
#!/usr/bin/python3

import os
import pathlib
import shlex
from typing import List

BASE_IMAGE_FILENAME = "base-image-qemuarm64.wic"


def qemu_get_image_wic_path(filename: str) -> pathlib.Path:
    bitbake_path = os.getenv("BUILDDIR")
    assert bitbake_path, "Missing BUILDDIR environment variable"

    return (
        pathlib.Path(bitbake_path)
        / "tmp"
        / "deploy"
        / "images"
        / "qemuarm64"
        / filename
    )


def qemu_build_command(
    image_wic_path: os.PathLike = qemu_get_image_wic_path(BASE_IMAGE_FILENAME),
) -> List[str]:
    bitbake_path = os.getenv("BUILDDIR")
    assert bitbake_path, "Missing BUILDDIR environment variable"

    build_output_path = bitbake_path + "/tmp/deploy/images/qemuarm64"

    command_parts = []
    command_parts.append(
        bitbake_path
        + "/tmp/work/x86_64-linux/qemu-helper-native/1.0-r1/recipe-sysroot-native/usr/bin/qemu-system-aarch64"
    )
    command_parts.append(
        "-device virtio-net-pci,netdev=net0,mac=52:54:00:12:35:02 -netdev user,id=net0,hostfwd=tcp::2222-:22,hostfwd=tcp::2323-:23,tftp="
        + build_output_path
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
    command_parts.append("-machine virt -cpu cortex-a57 -smp 4 -m 256")
    command_parts.append("-serial mon:stdio -serial null")
    command_parts.append("-bios " + build_output_path + "/u-boot.bin")

    return shlex.split(" ".join(command_parts))


if __name__ == "__main__":
    executable, *args = qemu_build_command()
    os.execv(executable, [executable] + args)
