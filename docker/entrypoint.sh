#!/bin/bash -ex

poky_dir="${HOME}/yocto/sources/poky"
if [ ! -d "${poky_dir}" ]; then
    git clone https://git.yoctoproject.org/git/poky --branch kirkstone "${poky_dir}"
else
    git -C "${poky_dir}" checkout kirkstone && git -C "${poky_dir}" pull --ff-only
fi

openembedded_dir="${HOME}/yocto/sources/meta-openembedded"
if [ ! -d "${openembedded_dir}" ]; then
    git clone https://github.com/openembedded/meta-openembedded.git --branch kirkstone "${openembedded_dir}"
else
    git -C "${openembedded_dir}" checkout kirkstone && git -C "${openembedded_dir}" pull --ff-only
fi

swupdate_dir="${HOME}/yocto/sources/meta-swupdate"
if [ ! -d "${swupdate_dir}" ]; then
    git clone https://github.com/sbabic/meta-swupdate.git --branch kirkstone "${swupdate_dir}"
else
    git -C "${swupdate_dir}" checkout kirkstone && git -C "${swupdate_dir}" pull --ff-only
fi

cd "${HOME}/yocto"
TEMPLATECONF=../memfault-linux-sdk/meta-memfault-example/conf/ source "${HOME}/yocto/sources/poky/oe-init-build-env" build

# run any args given to us (defaults to Dockerfile's CMD)
exec "$@"
