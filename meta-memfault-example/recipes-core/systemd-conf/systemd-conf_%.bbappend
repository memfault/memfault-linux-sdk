SUMMARY = "Replace default journald config to disable syslog and limit size"
FILESEXTRAPATHS:prepend := "${THISDIR}/files:"

SRC_URI += "file://journald-memfault-example.conf"

do_install:append() {
	install -D -m0644 ${WORKDIR}/journald-memfault-example.conf ${D}${systemd_unitdir}/journald.conf.d/10-memfault-example.conf
}
