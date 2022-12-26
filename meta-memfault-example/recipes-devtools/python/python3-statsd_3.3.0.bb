DESCRIPTION = "A simple statsd client."
HOMEPAGE = "https://github.com/jsocol/pystatsd"
SECTION = "devel/python"
LICENSE = "MIT"
LIC_FILES_CHKSUM = "file://LICENSE;md5=4d8aa8ac1dc54b8aee4054bd5e5c61bd"

inherit setuptools3 pypi

SRC_URI[md5sum] = "b397ccf880f37cf099e775907ebf7a46"
SRC_URI[sha256sum] = "e3e6db4c246f7c59003e51c9720a51a7f39a396541cb9b147ff4b14d15b5dd1f"

BBCLASSEXTEND = "native nativesdk"
