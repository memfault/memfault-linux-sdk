FILESEXTRAPATHS:prepend := "${THISDIR}/files:"

# The patches are only used on qemu - For the raspberrypi3 we provide a custom boot script in rpi-uboot-scr
# 0004/0005 are the bootstd memfault_boot route; the release packaging swaps in 0003 for older releases.
SRC_URI:append:qemuall = " \
    file://0001-env-in-fat-defconfig-${PV}.patch \
    file://0002-initr_env-delay-${PV}.patch \
    file://0004-qemu-arm-rootpart-env.patch \
    file://0005-memfault-bootcmd-defconfig-${PV}.patch \
    file://fw_env.config \
"

# Build the default env into a uboot.env image (see IMAGE_BOOT_FILES). Under
# bootstd nothing runs saveenv, so without this there is no env file for
# libubootenv (and so swupdate) to open. Must match CONFIG_ENV_SIZE and the
# size in fw_env.config.
UBOOT_INITIAL_ENV_BINARY = "1"
UBOOT_INITIAL_ENV_BINARY_SIZE = "0x4000"
