DESCRIPTION = "memfaultd application"
LICENSE = "Proprietary"
LICENSE_FLAGS = "commercial"
LIC_FILES_CHKSUM = "file://${FILE_DIRNAME}/../../../License.txt;md5=f10c502d265f86bd71f9dac8ec7827c2"

FILESEXTRAPATHS:prepend := "${FILE_DIRNAME}/../../:"

SRC_URI = " \
    file://memfaultd \
    file://memfaultd.service \
    file://VERSION \
"

S = "${WORKDIR}/memfaultd"

inherit systemd pkgconfig cmake

SYSTEMD_AUTO_ENABLE = "enable"
SYSTEMD_SERVICE:${PN} = "memfaultd.service"

DEPENDS = "curl json-c systemd vim-native"

PACKAGECONFIG ??= "plugin_coredump plugin_collectd plugin_reboot plugin_swupdate"
PACKAGECONFIG[plugin_coredump] = "-DPLUGIN_COREDUMP=1"
PACKAGECONFIG[plugin_collectd] = "-DPLUGIN_COLLECTD=1"
PACKAGECONFIG[plugin_reboot] = "-DPLUGIN_REBOOT=1"
PACKAGECONFIG[plugin_swupdate] = "-DPLUGIN_SWUPDATE=1"

RDEPENDS:append:${PN} = " \
    ${@bb.utils.contains('PACKAGECONFIG', 'plugin_coredump', \
        'util-linux-libuuid', \
        '', \
    d)} \
"

DEPENDS:append = " \
    ${@bb.utils.contains('PACKAGECONFIG', 'plugin_reboot', \
        'libubootenv', \
        '', \
    d)} \
"

DEPENDS:append = " \
    ${@bb.utils.contains('PACKAGECONFIG', 'plugin_swupdate', \
        'libconfig', \
        '', \
    d)} \
"

def get_cflags(d):
    ret = []
    versionFile = d.expand("${FILE_DIRNAME}") + "/../../VERSION"
    if os.path.exists(versionFile):
        with open(versionFile) as file:
            for line in file.readlines():
                ret.append(" -D" + line.strip().replace(" ", "").replace(":", "="))
    return ''.join(ret)

CFLAGS = "${@get_cflags(d)}"

EXTRA_OECMAKE = "-DTESTS=0"

do_install:append() {
    install -d ${D}/${systemd_unitdir}/system
    install -m 0644 ${WORKDIR}/memfaultd.service ${D}/${systemd_unitdir}/system
}
