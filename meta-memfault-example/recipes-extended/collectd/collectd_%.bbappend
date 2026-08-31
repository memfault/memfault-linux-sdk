FILESEXTRAPATHS:prepend := "${THISDIR}/files:"

SRC_URI:append = " \
    file://collectd.conf \
"

do_install:append() {
    install -Dm 0644 ${UNPACKDIR}/collectd.conf ${D}${sysconfdir}/collectd.conf
}

# collectd autodetects plugins from whatever it finds in the sysroot, which on
# wrynose builds ~90 of them. Restrict it to the plugins collectd.conf loads,
# plus the cpufreq/filecount/processes ones downstream configs add on top. An
# --enable for a plugin whose dependencies are missing fails configure.
EXTRA_OECONF += " \
    --disable-all-plugins \
    --enable-aggregation \
    --enable-cpu \
    --enable-cpufreq \
    --enable-df \
    --enable-disk \
    --enable-filecount \
    --enable-interface \
    --enable-logfile \
    --enable-match_regex \
    --enable-memory \
    --enable-processes \
    --enable-target_set \
    --enable-uptime \
    --enable-write_http \
"
