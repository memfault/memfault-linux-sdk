# Development

`memfaultd` build is controlled by Cargo. The `Cargo.toml` and `build.rs`
control the rust build process and compile the few C files.

## Building outside Yocto

### Dependencies

#### Debian/Ubuntu

```sh
apt install libsystemd-dev libconfig-dev
```

#### macOS

```sh
brew install pkg-config libconfig
```

(note: `libsystemd` is not available on macOS and the build system will not try
to link it)

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
cargo test
```

### Updating snapshots

Install `insta` if necessary, and run the command:

```bash
cargo install cargo-insta
cargo insta review
```

## IDE integration

### Using VSCode to work on memfaultd

VSCode rust plugin will not find the `Cargo.toml` file unless you open the
`meta-memfault/recipes-memfault/memfaultd/files/memfaultd/` directly.
