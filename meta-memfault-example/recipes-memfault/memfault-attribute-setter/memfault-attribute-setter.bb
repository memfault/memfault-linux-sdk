DESCRIPTION = "Example service that sets Memfault Attributes"
LICENSE = "MIT"
LIC_FILES_CHKSUM = "file://${COMMON_LICENSE_DIR}/MIT;md5=0835ade698e0bcf8506ecda2f7b4f302"

SRC_URI = " \
    file://memfault-attribute-setter \
    file://10-memfault-attribute-setter.conf \
    file://memfault-attribute-setter.service \
"

S = "${WORKDIR}"

RDEPENDS_${PN} = " \
    memfaultd \
"

do_install () {
    install -d ${D}${bindir}
    install -Dm 0755 ${S}/memfault-attribute-setter ${D}${bindir}

    install -d ${D}/${systemd_unitdir}/system
    install -d ${D}/${systemd_unitdir}/system/memfaultd.service.d 
    
    # Install systemd unit file
    install -m 0644 ${WORKDIR}/memfault-attribute-setter.service ${D}/${systemd_unitdir}/system/

    # Install drop-in file that establishes that memfaultd wants memfault-attribute-setter     
    install -m 0644 ${WORKDIR}/10-memfault-attribute-setter.conf ${D}/${systemd_unitdir}/system/memfaultd.service.d     
    

}


FILES_${PN} = " \
    ${systemd_unitdir}/system/memfaultd.service.d/10-memfault-attribute-setter.conf \
    ${systemd_unitdir}/system/memfault-attribute-setter.service \
    ${bindir}/memfault-attribute-setter \ 
"
