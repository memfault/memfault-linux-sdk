FILESEXTRAPATHS:prepend := "${THISDIR}/files/${SOC_FAMILY}:${THISDIR}/files:"

SRC_URI += " \
    file://hosts.append \
    file://fstab.append \
"

do_install:append () {
    cat ${UNPACKDIR}/fstab.append >> ${D}${sysconfdir}/fstab
    cat ${UNPACKDIR}/hosts.append >> ${D}${sysconfdir}/hosts
}
