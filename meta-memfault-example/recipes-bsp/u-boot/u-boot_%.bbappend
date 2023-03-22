FILESEXTRAPATHS:prepend := "${THISDIR}/files:"

# The patches are only used on qemu - For the raspberrypi3 we provide a custom boot script in rpi-uboot-scr
SRC_URI:append:qemuall = " \
    file://0001-env-in-fat-defconfig-${PV}.patch \
    file://0002-initr_env-delay-${PV}.patch \
    file://0003-memfault_boot-boot-commands.patch \
    file://fw_env.config \
"
