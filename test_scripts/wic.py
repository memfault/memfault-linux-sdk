#
# Copyright (c) Memfault, Inc.
# See License.txt for details
import os
import pathlib
import platform
import shutil
import subprocess
from functools import cache

DEFAULT_PART = 2


@cache
def _wic_executable() -> str:
    """Resolve wic, falling back to bitbake's sysroots-components dir."""
    if shutil.which("wic"):
        return "wic"

    build_dir = os.getenv("BUILDDIR")
    if not build_dir:
        return "wic"

    components_dir = pathlib.Path(build_dir) / "tmp/sysroots-components" / platform.machine()
    path = components_dir / "wic-native/usr/bin/wic"
    if not path.exists():
        return "wic"

    python3_native_bin = components_dir / "python3-native/usr/bin"
    if python3_native_bin.is_dir():
        os.environ["PATH"] = os.pathsep.join([
            str(python3_native_bin),
            *([v] if (v := os.environ.get("PATH")) else []),
        ])

    site_packages = sorted((components_dir / "wic-native/usr/lib").glob("python3.*/site-packages"))
    if site_packages:
        os.environ["PYTHONPATH"] = os.pathsep.join([
            str(site_packages[-1]),
            *([v] if (v := os.environ.get("PYTHONPATH")) else []),
        ])

    return str(path)


class WicImage:
    dest_wic: pathlib.Path
    default_part: int

    def __init__(self, src_wic: pathlib.Path, dest_wic: pathlib.Path, default_part: int) -> None:
        self.dest_wic = dest_wic
        self.default_part = default_part
        shutil.copyfile(src_wic, self.dest_wic)

    def rm(self, path: str, part: int | None = None) -> None:
        if part is None:
            part = self.default_part
        subprocess.check_output([_wic_executable(), "rm", f"{self.dest_wic}:{part}{path}"])

    def add_file(self, src: pathlib.Path, to: str, part: int | None = None) -> None:
        """Add a file into one of the image partitions. Note that this method cannot create new folder."""
        if part is None:
            part = self.default_part
        subprocess.check_output([
            _wic_executable(),
            "cp",
            src,
            f"{self.dest_wic}:{part}{to}",
        ])

    def extract_file(self, src: str, to: pathlib.Path, part: int | None = None) -> None:
        """Copy a file from one of the image partition to the local machine."""
        if part is None:
            part = self.default_part
        subprocess.check_output([_wic_executable(), "cp", f"{self.dest_wic}:{part}{src}", to])
