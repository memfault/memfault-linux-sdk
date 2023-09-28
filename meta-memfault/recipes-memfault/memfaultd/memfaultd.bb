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
    file://memfaultd.init \
    file://Cargo.toml \
    file://Cargo.lock \
    file://VERSION \
"

S = "${WORKDIR}"

inherit cargo_bin update-rc.d systemd

SYSTEMD_SERVICE_${PN} = "memfaultd.service"
INITSCRIPT_NAME = "memfaultd"
# Sequence 15 places memfaultd after networking (01) and before collectd (20)
# and swupdate (70).
INITSCRIPT_PARAMS = "defaults 15"

DEPENDS = "vim-native cmake-native zlib"

PACKAGECONFIG ??= "coredump collectd swupdate logging"
PACKAGECONFIG[coredump] = ""
PACKAGECONFIG[collectd] = ""
PACKAGECONFIG[swupdate] = ""
PACKAGECONFIG[logging] = ""
PACKAGECONFIG[openssl-tls] = ""

# Tell Cargo to disable all features and only enable the ones we will use.
EXTRA_CARGO_FLAGS = "--no-default-features"

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

# OpenSSL is not the default as of v1.16
CARGO_FEATURES_append = " \
    ${@bb.utils.contains('PACKAGECONFIG', 'openssl-tls', \
        'openssl-tls', \
        'rust-tls', \
    d)} \
"
DEPENDS_append = " \
    ${@bb.utils.contains('PACKAGECONFIG', 'openssl-tls', \
        'openssl', \
        '', \
    d)} \
"

# Systemd is added automatically when the system is built with it
CARGO_FEATURES_append = " \
    ${@bb.utils.contains('DISTRO_FEATURES', 'systemd', \
        'systemd', \
        '', \
    d)} \
"
DEPENDS_append = " \
    ${@bb.utils.contains('DISTRO_FEATURES', 'systemd', \
        'systemd', \
        '', \
    d)} \
"

# Network access required to download Cargo dependencies
do_compile[network] = "1"

do_install_append() {
    # Start/Stop script for Systemd
    install -d ${D}/${systemd_unitdir}/system
    install -m 0644 ${WORKDIR}/memfaultd.service ${D}/${systemd_unitdir}/system
    # Start/Stop script for SysVInit
    install -d ${D}${sysconfdir}/init.d
    install -m 755 ${WORKDIR}/memfaultd.init ${D}${sysconfdir}/init.d/memfaultd

    # Cargo will build two binaries but we know they are the same.
    # To save space we replace memfaultctl with a symbolic link to memfaultd.
    rm ${D}/usr/bin/memfaultctl
    ln -s /usr/bin/memfaultd ${D}/usr/bin/memfaultctl

    # TODO: only install if feature is enabled
    rm ${D}/usr/bin/memfault-core-handler
    mkdir -p ${D}/usr/sbin
    ln -s /usr/bin/memfaultd ${D}/usr/sbin/memfault-core-handler
}
