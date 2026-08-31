FILESEXTRAPATHS:prepend := "${THISDIR}/files:"

SRC_URI:append = " \
    file://memfaultd_example.conf.in \
"

do_install:append() {
    # Yocto dependency checking can be broken if we modify the source file
    # directly during the build process, create a 'output' file to modify
    cp ${UNPACKDIR}/memfaultd_example.conf.in ${UNPACKDIR}/memfaultd_example.conf
    sed -i -e "s%MEMFAULT_BASE_URL%${MEMFAULT_BASE_URL}%" ${UNPACKDIR}/memfaultd_example.conf
    sed -i -e "s%MEMFAULT_PROJECT_KEY%${MEMFAULT_PROJECT_KEY}%" ${UNPACKDIR}/memfaultd_example.conf
    sed -i -e "s%MEMFAULT_SOFTWARE_TYPE%${MEMFAULT_SOFTWARE_TYPE}%" ${UNPACKDIR}/memfaultd_example.conf
    sed -i -e "s%MEMFAULT_SOFTWARE_VERSION%${MEMFAULT_SOFTWARE_VERSION}%" ${UNPACKDIR}/memfaultd_example.conf

    install -m 0644 ${UNPACKDIR}/memfaultd_example.conf ${D}${sysconfdir}/memfaultd.conf
}
