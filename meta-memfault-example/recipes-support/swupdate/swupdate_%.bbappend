FILESEXTRAPATHS:prepend := "${THISDIR}/files:"

SRC_URI:append = " \
    file://09-swupdate-args.in \
    file://swupdate.cfg \
    file://defconfig \
"

do_swupdate_args_update() {
    # Yocto dependency checking can be broken if we modify the source file
    # directly during the build process, create a 'output' file to modify
    cp ${UNPACKDIR}/09-swupdate-args.in ${UNPACKDIR}/09-swupdate-args
    sed -i -e "s%__MEMFAULT_HARDWARE_VERSION%${MEMFAULT_HARDWARE_VERSION}%" ${UNPACKDIR}/09-swupdate-args
    sed -i -e "s%__OTA_PARTITION_A%${OTA_PARTITION_A}%" ${UNPACKDIR}/09-swupdate-args
}
addtask do_swupdate_args_update before do_install after do_unpack


do_install:append() {
    install -Dm 0644 ${UNPACKDIR}/09-swupdate-args ${D}${libdir}/swupdate/conf.d/09-swupdate-args
    install -Dm 0644 ${UNPACKDIR}/swupdate.cfg ${D}${sysconfdir}/swupdate.cfg
}

INSANE_SKIP:${PN} += "buildpaths"
