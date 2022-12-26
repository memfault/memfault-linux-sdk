#!/usr/bin/env bash

set -euo pipefail

if [ "$(uname -s)" == "Darwin" ]; then
  echo "Not supposed to run this on macOS"
  exit 125
fi

echo "Installing cpputest dependencies"
dpkg --add-architecture i386
apt-get update

# Work-around for conflicting binary (32 vs 64 bit) -- we don't use/need it
rm -rf /usr/bin/curl-config

apt-get install -y \
    gcc-multilib g++-multilib \
    libjson-c-dev:i386 \
    libsystemd-dev:i386 \
    uuid-dev:i386 \
    libcurl4-openssl-dev:i386 \
    zlib1g-dev:i386

# Work-around for libubootenv-dev:i386 package not existing -- we only need the header to compiler the unit tests:
cp /usr/include/x86_64-linux-gnu/libuboot.h /usr/include/i386-linux-gnu/libuboot.h


# Install a more up to date version of cpputest (debian buster installs 3.8-7)
CPPUTEST_URL=https://github.com/cpputest/cpputest/releases/download/v4.0/cpputest-4.0.tar.gz
CPPUTEST_SHA256SUM=21c692105db15299b5529af81a11a7ad80397f92c122bd7bf1e4a4b0e85654f7

# Download and sha check
BUILDDIR=/tmp/cpputest
mkdir -p ${BUILDDIR}
cd ${BUILDDIR}
wget ${CPPUTEST_URL} -O cpputest.tar.gz
shasum --algorithm 256 --check <(echo "${CPPUTEST_SHA256SUM}  cpputest.tar.gz")

# Unpack, build, install
tar zvxf cpputest.tar.gz
rm cpputest.tar.gz
cd cpputest*

for arch_and_cflag in "i386 -m32" "x86_64 -m64"
do
    # shellcheck disable=SC2086
    set -- $arch_and_cflag

    # set specific directories for the lib and include install. the cpputest
    # MakefileWorker.mk w/ our mods makes certain assumptions and it's simpler to
    # just adhere to them here
    ./configure "--libdir=/usr/lib/$1-linux-gnu" --includedir=/usr/include CFLAGS=$2 CXXFLAGS=$2 LDFLAGS=$2
    make -j8
    sudo make install

    # Check library is installed
    if ! (find "/usr/lib/$1-linux-gnu" -name libCppUTest.a | grep -q libCppUTest.a); then
      echo "Can't find libCppUTest.a for in /usr/lib/$1-linux-gnu!"
      exit 1
    fi

    make clean
done

# Clean up installation files
cd ~
rm -rf ${BUILDDIR}
