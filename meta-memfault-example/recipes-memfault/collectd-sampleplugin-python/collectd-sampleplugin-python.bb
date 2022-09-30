DESCRIPTION = "Sample collectd python plugin"
LICENSE = "MIT"
LIC_FILES_CHKSUM = "file://${COMMON_LICENSE_DIR}/MIT;md5=0835ade698e0bcf8506ecda2f7b4f302"

SRC_URI = " \
    file://sampleplugin.py \
"

RDEPENDS:${PN} = "collectd python3"

do_install () {
    install -d ${D}/${libdir}/collectd
    install -m 0755 ${WORKDIR}/sampleplugin.py ${D}/${libdir}/collectd
}

FILES:${PN} = "${libdir}/collectd/*"
