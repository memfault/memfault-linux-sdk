DESCRIPTION = "StatsD Sample Application - Python"
LICENSE = "MIT"
LIC_FILES_CHKSUM = "file://${COMMON_LICENSE_DIR}/MIT;md5=0835ade698e0bcf8506ecda2f7b4f302"

SRC_URI = " \
    file://statsd-sampleapp-python.py \
"

S = "${WORKDIR}"

DEPENDS = " \
    python3-statsd \
"

RDEPENDS:${PN} = " \
    python3-statsd \
"

do_install () {
    install -d ${D}${bindir}
    install -Dm 0755 ${S}/statsd-sampleapp-python.py ${D}${bindir}
}

FILES:${PN} = " \
    /usr/bin/* \
"
