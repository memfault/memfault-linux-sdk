#!/bin/sh -e

# NB: YOCTO_RELEASE is searched and replaced by linux_sdk_release.py during archival.
# Treat is as generated code, or update linux_sdk_release.py accordingly.
YOCTO_RELEASE="dunfell"

command=""

MEMFAULT_YOCTO_BUILD_MOUNT_PREFIX=${MEMFAULT_YOCTO_BUILD_MOUNT_PREFIX:-"/tmp/yocto-build-"}
buildmount="--mount type=volume,source=yocto-build-${YOCTO_RELEASE},target=/home/build/yocto/build"

while getopts "bv:c:e:r:tv:" options; do
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
    r)
      YOCTO_RELEASE="${OPTARG}"
      buildmount="--mount type=volume,source=yocto-build-${YOCTO_RELEASE},target=/home/build/yocto/build"
      ;;
    t)
      # Use a bind mount at ${MEMFAULT_YOCTO_BUILD_MOUNT_PREFIX}${YOCTO_RELEASE} for the build artifacts
      # for easy inspection of output. Example usage:
      #
      # $ MEMFAULT_YOCTO_BUILD_MOUNT_PREFIX=${HOME}/yocto ./run.sh -bt
      #
      # The build artifacts will be placed at ${HOME}/yocto-${YOCTO_RELEASE}`
      mkdir -p "${MEMFAULT_YOCTO_BUILD_MOUNT_PREFIX}${YOCTO_RELEASE}"
      buildmount="--mount type=bind,source=${MEMFAULT_YOCTO_BUILD_MOUNT_PREFIX}${YOCTO_RELEASE},target=/home/build/yocto/build"
      ;;
    *) exit 1 ;;
  esac
done

metamount="--mount type=bind,source=${PWD}/..,target=/home/build/yocto/sources/memfault-linux-sdk"
sourcesmount="--mount type=volume,source=yocto-sources-${YOCTO_RELEASE},target=/home/build/yocto/sources"

if [ -n "$MEMFAULT_CLI_DIST_PATH" ]; then
  memfaultclimount="--mount type=bind,source=$(readlink -f "$MEMFAULT_CLI_DIST_PATH"),target=/home/build/memfault-cli-dist"
else
  memfaultclimount=""
fi

# vars are overridden from the local environment, falling back to env.list
env_vars="
--env MACHINE
--env MEMFAULT_BASE_URL
--env MEMFAULT_PROJECT_KEY
--env MEMFAULT_DEVICE_ID
--env MEMFAULT_SOFTWARE_VERSION
--env MEMFAULT_HARDWARE_VERSION
--env MEMFAULT_SOFTWARE_TYPE
--env-file env.list
"

# vars for E2E test scripts
e2e_test_env_vars="
--env MEMFAULT_E2E_API_BASE_URL
--env MEMFAULT_E2E_ORGANIZATION_SLUG
--env MEMFAULT_E2E_PROJECT_SLUG
--env MEMFAULT_E2E_ORG_TOKEN
--env MEMFAULT_E2E_TIMEOUT_SECONDS
--env-file env-test-scripts.list
"

# shellcheck disable=SC2086
docker run \
  --interactive --rm --tty \
  --network="host" \
  --name memfault-linux-qemu \
  ${buildmount} \
  ${sourcesmount} \
  ${memfaultclimount} \
  ${metamount} \
  ${env_vars} \
  ${e2e_test_env_vars} \
  --env YOCTO_RELEASE="${YOCTO_RELEASE}" \
  ${entrypoint} \
  yocto \
  ${command}
