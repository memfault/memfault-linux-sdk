DESCRIPTION = "memfaultd application"
LICENSE = "Proprietary"
LICENSE_FLAGS = "commercial"
LIC_FILES_CHKSUM = "file://${FILE_DIRNAME}/../../../License.txt;md5=f10c502d265f86bd71f9dac8ec7827c2"

FILESEXTRAPATHS:prepend := "${FILE_DIRNAME}/../../:"

SRC_URI = " \
    file://memfaultc-sys \
    file://memfaultd \
    file://memfault-ssf \
    file://memfaultd.service \
    file://memfaultd.init \
    file://Cargo.toml \
    file://Cargo.lock \
    file://VERSION \
"

S = "${WORKDIR}"

inherit cargo_bin update-rc.d systemd pkgconfig

# rustc embeds the absolute build path in panic-location strings, which
# -ffile-prefix-map doesn't cover; cosmetic only.
INSANE_SKIP:${PN} += "buildpaths"
INSANE_SKIP:${PN}-dbg += "buildpaths"
INSANE_SKIP:${PN}-dev += "buildpaths"

SYSTEMD_SERVICE:${PN} = "memfaultd.service"
INITSCRIPT_NAME = "memfaultd"
# Sequence 15 places memfaultd after networking (01) and before collectd (20)
# and swupdate (70).
INITSCRIPT_PARAMS = "defaults 15"

DEPENDS = "zlib"

PACKAGECONFIG ??= "coredump swupdate logging chunks-relay"
PACKAGECONFIG[coredump] = ""
PACKAGECONFIG[collectd] = ""
PACKAGECONFIG[ebpf] = ""
PACKAGECONFIG[swupdate] = ""
PACKAGECONFIG[logging] = ""
PACKAGECONFIG[openssl-tls] = ""
PACKAGECONFIG[syslog] = ""
PACKAGECONFIG[chunks-relay] = ""

# Tell Cargo to disable all features and only enable the ones we will use.
EXTRA_CARGO_FLAGS = "--no-default-features"

# Coredump
CARGO_FEATURES:append = " \
    ${@bb.utils.contains('PACKAGECONFIG', 'coredump', \
        'coredump', \
        '', \
    d)} \
"

# SWUpdate
CARGO_FEATURES:append = " \
    ${@bb.utils.contains('PACKAGECONFIG', 'swupdate', \
        'swupdate', \
        '', \
    d)} \
"
DEPENDS:append = " \
    ${@bb.utils.contains('PACKAGECONFIG', 'swupdate', \
        'libconfig', \
        '', \
    d)} \
"
RRECOMMENDS:${PN}:append = " \
    ${@bb.utils.contains('PACKAGECONFIG', 'swupdate', \
        'swupdate swupdate-tools-ipc swupdate-tools-hawkbit', \
        '', \
    d)} \
"

# Logging
CARGO_FEATURES:append = " \
    ${@bb.utils.contains('PACKAGECONFIG', 'logging', \
        'logging', \
        '', \
    d)} \
"

# OpenSSL is not the default as of v1.16
CARGO_FEATURES:append = " \
    ${@bb.utils.contains('PACKAGECONFIG', 'openssl-tls', \
        'openssl-tls', \
        'rust-tls', \
    d)} \
"
DEPENDS:append = " \
    ${@bb.utils.contains('PACKAGECONFIG', 'openssl-tls', \
        'openssl', \
        '', \
    d)} \
"

# Systemd is added automatically when the system is built with it
CARGO_FEATURES:append = " \
    ${@bb.utils.contains('DISTRO_FEATURES', 'systemd', \
        'systemd', \
        '', \
    d)} \
"
DEPENDS:append = " \
    ${@bb.utils.contains('DISTRO_FEATURES', 'systemd', \
        'systemd', \
        '', \
    d)} \
"

# Syslog
CARGO_FEATURES:append = " \
    ${@bb.utils.contains('PACKAGECONFIG', 'syslog', \
        'syslog', \
        '', \
    d)} \
"

# eBPF
CARGO_FEATURES:append = " \
    ${@bb.utils.contains('PACKAGECONFIG', 'ebpf', \
        'ebpf', \
        '', \
    d)} \
"
DEPENDS:append = " \
    ${@bb.utils.contains('PACKAGECONFIG', 'ebpf', \
        'libbpf', \
        '', \
    d)} \
"

# Chunks relay (write-chunks)
CARGO_FEATURES:append = " \
    ${@bb.utils.contains('PACKAGECONFIG', 'chunks-relay', \
        'chunks-relay', \
        '', \
    d)} \
"

# Network access required to download Cargo dependencies
do_compile[network] = "1"

do_install:append() {
    # Start/Stop script for Systemd
    install -d ${D}/${systemd_unitdir}/system
    install -m 0644 ${WORKDIR}/memfaultd.service ${D}/${systemd_unitdir}/system
    # Start/Stop script for SysVInit
    install -d ${D}${sysconfdir}/init.d
    install -m 755 ${WORKDIR}/memfaultd.init ${D}${sysconfdir}/init.d/memfaultd

    # Cargo will build two binaries but we know they are the same.
    # To save space we replace memfaultctl with a symbolic link to memfaultd.
    rm ${D}/usr/bin/memfaultctl
    ln -sf /usr/bin/memfaultd ${D}/usr/bin/memfaultctl

    rm ${D}/usr/bin/memfault-core-handler
    mkdir -p ${D}/usr/sbin
    ln -sf /usr/bin/memfaultd ${D}/usr/sbin/memfault-core-handler
}
