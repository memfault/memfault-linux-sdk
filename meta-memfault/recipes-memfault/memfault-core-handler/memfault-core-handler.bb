DESCRIPTION = "memfault-core-handler application"
LICENSE = "Proprietary"
LICENSE_FLAGS = "commercial"
LIC_FILES_CHKSUM = "file://${FILE_DIRNAME}/../../../License.txt;md5=56e72796d5838e30cf6671385527172c"

SRC_URI = " \
    file://memfault-core-handler \
"

S = "${WORKDIR}/memfault-core-handler"

inherit cmake
