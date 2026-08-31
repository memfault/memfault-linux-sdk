# The hardware database is 9 MiB of vendor/product IDs. Nothing on this image
# looks devices up by ID, and udev runs without it.
EXTRA_OEMESON:append = " -Dhwdb=false"

# Both are hard RDEPENDS of systemd via PACKAGECONFIG. nss-resolve is only
# useful with systemd-resolved, which DISTRO_FEATURES does not enable, and
# myhostname duplicates the entry base-files already puts in /etc/hosts.
PACKAGECONFIG:remove = "myhostname nss-resolve"
