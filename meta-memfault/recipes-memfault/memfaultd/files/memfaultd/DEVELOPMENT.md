# Development

`memfaultd` build is controlled by Cargo. The `Cargo.toml` and `build.rs`
control the rust build process and will call `cmake`/`configure`/`make` to build
the C libraries during the build process.

## Building outside Yocto

### Installing dependencies

#### On Debian

```sh
apt install \
  cpputest \
  libjson-c-dev \
  uuid-dev \
  libsystemd-dev \
  libubootenv-dev \
  libconfig-dev
```

#### On macOS

```sh
brew install cmake cpputest libconfig util-linux json-c
```

### Building

```sh
cargo build
```

## Building with Yocto

Use the `docker/run.sh` script to run a docker container with all the required
dependencies. Use the alias `b` to build the image.

## Running tests

### Unit tests

Do this after running a build, inside the (cmake) build directory:

```sh
mkdir build
cd build
cmake ..
make
make test
```

### Integration tests (inside docker)

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

## IDE integration

### Using CLion to work on memfaultd

- Add `-DPLUGIN_REBOOT=1` (and any other plugins you want to compile in) to the
  CMake arguments in Clion's Settings.
- If you are using a conda env, add
  `-DPKG_CONFIG_EXECUTABLE=<path/to/pkg-config>` to the CMake arguments, to make
  sure the correct `pkg-config` binary is used.
- Find meta-memfault/recipes-memfault/memfaultd/files/memfaultd/CMakeLists.txt
  in the Project.
- Right click it and select "Load Cmake Project".
- `memfaultd` and various `test_...` targets are now available to build, run and
  debug from CLion!

### Using VSCode to work on memfaultd

VSCode rust plugin will not find the `Cargo.toml` file unless you open the
`meta-memfault/recipes-memfault/memfaultd/files/memfaultd/` directly.
