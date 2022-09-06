# `meta-memfault-example`

This layer is an example implementation that builds a Yocto image for
`qemuarm64` that's fully integrated with the Memfault Linux SDK.

Useful links:

- [Introduction to Memfault for Linux][docs-linux-introduction]: a high-level
  introduction.
- [Getting started with Memfault on Linux][docs-linux-getting-started]: our
  integration guide.
- [Linux OTA Management][docs-linux-ota]: integrate with our over-the-air
  updates (OTA) system.
- [Linux Metrics][docs-linux-metrics]: integrate with our Metrics system.

## Contents

While following [the getting-started guide][docs-linux-getting-started], you'll
find these the most interesting:

- [`recipes-memfault/memfaultd`](recipes-memfault/memfaultd): shows a way to
  include a custom `/etc/memfaultd.conf` file.
- [`recipes-memfault/memfault-device-info`](recipes-memfault/memfault-device-info):
  an example `memfault-device-info` script. Note that your own implementation is
  likely to result in something more complex, since you'll need to output (e.g.)
  a device ID from your actual target devices.

If you're [integrating with OTA][docs-linux-ota], check out these:

- [`recipes-support/swupdate`](recipes-support/swupdate) contains an example
  `swupdate.cfg` file as well as a `defconfig` file with which to build SWUpdate
  so that hawkBit DDI API support is enabled. It also contains an include file
  `09-swupdate-args` which points SWUpdate at the Memfault-generated config.
- Other directories form a basic dual-copy SWUpdate setup. Your project will
  most likely differ from this, but nevertheless they are the following:
  - [`recipes-core/images`](recipes-core/images) contains a base image and an
    update image (`.swu`) which extends the base image, and is used as an OTA
    payload in the Memfault Web App.
  - [`recipes-core/images/files/sw-description.in`](recipes-core/images/files/sw-description.in)
    , [`wic` configuration](wic) and
    [`u-boot` configuration](recipes-bsp/u-boot) are all aligned, expecting a
    certain partition layout and configuring the update image to use it.

While [integrating with Metrics][docs-linux-metrics], you'll want to read the
following:

- [`recipes-extended/collectd`](recipes-extended/collectd) contains our
  recommended `/etc/collectd.conf` file, including a set of standard plugins
  that enjoy special support on the Memfault platform.
- Sample apps using a StatsD client that sends application metrics to CollectD:
  - In C:
    [`recipes-memfault/statsd-sampleapp-c`](recipes-memfault/statsd-sampleapp-c)
  - In Python:
    [`recipes-memfault/statsd-sampleapp-python`](recipes-memfault/statsd-sampleapp-python)

## Quick Start

### Create a Memfault Project

Go to [app.memfault.com](https://app.memfault.com) and from the "Select a
Project" dropdown, click on "Create Project". Once you're done, you can find a
project key, referenced as `YOUR_PROJECT_KEY` in this document, in the
[Project settings page](https://app.memfault.com/organizations/-/projects/-/settings).

### Prepare your environment

Check out [`env.list`](/docker/env.list) to see defaults. At a minimum, you'll
need `MEMFAULT_PROJECT_KEY` set in your environment:

```shell
export MEMFAULT_PROJECT_KEY=<YOUR_PROJECT_KEY>
```

### Create a Docker to build with Yocto

This example includes a [`Dockerfile`](/docker/Dockerfile) and a
[`run.sh` script](/docker/run.sh) to create a container.

```shell
$ cd /path/to/memfault-linux-sdk/docker
$ MEMFAULT_PROJECT_KEY=<YOUR_PROJECT_KEY> ./run.sh -b
$ bitbake memfault-image
```

Note that building the image for the first time will take around two hours.

### Run the image on QEMU

```shell
$ q # Run image in QEMU
login: root
```

### Inspect the integration

Restart your QEMU device and confirm that a reset appears on the Memfault web
app. To find it, look for your device in **Fleet -> Devices** and then find the
**Reboots** tab in its detail view.

## Docker

The Dockerfile and supporting files are all inside
[in the `/docker/` subfolder](/docker/).

### Docker image layout

- `/` : main container - Just contains the core operating system
- `~/yocto/build` : volume mount - Contains the build, images, etc.
- `~/yocto/sources` : volume mount - Contains the git clones.
- `~/yocto/sources/memfault-linux-sdk` : bind mount - Contains this repository
- `~/yocto/build/downloads` : Optional bind mount from host - Contains any
  packages downloaded via the yocto build, currently about 4.8GB

### Output Images

The final output of the bitbake build is stored at
`~/yocto/build/tmp/deploy/images/qemuarm64`; we are particularly interested in a
few core files:

- `u-boot.bin` - This is the DAS U-Boot binary, it is outside the Yocto
  filesystem due to limitations in the standard libvirt QEMU virtual machine.
  More usually this file would be in the first partition of the disk image
- `base-image-qemuarm64.wic` - This is the main disk image, it contains 3
  partitions:
  - `/dev/vda1`, vfat, contains the u-boot runtime configuration
  - `/dev/vda2`, ext4, the rootfs image
  - `/dev/vda3`, empty on first boot, used as the alternate rootfs
  - `/dev/vda4`, ext4, r/w media partition, used to store data which needs to
    persist over a system upgrade
- `memfault-image-qemuarm64.swu` - The SWUpdate package used to upgrade the
  complete rootfs system

This wic partition table is defined in `wic/memfault.wks`

## QEMU

QEMU is built as part of Yocto and doesn't require any additional packages to be
installed in the host Docker container. We provide a convenient wrapper script
around QEMU in [`test-scripts/runqemu.py`](/test-scripts/runqemu.py). You can
run that script directly or invoke it using our alias:

```
$ q
```

Note that the Yocto built-in `runqemu` is not compatible with our example
SWUpdate dual-copy setup in versions older than 4.0 'kirkstone'.

Login information for the QEMU environment:

- Username: root
- Password: _not required_

[docs-linux-introduction]: https://docs.memfault.com/docs/linux/introduction
[docs-linux-getting-started]: https://mflt.io/linux-getting-started
[docs-linux-metrics]: https://mflt.io/linux-metrics
[docs-linux-ota]: https://mflt.io/linux-ota-integration-guide
