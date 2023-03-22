DESCRIPTION = "Memfault SWUpdate compound image"

LICENSE = "MIT"
LIC_FILES_CHKSUM = "file://${COMMON_LICENSE_DIR}/MIT;md5=0835ade698e0bcf8506ecda2f7b4f302"

inherit swupdate

SRC_URI = "\
    file://sw-description.in \
"

# images to build before building swupdate image
IMAGE_DEPENDS = "base-image"

# images and files that will be included in the .swu image
SWUPDATE_IMAGES = "base-image-${MACHINE}"

python() {
  d.appendVarFlag("SWUPDATE_IMAGES_FSTYPES", f"base-image-{d.getVar('MACHINE')}", ".ext4.gz")
}

do_swupdate_update_swdescription() {
    # Yocto dependency checking can be broken if we modify the source file
    # directly during the build process, create a 'output' file to modify
    cp ${WORKDIR}/sw-description.in ${WORKDIR}/sw-description
    sed -i -e "s%__MEMFAULT_SOFTWARE_VERSION%${MEMFAULT_SOFTWARE_VERSION}%" ${WORKDIR}/sw-description
    sed -i -e "s%__MEMFAULT_HARDWARE_VERSION%${MEMFAULT_HARDWARE_VERSION}%" ${WORKDIR}/sw-description
    sed -i -e "s%__OTA_PARTITION_A%${OTA_PARTITION_A}%" ${WORKDIR}/sw-description
    sed -i -e "s%__OTA_PARTITION_B%${OTA_PARTITION_B}%" ${WORKDIR}/sw-description
    sed -i -e "s%__MACHINE%${MACHINE}%" ${WORKDIR}/sw-description
}
addtask do_swupdate_update_swdescription before do_swuimage after do_unpack do_prepare_recipe_sysroot
