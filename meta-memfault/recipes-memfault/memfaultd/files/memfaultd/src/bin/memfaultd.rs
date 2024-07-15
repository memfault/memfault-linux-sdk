//
// Copyright (c) Memfault, Inc.
// See License.txt for details
use memfaultd::cli;

/// memfaultd is an alias to the main function in cli.rs
///
/// In the target machine, memfaultd is the only binary that remains. memfaultctl and
/// memfault-core-handler are symlinked by build scripts. In the case of Yocto, this is
/// meta-memfault/recipes-memfault/memfaultd/memfaultd.bb
///
/// The binary setup in this crate could opt for a single bin/memfaultd.rs entrypoint, however
/// setting up three different binaries makes development easier because it mimics what build
/// systems do in target devices.
///
/// Alternative solutions are possible, for example adding a post-build script that does the
/// symlinking manually in the build directory.
fn main() {
    cli::main()
}
