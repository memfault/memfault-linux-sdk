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

inherit systemd cargo

SYSTEMD_SERVICE_${PN} = "memfaultd.service"

DEPENDS = "json-c systemd vim-native cmake-native"

PACKAGECONFIG ??= "plugin_coredump plugin_collectd plugin_reboot plugin_swupdate "
PACKAGECONFIG[plugin_coredump] = ""
PACKAGECONFIG[plugin_collectd] = ""
PACKAGECONFIG[plugin_reboot] = ""
PACKAGECONFIG[plugin_swupdate] = ""
PACKAGECONFIG[plugin_logging] = ""

# Tell Cargo to disable all plugins and only enable the ones we will use.
EXTRA_CARGO_FLAGS = "--no-default-features"

# Plugin Coredump
CARGO_FEATURES_append = " \
    ${@bb.utils.contains('PACKAGECONFIG', 'plugin_coredump', \
        'coredump', \
        '', \
    d)} \
"
RDEPENDS_append_${PN} = " \
    ${@bb.utils.contains('PACKAGECONFIG', 'plugin_coredump', \
        'util-linux-libuuid', \
        '', \
    d)} \
"

# Plugin Collectd
CARGO_FEATURES_append = " \
    ${@bb.utils.contains('PACKAGECONFIG', 'plugin_collectd', \
        'collectd', \
        '', \
    d)} \
"

# Plugin Reboot
CARGO_FEATURES_append = " \
    ${@bb.utils.contains('PACKAGECONFIG', 'plugin_reboot', \
        'reboot', \
        '', \
    d)} \
"
DEPENDS_append = " \
    ${@bb.utils.contains('PACKAGECONFIG', 'plugin_reboot', \
        'libubootenv', \
        '', \
    d)} \
"

# Plugin SWUpdate
CARGO_FEATURES_append = " \
    ${@bb.utils.contains('PACKAGECONFIG', 'plugin_swupdate', \
        'swupdate', \
        '', \
    d)} \
"
DEPENDS_append = " \
    ${@bb.utils.contains('PACKAGECONFIG', 'plugin_swupdate', \
        'libconfig', \
        '', \
    d)} \
"

# Plugin Logging
CARGO_FEATURES_append = " \
    ${@bb.utils.contains('PACKAGECONFIG', 'plugin_logging', \
        'logging', \
        '', \
    d)} \
"

do_install_append() {
    install -d ${D}/${systemd_unitdir}/system
    install -m 0644 ${WORKDIR}/memfaultd.service ${D}/${systemd_unitdir}/system

    # Cargo will build two binaries but we know they are the same.
    # To save space we replace memfaultctl with a symbolic link to memfaultd.
    rm ${D}/usr/bin/memfaultctl
    ln -s /usr/bin/memfaultd ${D}/usr/bin/memfaultctl

    # TODO: only install if plugin is enabled
    rm ${D}/usr/bin/memfault-core-handler
    mkdir -p ${D}/usr/sbin
    ln -s /usr/bin/memfaultd ${D}/usr/sbin/memfault-core-handler
}
