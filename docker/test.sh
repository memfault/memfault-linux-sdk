#!/usr/bin/env bash

set -euo pipefail

TEST_BUILD_DIR=/tmp/build-test

function do_clean() {
  for arch_and_cflag in "i386 -m32" "x86_64 -m64"; do
    # shellcheck disable=SC2086
    set -- $arch_and_cflag
    echo "Cleaning build directory... ${TEST_BUILD_DIR}-$1"
    rm -rf "${TEST_BUILD_DIR}-$1"
  done
}

while getopts "c" options; do
  case "${options}" in
  c)
    do_clean
    ;;
  *) exit 1 ;;
  esac
done

for arch_and_cflag in "i386 -m32" "x86_64 -m64"; do
  # shellcheck disable=SC2086
  set -- $arch_and_cflag
  cmake \
    "-DCMAKE_CXX_FLAGS=$2" \
    "-DCMAKE_C_FLAGS=$2" \
    -B "${TEST_BUILD_DIR}-$1" \
    /home/build/yocto/sources/memfault-linux-sdk/meta-memfault/recipes-memfault/memfaultd/files/libmemfaultc
  cd "${TEST_BUILD_DIR}-$1"
  make --trace "-j$(nproc)"
  make test ARGS=--output-on-failure
done
