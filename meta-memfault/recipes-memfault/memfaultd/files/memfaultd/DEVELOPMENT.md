# Development

## Using CLion to work on memfaultd

- Install the library dependencies on your system / environment, i.e.
  `apt install libjson-c-dev libcurl4-openssl-dev libsystemd-dev libubootenv-dev`.
- Add `-DENABLE_PLUGINS=1 -DPLUGIN_REBOOT=1` (and any other plugins you want to
  compile in) to the CMake arguments in Clion's Settings.
- If you are using a conda env, add
  `-DPKG_CONFIG_EXECUTABLE=<path/to/pkg-config>` to the CMake arguments, to make
  sure the correct `pkg-config` binary is used.
- Find meta-memfault/recipes-memfault/memfaultd/files/memfaultd/CMakeLists.txt
  in the Project.
- Right click it and select "Load Cmake Project".
- `memfaultd` and various `test_...` targets are now available to build, run and
  debug from CLion!

## CppUTest unit tests

A helper script called `/test.sh` is part of the Docker image that runs
`memfaultd`'s CppUTest unit tests.

From within the container, run:

```console
/test.sh
```

Or from the host:

```console
./run.sh -b -e /test.sh
```
