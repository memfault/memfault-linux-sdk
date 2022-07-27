# `meta-memfault-example`

This layer is an example implementation that builds a Yocto image for
`qemuarm64` that's fully integrated with the Memfault Linux SDK.

Useful links:

- [Introduction to Memfault for Linux][docs-linux-introduction]: a high-level
  introduction.
- [Getting started with Memfault on Linux][docs-linux-introduction]: our
  integration guide.

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
## Run image in QEMU
$ runqemu qemuarm64 slirp nographic
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
installed in the host Docker container. Yocto provides a convenient wrapper
script around QEMU called `runqemu`, in addition to this we need to set some
additional options:

```console
$ runqemu qemuarm64 slirp nographic
```

- `slirp` - Allows for user networking for the virtual machine
- `nographic` - Causes QEMU to run directly in the console window, without
  creating an additional window / VNC endpoint Experimental options:
- `qemuparams="-net nic -net user,smb=/home/build/yocto/build"` - This makes an
  SMB mountpoint of `/home/build/yocto/build` available to the virtual machine
  (seems to be failing 50% of the time, temporarily replaced by `tftp`)

Login information for the QEMU environment:

- Username: root
- Password: _not required_

[docs-linux-introduction]: https://docs.memfault.com/docs/linux/introduction
[docs-linux-getting-started]:
  https://docs.memfault.com/docs/linux/linux-getting-started-guide
