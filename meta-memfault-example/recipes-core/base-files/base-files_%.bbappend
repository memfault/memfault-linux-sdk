FILESEXTRAPATHS:prepend := "${THISDIR}/files/${SOC_FAMILY}:${THISDIR}/files:"

SRC_URI += " \
    file://hosts.append \
    file://fstab.append \
"

do_install:append () {
    cat ${WORKDIR}/fstab.append >> ${D}${sysconfdir}/fstab
    cat ${WORKDIR}/hosts.append >> ${D}${sysconfdir}/hosts
}
