#
# Copyright (c) Memfault, Inc.
# See License.txt for details
"""Deploy-dir lookups and disk-image reads for the delta OTA E2E test."""

import dataclasses
import gzip
import hashlib
import pathlib
import struct
from typing import cast

from scripts.delta_ota_local_server import Summary
from test_scripts import runqemu

# Copied out of the deploy dir between the two builds, because the 0.0.2 build
# re-points every IMAGE_LINK_NAME symlink there, ci-test-image.wic included.
SOURCE_WIC_FILENAME = "delta-ota-source.wic"

# OTA_PARTITION_B from conf/machine/include/qemu-memfault.inc; the source build
# boots OTA_PARTITION_A, so the install goes here.
TARGET_PARTITION = 3

_MBR_TABLE_OFFSET = 446
_MBR_ENTRY_SIZE = 16
_MBR_ENTRY_COUNT = 4
_SECTOR_SIZE = 512
_BLOCK_SIZE = 1 << 20


def _image_link(deploy: pathlib.Path, pattern: str) -> pathlib.Path:
    """Resolve an IMAGE_LINK_NAME symlink, which points at the newest build."""
    matches = sorted({p.resolve() for p in deploy.glob(pattern) if p.is_symlink()})
    assert len(matches) == 1, f"expected one {pattern} symlink in {deploy}, found {matches}"
    return matches[0]


def _sha256_of_gzip(path: pathlib.Path) -> tuple[str, int]:
    digest = hashlib.sha256()
    size = 0
    with gzip.open(path, "rb") as f:
        while block := f.read(_BLOCK_SIZE):
            digest.update(block)
            size += len(block)
    return digest.hexdigest(), size


def _partition_bounds(wic: pathlib.Path, index: int) -> tuple[int, int]:
    assert 1 <= index <= _MBR_ENTRY_COUNT, "an MBR holds four primary partitions"
    offset = _MBR_TABLE_OFFSET + _MBR_ENTRY_SIZE * (index - 1)
    with wic.open("rb") as f:
        entry = f.read(_SECTOR_SIZE)[offset : offset + _MBR_ENTRY_SIZE]
    first_lba, sectors = cast("tuple[int, int]", struct.unpack("<II", entry[8:16]))
    assert sectors, f"partition {index} of {wic.name} is empty"
    return first_lba * _SECTOR_SIZE, sectors * _SECTOR_SIZE


def sha256_of_partition(wic: pathlib.Path, index: int) -> str:
    start, length = _partition_bounds(wic, index)
    digest = hashlib.sha256()
    with wic.open("rb") as f:
        _ = f.seek(start)
        remaining = length
        while remaining:
            block = f.read(min(_BLOCK_SIZE, remaining))
            assert block, f"{wic.name} ends inside partition {index}"
            digest.update(block)
            remaining -= len(block)
    return digest.hexdigest()


@dataclasses.dataclass(frozen=True)
class InstalledSlot:
    """What the delta install left behind, read after QEMU exited."""

    summary: Summary
    partition_sha256: str


@dataclasses.dataclass(frozen=True)
class DeltaArtifacts:
    deploy: pathlib.Path
    source_wic: pathlib.Path
    full_swu: pathlib.Path
    rootfs_sha256: str
    rootfs_size: int

    @property
    def full_swu_size(self) -> int:
        return self.full_swu.stat().st_size


def collect() -> DeltaArtifacts:
    deploy = runqemu.get_deploy_dir()
    source_wic = deploy / SOURCE_WIC_FILENAME
    assert source_wic.is_file(), (
        f"{source_wic} is missing. It is the disk image of the release being upgraded from, "
        f"copied out of the deploy dir before the second build; see the README."
    )
    rootfs_sha256, rootfs_size = _sha256_of_gzip(_image_link(deploy, "base-image-*.ext4.gz"))
    return DeltaArtifacts(
        deploy=deploy,
        source_wic=source_wic,
        full_swu=_image_link(deploy, "swupdate-image-*.swu"),
        rootfs_sha256=rootfs_sha256,
        rootfs_size=rootfs_size,
    )
