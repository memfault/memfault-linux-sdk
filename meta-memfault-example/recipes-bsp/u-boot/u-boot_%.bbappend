FILESEXTRAPATHS:prepend := "${THISDIR}/files:"

SRC_URI:append = " \
    file://0001-env-in-fat-defconfig.patch \
    file://0002-initr_env-delay.patch \
    file://0003-memfault_boot-boot-commands.patch \
    file://fw_env.config \
"

#do_deploy[depends] = " dosfstools-native:do_populate_sysroot coreutils-native:do_populate_sysroot util-linux-native:do_populate_sysroot"
#do_deploy:append:qemuall() {
#    dd if=/dev/zero count=65536 bs=1024 of=${DEPLOYDIR}/u-boot-envstore.img
#    echo "/dev/vdb1 : start= 2048, size= 129024, type=b" | sfdisk ${DEPLOYDIR}/u-boot-envstore.img
#    mkfs.vfat -C ${DEPLOYDIR}/tmp.img 64512
#    dd if=${DEPLOYDIR}/tmp.img seek=1024 bs=1024 of=${DEPLOYDIR}/u-boot-envstore.img
#    rm ${DEPLOYDIR}/tmp.img
#}
