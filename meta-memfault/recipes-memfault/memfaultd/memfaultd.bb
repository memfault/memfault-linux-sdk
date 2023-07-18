DESCRIPTION = "memfaultd application"
LICENSE = "Proprietary"
LICENSE_FLAGS = "commercial"
LIC_FILES_CHKSUM = "file://${FILE_DIRNAME}/../../../License.txt;md5=f10c502d265f86bd71f9dac8ec7827c2"

FILESEXTRAPATHS_prepend := "${FILE_DIRNAME}/../../:"

SRC_URI = " \
    file://libmemfaultc \
    file://memfaultc-sys \
    file://memfaultd \
    file://memfaultd.service \
    file://Cargo.toml \
    file://Cargo.lock \
    file://VERSION \
"

S = "${WORKDIR}"

inherit systemd cargo_bin

SYSTEMD_SERVICE_${PN} = "memfaultd.service"

DEPENDS = "systemd vim-native cmake-native openssl"

PACKAGECONFIG ??= "coredump collectd swupdate logging"
PACKAGECONFIG[coredump] = ""
PACKAGECONFIG[collectd] = ""
PACKAGECONFIG[swupdate] = ""
PACKAGECONFIG[logging] = ""

# Tell Cargo to disable all features and only enable the ones we will use.
EXTRA_CARGO_FLAGS = "--no-default-features"

# Always include the systemd feature so we have a working service manager
CARGO_FEATURES_append = " systemd"

# Coredump
CARGO_FEATURES_append = " \
    ${@bb.utils.contains('PACKAGECONFIG', 'coredump', \
        'coredump', \
        '', \
    d)} \
"

# Collectd
CARGO_FEATURES_append = " \
    ${@bb.utils.contains('PACKAGECONFIG', 'collectd', \
        'collectd', \
        '', \
    d)} \
"
RRECOMMENDS_${PN} += " \
    ${@bb.utils.contains('PACKAGECONFIG', 'collectd', \
        'collectd', \
        '', \
    d)} \
"

# SWUpdate
CARGO_FEATURES_append = " \
    ${@bb.utils.contains('PACKAGECONFIG', 'swupdate', \
        'swupdate', \
        '', \
    d)} \
"
DEPENDS_append = " \
    ${@bb.utils.contains('PACKAGECONFIG', 'swupdate', \
        'libconfig', \
        '', \
    d)} \
"
RRECOMMENDS_${PN}_append = " \
    ${@bb.utils.contains('PACKAGECONFIG', 'swupdate', \
        'swupdate swupdate-tools-ipc swupdate-tools-hawkbit', \
        '', \
    d)} \
"

# Logging
CARGO_FEATURES_append = " \
    ${@bb.utils.contains('PACKAGECONFIG', 'logging', \
        'logging', \
        '', \
    d)} \
"
RRECOMMENDS_${PN} += " \
    ${@bb.utils.contains('PACKAGECONFIG', 'logging', \
        'fluent-bit', \
        '', \
    d)} \
"

# Network access required to download Cargo dependencies
do_compile[network] = "1"

do_install_append() {
    install -d ${D}/${systemd_unitdir}/system
    install -m 0644 ${WORKDIR}/memfaultd.service ${D}/${systemd_unitdir}/system

    # Cargo will build two binaries but we know they are the same.
    # To save space we replace memfaultctl with a symbolic link to memfaultd.
    rm ${D}/usr/bin/memfaultctl
    ln -s /usr/bin/memfaultd ${D}/usr/bin/memfaultctl

    # TODO: only install if feature is enabled
    rm ${D}/usr/bin/memfault-core-handler
    mkdir -p ${D}/usr/sbin
    ln -s /usr/bin/memfaultd ${D}/usr/sbin/memfault-core-handler
}
