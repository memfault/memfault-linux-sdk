FILESEXTRAPATHS_prepend := "${THISDIR}/files:"

SRC_URI_append = " \
    file://fstab.append \
    file://hosts.append \
"

do_install_append () {
    cat ${WORKDIR}/fstab.append >> ${D}${sysconfdir}/fstab
    cat ${WORKDIR}/hosts.append >> ${D}${sysconfdir}/hosts
}
