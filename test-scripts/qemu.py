#
# Copyright (c) Memfault, Inc.
# See License.txt for details
import os
import sys

import pexpect


class QEMU:
    def __init__(self):
        bitbake_path = os.getenv("BUILDDIR")
        assert bitbake_path, "Missing BUILDDIR environment variable"

        build_output_path = bitbake_path + "/tmp/deploy/images/qemuarm64"

        cmd_line_parts = []
        cmd_line_parts.append(
            bitbake_path
            + "/tmp/work/x86_64-linux/qemu-helper-native/1.0-r1/recipe-sysroot-native/usr/bin/qemu-system-aarch64"
        )
        cmd_line_parts.append(
            "-device virtio-net-pci,netdev=net0,mac=52:54:00:12:35:02 -netdev user,id=net0,hostfwd=tcp::2222-:22,hostfwd=tcp::2323-:23,tftp="
            + build_output_path
        )
        cmd_line_parts.append(
            "-object rng-random,filename=/dev/urandom,id=rng0 -device virtio-rng-pci,rng=rng0"
        )
        cmd_line_parts.append(
            "-drive id=disk0,file="
            + build_output_path
            + "/base-image-qemuarm64.wic,if=none,format=raw -device virtio-blk-device,drive=disk0"
        )
        cmd_line_parts.append(
            "-device qemu-xhci -device usb-tablet -device usb-kbd -device virtio-gpu-pci -nographic"
        )
        cmd_line_parts.append("-machine virt -cpu cortex-a57 -smp 4 -m 256")
        cmd_line_parts.append("-serial mon:stdio -serial null")
        cmd_line_parts.append("-bios " + build_output_path + "/u-boot.bin")

        self.pid = pexpect.spawn(
            " ".join(cmd_line_parts), timeout=120, logfile=sys.stdout.buffer
        )

    def __del__(self):
        self.pid.close()
        self.pid.wait()

    def child(self):
        return self.pid
