FILESEXTRAPATHS:append := "${THISDIR}/files:"

SRC_URI:append = " \
    file://collectd.conf \
    file://collectd-setinterval \
"

do_install:append() {
    install -Dm 0644 ${WORKDIR}/collectd.conf ${D}${sysconfdir}/collectd.conf
    install -Dm 0755 ${WORKDIR}/collectd-setinterval ${D}${bindir}/collectd-setinterval
}
