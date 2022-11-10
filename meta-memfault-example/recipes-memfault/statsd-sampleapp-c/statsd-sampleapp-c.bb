DESCRIPTION = "StatsD Sample Application - C"
LICENSE = "MIT"
LIC_FILES_CHKSUM = "file://${COMMON_LICENSE_DIR}/MIT;md5=0835ade698e0bcf8506ecda2f7b4f302"

SRC_URI = " \
    file://main.c \
    file://Makefile \
"

DEPENDS = " \
    statsd-c-client \
"

S = "${WORKDIR}"

TARGET_CC_ARCH = "${LDFLAGS} ${TUNE_CCARGS}"

do_install () {
    install -d ${D}/usr/bin
    install -m 0755 ${S}/statsd-sampleapp-c ${D}/usr/bin
}
