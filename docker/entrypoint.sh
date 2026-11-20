#!/bin/bash -exu

# YOCTO_RELEASE environment variable is expected to be set to i.e. "kirkstone", "dunfell", ...

case "${YOCTO_RELEASE}" in
  dunfell | kirkstone | scarthgap) ;;
  *)
    setup_dir="${HOME}/yocto"
    sdk_dir="${setup_dir}/sources/memfault-linux-sdk"
    bitbake_setup=(
      /opt/bitbake/bin/bitbake-setup
      --setting default top-dir-prefix "$(dirname "${setup_dir}")"
      --setting default top-dir-name .
      --setting default dl-dir "${setup_dir}/build/downloads"
    )
    touch "$(dirname "${setup_dir}")/site.conf"
    if [ -f "${setup_dir}/config/config-upstream.json" ] && [ -f "${setup_dir}/build/init-build-env" ]; then
      "${bitbake_setup[@]}" update --setup-dir "${setup_dir}" --update-bb-conf no
    else
      rm -rf "${setup_dir}/config"
      "${bitbake_setup[@]}" init --non-interactive --no-init-vscode \
        --setup-dir-name "$(basename "${setup_dir}")" \
        -L memfault-linux-sdk "${sdk_dir}" \
        "${sdk_dir}/bitbake-setup/memfault-linux-sdk.conf.json" memfault-example
    fi

    set +u
    cd "${setup_dir}"
    # shellcheck disable=SC1091
    TEMPLATECONF="${setup_dir}/layers/memfault-linux-sdk/meta-memfault-example/conf/templates/default" \
      source "${setup_dir}/layers/openembedded-core/oe-init-build-env" build
    exec "$@"
    ;;
esac

poky_dir="${HOME}/yocto/sources/poky"
if [ ! -d "${poky_dir}" ]; then
  git clone https://git.yoctoproject.org/poky --branch "${YOCTO_RELEASE}" "${poky_dir}"
else
  git -C "${poky_dir}" remote set-url origin https://git.yoctoproject.org/poky
  git -C "${poky_dir}" checkout "${YOCTO_RELEASE}" && git -C "${poky_dir}" pull --ff-only
fi

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
TEMPLATECONF="../memfault-linux-sdk/meta-memfault-example/conf/" source "${HOME}/yocto/sources/poky/oe-init-build-env" build

# run any args given to us (defaults to Dockerfile's CMD)
exec "$@"
