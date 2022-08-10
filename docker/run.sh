#!/bin/sh -e

command=""
while getopts "bv:c:e:" options; do
    case "${options}" in
    b)
        docker build --tag yocto .
        ;;
    c)
        command="${OPTARG}"
        ;;
    e)
        entrypoint="--entrypoint ${OPTARG}"
        ;;
    *) exit 1;;
    esac
done

metamount="--mount type=bind,source=${PWD}/..,target=/home/build/yocto/sources/memfault-linux-sdk"

buildmount="--mount type=volume,source=yocto-build,target=/home/build/yocto/build"
sourcesmount="--mount type=volume,source=yocto-sources,target=/home/build/yocto/sources"

# vars are overridden from the local environment, falling back to env.list
env_vars="--env MEMFAULT_BASE_URL --env MEMFAULT_PROJECT_KEY --env MEMFAULT_DEVICE_ID --env MEMFAULT_SOFTWARE_VERSION --env MEMFAULT_HARDWARE_VERSION --env MEMFAULT_SOFTWARE_TYPE --env-file env.list"

docker run \
    --interactive --rm --tty \
    --network="host" \
    --name memfault-linux-qemu \
    ${buildmount} \
    ${sourcesmount} \
    ${metamount} \
    ${env_vars} \
    ${entrypoint} \
    yocto \
    ${command}
