DESCRIPTION = "Memfault SWUpdate compound image, zchunk delta variant"

LICENSE = "MIT"
LIC_FILES_CHKSUM = "file://${COMMON_LICENSE_DIR}/MIT;md5=0835ade698e0bcf8506ecda2f7b4f302"

inherit swupdate

SRC_URI = "\
    file://sw-description-delta.in \
"

SWUPDATE_SRC_URI_EXCLUDE = "sw-description-delta.in"

IMAGE_DEPENDS = "base-image"

SWUPDATE_IMAGES = "base-image-${MACHINE}"

ZCK_IMAGE = "base-image-${MACHINE}${IMAGE_NAME_SUFFIX}.ext4.zck"

python() {
  if d.getVar("MEMFAULT_DELTA_OTA") != "1":
      raise bb.parse.SkipRecipe('set MEMFAULT_DELTA_OTA = "1" in conf/local.conf to '
                                "build the zchunk delta artifacts this .swu needs")
  suffix = d.getVar("IMAGE_NAME_SUFFIX") or ""
  d.appendVarFlag("SWUPDATE_IMAGES_FSTYPES", f"base-image-{d.getVar('MACHINE')}",
                  f"{suffix}.ext4.zck.zckheader")
}

do_swupdate_update_swdescription() {
    # Yocto dependency checking can be broken if we modify the source file
    # directly during the build process, create a 'output' file to modify
    cp ${WORKDIR}/sw-description-delta.in ${WORKDIR}/sw-description
    sed -i -e "s%__MEMFAULT_SOFTWARE_VERSION%${MEMFAULT_SOFTWARE_VERSION}%" ${WORKDIR}/sw-description
    sed -i -e "s%__MEMFAULT_HARDWARE_VERSION%${MEMFAULT_HARDWARE_VERSION}%" ${WORKDIR}/sw-description
    sed -i -e "s%__OTA_PARTITION_A%${OTA_PARTITION_A}%" ${WORKDIR}/sw-description
    sed -i -e "s%__OTA_PARTITION_B%${OTA_PARTITION_B}%" ${WORKDIR}/sw-description
    sed -i -e "s%__ZCK_HEADER_FILE%${ZCK_IMAGE}.zckheader%" ${WORKDIR}/sw-description
    sed -i -e "s%__ZCK_FILE%${ZCK_IMAGE}%" ${WORKDIR}/sw-description
}
addtask do_swupdate_update_swdescription before do_swuimage after do_unpack do_prepare_recipe_sysroot
