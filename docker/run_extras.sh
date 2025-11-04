#!/bin/sh -e

wefaultd_path=${PWD}/../../../wefault/wefaultd
metawefault_path=${PWD}/../../../wefault/meta-wefault

if [ -f "${wefaultd_path}" ]; then
  wefaultdmount="--mount type=bind,source=${wefaultd_path},target=/home/build/yocto/sources/wefaultd"
fi

if [ -f "${metawefault_path}" ]; then
  metawefaultmount="--mount type=bind,source=${metawefault_path},target=/home/build/yocto/sources/meta-wefault"
fi

export extramounts="${wefaultdmount} ${metawefaultmount}"
