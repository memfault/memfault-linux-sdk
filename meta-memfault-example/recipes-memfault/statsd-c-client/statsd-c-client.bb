DESCRIPTION = "StatsD C Client - https://github.com/romanbsd/statsd-c-client"
LICENSE = "MIT"
LIC_FILES_CHKSUM = "file://LICENSE;md5=bb024dfc627ee8676cd1b66160668e77"

SRC_URI = " \
    git://github.com/romanbsd/statsd-c-client.git;protocol=https;branch=master;rev=08ecca678345f157e72a1db1446facb403cbeb65 \
    file://0001-add-soname-to-library.patch \
"

S = "${WORKDIR}/git"

TARGET_CC_ARCH = "${LDFLAGS} ${TUNE_CCARGS}"

do_install() {
    install -d ${D}${libdir}
    oe_soinstall ${S}/libstatsdclient.so.2.0.1 ${D}${libdir}

    install -d ${D}${includedir}
    install -Dm 644 ${S}/statsd-client.h ${D}${includedir}
}
