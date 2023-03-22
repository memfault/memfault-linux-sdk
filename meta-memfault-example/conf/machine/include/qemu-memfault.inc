WKS_FILE = "image-qemu.wks"

SOC_FAMILY = "qemuall"
include conf/machine/include/soc-family.inc

EXTRA_IMAGEDEPENDS:append = " virtual/bootloader"
PREFERRED_PROVIDER_virtual/bootloader = "u-boot"
IMAGE_BOOT_FILES = "u-boot.bin boot.scr"

# Support runqemu script
QB_DEFAULT_FSTYPE = "wic"
QB_DRIVE_TYPE = "/dev/vdb"
QB_SYSTEM_NAME = "qemu-system-arm"
QB_DEFAULT_BIOS = "u-boot.bin"

# Used to generate the swupdate update description file
OTA_PARTITION_A = "vda2"
OTA_PARTITION_B = "vda3"
