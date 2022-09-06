#!/bin/bash -e

TEST_BUILD_DIR=/tmp/build-test

while getopts "c" options; do
    case "${options}" in
    c)
        echo "Cleaning build directory... ${TEST_BUILD_DIR}"
        rm -rf ${TEST_BUILD_DIR}
        ;;
    *) exit 1;;
    esac
done

cmake -B ${TEST_BUILD_DIR} /home/build/yocto/sources/memfault-linux-sdk/meta-memfault/recipes-memfault/memfaultd/files/memfaultd
cd ${TEST_BUILD_DIR}
make && make test ARGS=--output-on-failure
