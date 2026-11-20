FILESEXTRAPATHS_prepend := "${THISDIR}/files-dunfell:"

SRC_URI_remove = "git://git.yoctoproject.org/pseudo;branch=oe-core"
SRC_URI_remove = "file://0001-configure-Prune-PIE-flags.patch"
SRC_URI_append = " git://git.yoctoproject.org/pseudo;protocol=https;branch=master"

SRCREV = "43cbd8fb4914328094ccdb4bb827d74b1bac2046"
PV = "1.9.3+git${SRCPV}"

TARGET_CPPFLAGS_remove = "-D_LARGEFILE_SOURCE -D_FILE_OFFSET_BITS=64"
