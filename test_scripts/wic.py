#
# Copyright (c) Memfault, Inc.
# See License.txt for details
import pathlib
import shutil
import subprocess

DEFAULT_PART = 2


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
        subprocess.check_output(["wic", "rm", f"{self.dest_wic}:{part}{path}"])

    def add_file(self, src: pathlib.Path, to: str, part: int | None = None) -> None:
        """Add a file into one of the image partitions. Note that this method cannot create new folder."""
        if part is None:
            part = self.default_part
        subprocess.check_output([
            "wic",
            "cp",
            src,
            f"{self.dest_wic}:{part}{to}",
        ])

    def extract_file(self, src: str, to: pathlib.Path, part: int | None = None) -> None:
        """Copy a file from one of the image partition to the local machine."""
        if part is None:
            part = self.default_part
        subprocess.check_output(["wic", "cp", f"{self.dest_wic}:{part}{src}", to])
