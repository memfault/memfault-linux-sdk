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
SWUPDATE_IMAGES = "base-image-qemuarm64"

SWUPDATE_IMAGES_FSTYPES[base-image-qemuarm64] = ".ext4.gz"

do_swupdate_update_swdescription() {
    # Yocto dependency checking can be broken if we modify the source file
    # directly during the build process, create a 'output' file to modify
    cp ${WORKDIR}/sw-description.in ${WORKDIR}/sw-description
    sed -i -e "s%__MEMFAULT_SOFTWARE_VERSION%${MEMFAULT_SOFTWARE_VERSION}%" ${WORKDIR}/sw-description
}
addtask do_swupdate_update_swdescription before do_swuimage after do_unpack do_prepare_recipe_sysroot

# Create a copy of the .wic image. This is used as the "pristine" image by the E2E test scripts.
do_copy_wic_image() {
    cp -f ${DEPLOY_DIR_IMAGE}/base-image-${MACHINE}.wic ${DEPLOY_DIR_IMAGE}/ci-test-image.wic
}
do_copy_wic_image[depends] = "${IMAGE_DEPENDS}:do_build"
addtask do_copy_wic_image before do_swuimage after do_unpack do_prepare_recipe_sysroot