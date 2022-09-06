#!/usr/bin/env bash

# Wrapper script for running flake8 when passing multiple file paths as
# positional args.
#
# flake8 will ignore the exclude list when file paths (not directories!) are
# passed. Instead, pass the file paths as '--filename' pattern argument. This
# will still honor the exclude list in .flake8.
#
# This script is used as a workaround for running flake8 in pre-commit, which
# calls the linting tools with multiple file paths.

set -euo pipefail

# convert script args into a bash array
declare -a ARGS=( "$@" )

# join the filenames with ',./', which is the format the '--filename' arg uses.
#
# note: it's required to prepend './' to the filenames, otherwise the glob
# matching flake8 (as of 4.0.1) uses is BROKEN:
#
# https://github.com/PyCQA/flake8/issues/298
function join_by {
  local separator="$1"
  shift
  local first="$1"
  shift
  printf "%s" "$first" "${@/#/$separator}"
}
FILENAME=./$(join_by ",./" "${ARGS[@]}")
flake8 --config ./.flake8 --filename "$FILENAME"
