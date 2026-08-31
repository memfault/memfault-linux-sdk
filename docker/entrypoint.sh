#!/bin/bash -exu

# YOCTO_RELEASE environment variable is expected to be set to i.e. "kirkstone", "dunfell", ...

poky_dir="${HOME}/yocto/sources/poky"

# Yocto 6.0+ (wrynose+) dropped the poky combo-repo; assemble oe-core, bitbake, and
# meta-yocto directly instead.
case "${YOCTO_RELEASE}" in
  dunfell | kirkstone | scarthgap)
    templateconf="../memfault-linux-sdk/meta-memfault-example/conf/"
    if [ ! -d "${poky_dir}" ]; then
      git clone https://git.yoctoproject.org/poky --branch "${YOCTO_RELEASE}" "${poky_dir}"
    else
      git -C "${poky_dir}" remote set-url origin https://git.yoctoproject.org/poky
      git -C "${poky_dir}" checkout "${YOCTO_RELEASE}" && git -C "${poky_dir}" pull --ff-only
    fi
    ;;
  *)
    # oe-setup-builddir now requires TEMPLATECONF under conf/templates/<name>/.
    templateconf="../memfault-linux-sdk/meta-memfault-example/conf/templates/default/"
    if [ ! -d "${poky_dir}" ]; then
      git clone https://git.openembedded.org/openembedded-core --branch "${YOCTO_RELEASE}" "${poky_dir}"
    else
      git -C "${poky_dir}" remote set-url origin https://git.openembedded.org/openembedded-core
      git -C "${poky_dir}" checkout "${YOCTO_RELEASE}" && git -C "${poky_dir}" pull --ff-only
    fi

    # bitbake is versioned numerically, not by codename; 2.18 is oe-core's minimum for wrynose.
    bitbake_dir="${poky_dir}/bitbake"
    bitbake_branch="2.18"
    if [ ! -d "${bitbake_dir}" ]; then
      git clone https://git.openembedded.org/bitbake --branch "${bitbake_branch}" "${bitbake_dir}"
    else
      git -C "${bitbake_dir}" remote set-url origin https://git.openembedded.org/bitbake
      git -C "${bitbake_dir}" checkout "${bitbake_branch}" && git -C "${bitbake_dir}" pull --ff-only
    fi

    meta_yocto_dir="${HOME}/yocto/sources/meta-yocto"
    if [ ! -d "${meta_yocto_dir}" ]; then
      git clone https://git.yoctoproject.org/meta-yocto --branch "${YOCTO_RELEASE}" "${meta_yocto_dir}"
    else
      git -C "${meta_yocto_dir}" remote set-url origin https://git.yoctoproject.org/meta-yocto
      git -C "${meta_yocto_dir}" checkout "${YOCTO_RELEASE}" && git -C "${meta_yocto_dir}" pull --ff-only
    fi
    # bblayers.conf.sample expects meta-poky/meta-yocto-bsp inside poky_dir.
    ln -sfn "${meta_yocto_dir}/meta-poky" "${poky_dir}/meta-poky"
    ln -sfn "${meta_yocto_dir}/meta-yocto-bsp" "${poky_dir}/meta-yocto-bsp"
    ;;
esac

openembedded_dir="${HOME}/yocto/sources/meta-openembedded"
if [ ! -d "${openembedded_dir}" ]; then
  git clone https://github.com/openembedded/meta-openembedded.git --branch "${YOCTO_RELEASE}" "${openembedded_dir}"
else
  git -C "${openembedded_dir}" checkout "${YOCTO_RELEASE}" && git -C "${openembedded_dir}" pull --ff-only
fi

swupdate_dir="${HOME}/yocto/sources/meta-swupdate"
if [ ! -d "${swupdate_dir}" ]; then
  git clone https://github.com/sbabic/meta-swupdate.git --branch "${YOCTO_RELEASE}" "${swupdate_dir}"
else
  git -C "${swupdate_dir}" checkout "${YOCTO_RELEASE}" && git -C "${swupdate_dir}" pull --ff-only
fi

raspberrypi_dir="${HOME}/yocto/sources/meta-raspberrypi"
if [ ! -d "${raspberrypi_dir}" ]; then
  git clone https://git.yoctoproject.org/meta-raspberrypi --branch "${YOCTO_RELEASE}" "${raspberrypi_dir}"
else
  git -C "${raspberrypi_dir}" remote set-url origin https://git.yoctoproject.org/meta-raspberrypi
  git -C "${raspberrypi_dir}" checkout "${YOCTO_RELEASE}" && git -C "${raspberrypi_dir}" pull --ff-only
fi

rust_bin_dir="${HOME}/yocto/sources/meta-rust-bin"
# meta-rust-bin has no release branches, so tracking master lets the rust
# toolchain recipes change under us between builds. Pin instead; bump this when
# a new YOCTO_RELEASE needs it.
rust_bin_rev="54721fa1c7a4edaba7fb63cdbcaf0c6253c5fa3a"
if [ ! -d "${rust_bin_dir}" ]; then
  git clone https://github.com/rust-embedded/meta-rust-bin.git "${rust_bin_dir}"
fi
if ! git -C "${rust_bin_dir}" cat-file -e "${rust_bin_rev}^{commit}" 2> /dev/null; then
  git -C "${rust_bin_dir}" fetch origin master
fi
git -C "${rust_bin_dir}" reset --hard "${rust_bin_rev}"

# meta-rust-bin's LAYERSERIES_COMPAT can lag upstream; self-heal if needed.
rust_bin_layer_conf="${rust_bin_dir}/conf/layer.conf"
if ! grep -qw "${YOCTO_RELEASE}" "${rust_bin_layer_conf}"; then
  python3 - "${rust_bin_layer_conf}" "${YOCTO_RELEASE}" << 'EOF'
import re
import sys

path, release = sys.argv[1], sys.argv[2]
with open(path) as f:
    content = f.read()
content = re.sub(
    r'(LAYERSERIES_COMPAT_rust-bin-layer\s*=\s*"[^"]*)"',
    f'\\1    {release} \\\n"',
    content,
    count=1,
)
with open(path, "w") as f:
    f.write(content)
EOF
fi

# oe-init-build-env requires allowing unbound variables...:
set +u

cd "${HOME}/yocto"
# shellcheck disable=SC1091
TEMPLATECONF="${templateconf}" source "${HOME}/yocto/sources/poky/oe-init-build-env" build

# run any args given to us (defaults to Dockerfile's CMD)
exec "$@"
