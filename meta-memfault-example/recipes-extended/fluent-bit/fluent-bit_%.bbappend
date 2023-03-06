# Original: https://github.com/fluent/fluent-bit/blob/v2.0.8/fluent-bit_2.0.8.bb

# Modified by Memfault for newer bitbake compatibility
# 56,57c56,57
# < SYSTEMD_SERVICE_${PN} = "fluent-bit.service"
# < TARGET_CC_ARCH_append = " ${SELECTED_OPTIMIZATION}"
# ---
# > SYSTEMD_SERVICE_${PN} = "fluent-bit.service"
# > TARGET_CC_ARCH_append = " ${SELECTED_OPTIMIZATION}"

# Switch to https - git no longer recommended
SRC_URI = "git://github.com/fluent/fluent-bit.git;nobranch=1;protocol=https"
# Freeze revision
SRCREV = "9444fdc5ee858a42b58b9a97010d997096e63634"

# We do not use YAML and WASM runtime does not compile on 32 bit
EXTRA_OECMAKE += "-DFLB_CONFIG_YAML=Off -DFLB_WASM=Off"

FILESEXTRAPATHS_prepend := "${THISDIR}/files:"
SRC_URI_append = " file://fluent-bit.service file://fluent-bit.conf"

do_install_append() {
  install -d ${D}/${systemd_unitdir}/system
  install -m 0644 ${WORKDIR}/${PN}.service ${D}${systemd_unitdir}/system

  # Remove default config file and install our own in place
  rm ${D}/${sysconfdir}/${PN}/*
  install -m 0644 ${WORKDIR}/${PN}.conf ${D}${sysconfdir}/${PN}
}
