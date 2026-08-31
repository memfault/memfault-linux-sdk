#!/usr/bin/python3
#
# Copyright (c) Memfault, Inc.
# See License.txt for details
import dataclasses
import os
import pathlib
import platform
import re
import shlex
import subprocess


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


BASE_IMAGE_RECIPE = "base-image"

SYSTEM_PARTITION_A_INDEX = 2

_IMAGE_LINK_NAME_RE = re.compile(r'^(?:export )?IMAGE_LINK_NAME="(?P<value>.*)"$', re.MULTILINE)


def get_base_image_wic_path() -> pathlib.Path:
    # IMAGE_LINK_NAME (the stable symlink name bitbake creates for the most
    # recent build) is release-dependent; ask bitbake instead of guessing it.
    output = subprocess.run(
        ["bitbake", "-e", BASE_IMAGE_RECIPE],
        check=True,
        capture_output=True,
        text=True,
    ).stdout
    match = _IMAGE_LINK_NAME_RE.search(output)
    assert match, f"Could not find IMAGE_LINK_NAME in `bitbake -e {BASE_IMAGE_RECIPE}` output"
    return qemu_get_image_wic_path(f"{match['value']}.wic")


def qemu_get_image_wic_path(filename: str) -> pathlib.Path:
    return get_build_dir() / "tmp" / "deploy" / "images" / get_machine() / filename


def qemu_get_system_partition_a_index() -> int:
    return SYSTEM_PARTITION_A_INDEX


def _sysroots_components_dir() -> pathlib.Path:
    return get_build_dir() / "tmp/sysroots-components" / get_host_arch()


@dataclasses.dataclass(frozen=True)
class QemuInfo:
    executable_name: str
    cpu_name: str


def qemu_build_env() -> dict[str, str]:
    # qemu-system-native's shared libs (pixman, glib, slirp, ...) aren't on the
    # loader's search path outside of a bitbake task. Keep this out of
    # os.environ: these dirs also hold a native libext2fs that the host's
    # debugfs (used by `wic rm`) would then load and fail against.
    components_dir = _sysroots_components_dir()
    native_lib_dirs = [str(p) for p in components_dir.glob("*/usr/lib")]
    env = dict(os.environ)
    if native_lib_dirs:
        env["LD_LIBRARY_PATH"] = os.pathsep.join([
            *native_lib_dirs,
            *([v] if (v := os.environ.get("LD_LIBRARY_PATH")) else []),
        ])
    return env


def qemu_build_command(
    image_wic_path: pathlib.Path | None = None,
) -> list[str]:
    if image_wic_path is None:
        image_wic_path = get_base_image_wic_path()
    machine_to_qemu_info: dict[str, QemuInfo] = {
        "qemuarm": QemuInfo(executable_name="qemu-system-arm", cpu_name="cortex-a15"),
        "qemuarm64": QemuInfo(executable_name="qemu-system-aarch64", cpu_name="cortex-a57"),
    }

    bitbake_path = get_build_dir()
    machine = get_machine()
    qemu_info = machine_to_qemu_info[machine]

    build_output_path = bitbake_path / "tmp/deploy/images" / machine

    qemu_binary = (
        _sysroots_components_dir() / "qemu-system-native/usr/bin" / qemu_info.executable_name
    )
    assert qemu_binary.exists(), f"{qemu_binary} not found (bitbake qemu-system-native)"

    command_parts: list[str | pathlib.Path] = []
    command_parts.append(qemu_binary)
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

    return shlex.split(" ".join(str(part) for part in command_parts))


if __name__ == "__main__":
    executable, *args = qemu_build_command()
    os.execve(executable, [executable, *args], qemu_build_env())  # noqa: S606
