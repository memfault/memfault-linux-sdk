FILESEXTRAPATHS:append := "${THISDIR}/files:"

PACKAGECONFIG_CONFARGS = ""

SRC_URI:append = " \
    file://09-swupdate-args \
    file://swupdate.cfg \
    file://defconfig \
"

do_install:append() {
    install -Dm 0644 ${WORKDIR}/09-swupdate-args ${D}${libdir}/swupdate/conf.d/09-swupdate-args
    install -Dm 0644 ${WORKDIR}/swupdate.cfg ${D}${sysconfdir}/swupdate.cfg
}

