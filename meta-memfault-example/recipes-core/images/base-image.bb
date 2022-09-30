require recipes-core/images/core-image-minimal.bb

CORE_IMAGE_EXTRA_INSTALL += "lsb-release"

LICENSE = "MIT"
LIC_FILES_CHKSUM = "file://${COMMON_LICENSE_DIR}/MIT;md5=0835ade698e0bcf8506ecda2f7b4f302"

inherit buildhistory

IMAGE_INSTALL:append = " \
    cifs-utils \
    collectd-sampleplugin-python \
    kernel-modules \
    memfault-device-info \
    netcat \
    statsd-sampleapp-python \
    statsd-sampleapp-c \
    u-boot-env \
    u-boot-fw-utils \
"

# Create a copy of the .wic image. This is used as the "pristine" image by the E2E test scripts.
do_copy_wic_image() {
    if [ -f ${IMGDEPLOYDIR}/${PN}-${MACHINE}.wic ]; then
        cp -f ${IMGDEPLOYDIR}/${PN}-${MACHINE}.wic ${DEPLOY_DIR_IMAGE}/ci-test-image.wic
    fi
}
addtask copy_wic_image after do_image_wic before do_image_complete

IMAGE_POSTPROCESS_COMMAND:append = " buildhistory_get_imageinfo;"

# Support memfault-cli upload-yocto-symbols command
DEPENDS:append = " elfutils-native"
IMAGE_GEN_DEBUGFS = "1"
# IMAGE_FSTYPES_DEBUGFS must match IMAGE_FSTYPES
IMAGE_FSTYPES_DEBUGFS = "tar.bz2"
