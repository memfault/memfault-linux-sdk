DESCRIPTION = "memfaultd application"
LICENSE = "Proprietary"
LICENSE_FLAGS = "commercial"
LIC_FILES_CHKSUM = "file://${FILE_DIRNAME}/../../../License.txt;md5=56e72796d5838e30cf6671385527172c"

FILESEXTRAPATHS:append := "${FILE_DIRNAME}/../../:"

SRC_URI = " \
    file://memfaultd \
    file://memfaultd.service \
    file://VERSION \
"

S = "${WORKDIR}/memfaultd"

inherit systemd pkgconfig cmake

SYSTEMD_AUTO_ENABLE = "enable"
SYSTEMD_SERVICE:${PN} = "memfaultd.service"

DEPENDS = "curl json-c vim-native"

PACKAGECONFIG ??= "plugin_reboot plugin_swupdate"
PACKAGECONFIG[plugin_reboot] = "-DENABLE_PLUGINS=1 -DPLUGIN_REBOOT=1"
PACKAGECONFIG[plugin_swupdate] = "-DENABLE_PLUGINS=1 -DPLUGIN_SWUPDATE=1"

DEPENDS:append = " \
    ${@bb.utils.contains('PACKAGECONFIG', 'plugin_reboot', \
        'libubootenv systemd', \
        '', \
    d)} \
"

DEPENDS:append = " \
    ${@bb.utils.contains('PACKAGECONFIG', 'plugin_swupdate', \
        'libconfig systemd', \
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

CFLAGS += " ${@get_cflags(d)}"

EXTRA_OECMAKE = "-DTESTS=0"

do_install:append () {
    install -d ${D}/${systemd_unitdir}/system
    install -m 0644 ${WORKDIR}/memfaultd.service ${D}/${systemd_unitdir}/system
}
