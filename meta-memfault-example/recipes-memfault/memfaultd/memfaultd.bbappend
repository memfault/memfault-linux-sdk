# Turn on log-to-metrics for meta-memfault-memfaultd_example
# This feature is not enabled by default in meta-memfault.
CARGO_FEATURES:append = " log-to-metrics"

FILESEXTRAPATHS:prepend := "${THISDIR}/files:"

SRC_URI:append = " \
    file://memfaultd_example.conf.in \
"

do_install:append() {
    # Yocto dependency checking can be broken if we modify the source file
    # directly during the build process, create a 'output' file to modify
    cp ${WORKDIR}/memfaultd_example.conf.in ${WORKDIR}/memfaultd_example.conf
    sed -i -e "s%MEMFAULT_BASE_URL%${MEMFAULT_BASE_URL}%" ${WORKDIR}/memfaultd_example.conf
    sed -i -e "s%MEMFAULT_PROJECT_KEY%${MEMFAULT_PROJECT_KEY}%" ${WORKDIR}/memfaultd_example.conf
    sed -i -e "s%MEMFAULT_SOFTWARE_TYPE%${MEMFAULT_SOFTWARE_TYPE}%" ${WORKDIR}/memfaultd_example.conf
    sed -i -e "s%MEMFAULT_SOFTWARE_VERSION%${MEMFAULT_SOFTWARE_VERSION}%" ${WORKDIR}/memfaultd_example.conf

    install -m 0644 ${WORKDIR}/memfaultd_example.conf ${D}${sysconfdir}/memfaultd.conf
}
