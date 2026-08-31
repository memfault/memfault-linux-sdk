DESCRIPTION = "Main Memfault wrapper target build"

LICENSE = "MIT"
LIC_FILES_CHKSUM = "file://${COMMON_LICENSE_DIR}/MIT;md5=0835ade698e0bcf8506ecda2f7b4f302"

inherit image

# Disable most of the image build process
do_rootfs[noexec] = "1"
do_rootfs_wicenv[noexec] = "1"
do_image[noexec] = "1"
do_image_wic[noexec] = "1"
do_image_ext3[noexec] = "1"
do_image_ext4[noexec] = "1"
do_image_tar[noexec] = "1"
do_image_complete[noexec] = "1"
do_image_complete_setscene[noexec] = "1"
do_image_qa[noexec] = "1"
do_image_qa_setscene[noexec] = "1"
do_build[noexec] = "1"

# SPDX SBOM generation depends on do_rootfs's output, which is disabled above.
do_create_rootfs_spdx[noexec] = "1"
do_create_rootfs_spdx_setscene[noexec] = "1"
do_create_image_spdx[noexec] = "1"
do_create_image_spdx_setscene[noexec] = "1"
do_create_image_sbom_spdx[noexec] = "1"
do_create_image_sbom_spdx_setscene[noexec] = "1"

do_image[depends] += "swupdate-image:do_swuimage base-image:do_image_complete"
