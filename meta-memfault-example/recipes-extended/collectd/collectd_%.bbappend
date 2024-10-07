FILESEXTRAPATHS_prepend := "${THISDIR}/files:"

SRC_URI_append = " \
    file://collectd.conf \
"

do_install_append() {
    install -Dm 0644 ${WORKDIR}/collectd.conf ${D}${sysconfdir}/collectd.conf
}
