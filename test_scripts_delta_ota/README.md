# `test_scripts_delta_ota`

One E2E test: a zchunk delta OTA update on qemuarm64, installed from the mock
hawkBit DDI server in `scripts/delta_ota_local_server.py`, which `conftest.py`
imports and serves in a thread of the pytest process. It needs no Memfault
project, no org token and no CLI upload.

## What it asserts

| Test                                                     | Reads                                                                      |
| -------------------------------------------------------- | -------------------------------------------------------------------------- |
| `test_zck_is_fetched_only_with_range_requests`           | the mock's per-status counts for the `.zck`                                |
| `test_adjacent_chunk_runs_were_coalesced_into_one_range` | the bounds of every range served for the `.zck`                            |
| `test_delta_swu_is_fetched_whole`                        | the same per-status counts for the `.swu`                                  |
| `test_transfer_is_smaller_than_the_full_release`         | bytes served, against the size of `swupdate-image-*.swu` and of the `.zck` |
| `test_deployment_offers_the_target_version`              | the version in the served delta `.swu`                                     |
| `test_installed_partition_matches_the_release`           | sha256 of the target slot, against `base-image-*.ext4.gz`                  |
| `test_new_slot_booted_from_the_updated_partition`        | `/proc/cmdline` on the device                                              |
| `test_new_slot_reports_the_new_software_version`         | `/etc/memfaultd.conf` on the device                                        |
| `test_new_slot_contains_the_added_package`               | `strace` on the device                                                     |

The transfer test bounds the share at 50% of a full release as well as requiring
it to be smaller, because a fallback to a whole-file download would still be
smaller than a full `.swu`. It also bounds the part of the `.zck` that was
fetched at 25% of the `.zck` size.

The coalescing test is what verifies the second SWUpdate patch to merge adjacent
ranges into a single range in
`meta-memfault-example/recipes-support/swupdate/files/`.

Each test records one line through the `measure` fixture. The lines are printed
together under a `delta OTA measurements` separator at the end of the run.

## Running it

The device is built against the mock, not repointed at it afterwards:
`MEMFAULT_BASE_URL` reaches `/etc/memfaultd.conf` through memfaultd's
`do_install`, and memfaultd appends `/api/v0/hawkbit` to it when it writes
swupdate's suricatta section. So both builds and the test itself have to see the
same `MEMFAULT_BASE_URL` and `MEMFAULT_DEVICE_ID`.
`MEMFAULT_DELTA_OTA_TARGET_VERSION` has to be the `MEMFAULT_SOFTWARE_VERSION` of
the second build; the test reads it to check both what the mock served and what
the install wrote.

From the build directory inside the Docker container:

```bash
export MEMFAULT_BASE_URL=http://10.0.2.2:18080
export MEMFAULT_DEVICE_ID=delta-ota-tester
export MEMFAULT_DELTA_OTA_TARGET_VERSION=0.0.2
conf=/home/build/yocto/sources/memfault-linux-sdk/test_scripts_delta_ota/conf

# the release to upgrade from
MEMFAULT_SOFTWARE_VERSION=0.0.1 bitbake -R $conf/delta-ota.conf base-image
cp -L tmp/deploy/images/qemuarm64/ci-test-image.wic \
      tmp/deploy/images/qemuarm64/delta-ota-source.wic

# the release to upgrade to, its delta artifacts, and the full-image baseline
MEMFAULT_SOFTWARE_VERSION=$MEMFAULT_DELTA_OTA_TARGET_VERSION bitbake \
    -R $conf/delta-ota.conf -R $conf/delta-ota-strace.conf \
    swupdate-delta-image swupdate-image

pytest -vv -s /home/build/yocto/sources/memfault-linux-sdk/test_scripts_delta_ota
```

## Slot switching

`recipes-bsp/u-boot/files/0004-qemu-arm-rootpart-env.patch` adds
`memfault_boot`, which boots `/dev/vda${rootpart}` and puts the same partition
in `bootargs`, and `0005-memfault-bootcmd-defconfig-2026.01.patch` makes it the
boot command. swupdate sets `rootpart` to 3 while installing, so the reboot
after a successful install lands in slot B.

The install fixture therefore stops QEMU as soon as U-Boot prints
`Booting rootfs from /dev/vda3`: the partition has to be hashed before the new
slot mounts it, since ext4 touches the journal even on a read-only mount. It
then boots the wic again for the confirmation feedback (`-c 2`, from
`09-swupdate-args`) and the three on-device checks. `extlinux.conf` and the
`root=/dev/vda2` in `files/wic/image-qemu.wks` are unused.
