//
// Copyright (c) Memfault, Inc.
// See License.txt for details
extern crate cmake;

use std::env;
use walkdir::WalkDir;

fn main() {
    let mut config = cmake::Config::new("../libmemfaultc");

    // Disable tests when building from Cargo
    config.define("TESTS", "0");

    // Cross-compile C flags set by the CC Crate can conflict with the
    // flags set by Yocto. Tell CC-rs to not set default flags.
    env::set_var("CRATE_CC_NO_DEFAULTS", "true");

    // Cargo-rs would set this to MinSizeRel because this crate is configured
    // with opt-level='z' but we want to stick with the default which among
    // other things keeps asserts enabled.
    config.profile("");

    // Build a list of the library that we want to link into the final binary.
    let mut libs = vec![];

    // We only build coredump parsing on Linux
    if env::var("CARGO_FEATURE_COREDUMP").is_ok() && cfg!(target_os = "linux") {
        config.define("WITH_COREDUMP", "1");
        libs.push("zlib")
    }
    if env::var("CARGO_FEATURE_SYSTEMD").is_ok() {
        // Systemd is not available on macOS. We silently accept the feature so
        // that the rust code can be checked but we don't actually build the C
        // code.
        if cfg!(not(target_os = "macos")) {
            config.define("WITH_SYSTEMD", "1");
            libs.push("libsystemd");
        }
    }
    if env::var("CARGO_FEATURE_SWUPDATE").is_ok() {
        config.define("WITH_SWUPDATE", "1");
        libs.push("libconfig");
    }

    // Linting needs to run `cargo` (and thus execute this file) to verify the
    // project but the C dependencies are not installed on this machine.  This
    // environment variable will stop the script here, before doing any actual build.
    // There is no standard Cargo way to tell if we are being called as part of `cargo lint`.
    // See: https://github.com/rust-lang/cargo/issues/4001
    if env::var("MEMFAULTD_SKIP_CMAKE").is_ok() {
        return;
    }

    // Find required C libraries and tell Cargo how to link them
    let pkg_config = pkg_config::Config::new();
    libs.iter().for_each(|lib| {
        // This line will print the required Cargo config if the library is found.
        let r = pkg_config.probe(lib);
        if let Err(e) = r {
            println!("WARNING - Library {} was not found: {}", lib, e);
        }
    });

    // Build the libmemfaultc library and link tell Cargo to link it in the project
    let dst = config.build();
    println!("cargo:rustc-link-search=native={}/build", dst.display());
    println!("cargo:rustc-link-lib=static=memfaultc");

    // Tell cargo to rebuild the project when any of the C project files changes
    WalkDir::new("../libmemfaultc/src")
        .into_iter()
        .filter_map(Result::ok)
        .filter_map(
            |e| match e.path().extension().and_then(std::ffi::OsStr::to_str) {
                Some("c") => Some(e),
                Some("h") => Some(e),
                // CMake config files
                Some("txt") => Some(e),
                _ => None,
            },
        )
        .for_each(|e| println!("cargo:rerun-if-changed={}", e.path().display()));

    println!("cargo:rerun-if-changed=build.rs");
}
