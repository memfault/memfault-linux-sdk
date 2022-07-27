FILESEXTRAPATHS:prepend := "${THISDIR}/files:"

SRC_URI:append = " \
    file://fstab.append \
    file://hosts.append \
"

do_install:append () {
    cat ${WORKDIR}/fstab.append >> ${D}${sysconfdir}/fstab
    cat ${WORKDIR}/hosts.append >> ${D}${sysconfdir}/hosts
}
