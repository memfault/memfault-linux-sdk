require recipes-core/images/core-image-minimal.bb

CORE_IMAGE_EXTRA_INSTALL += "lsb-release"

LICENSE = "MIT"
LIC_FILES_CHKSUM = "file://${COMMON_LICENSE_DIR}/MIT;md5=0835ade698e0bcf8506ecda2f7b4f302"

IMAGE_INSTALL:append = " \
    cifs-utils \
    collectd-sampleplugin-python \
    kernel-modules \
    memfault-device-info \
    statsd-sampleapp-python \
    statsd-sampleapp-c \
    u-boot-env \
    u-boot-fw-utils \
"
