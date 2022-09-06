FILESEXTRAPATHS:prepend := "${THISDIR}/files:"

SRC_URI:append = " \
    file://collectd.conf \
"

do_install:append() {
    install -Dm 0644 ${WORKDIR}/collectd.conf ${D}${sysconfdir}/collectd.conf
}
