//
// Copyright (c) Memfault, Inc.
// See License.txt for details
use cc::Build;
use std::env;
use walkdir::WalkDir;

static LIBMEMFAULTC: &str = "libmemfaultc";

fn main() {
    // Cross-compile C flags set by the CC Crate can conflict with the
    // flags set by Yocto. Tell CC-rs to not set default flags.
    env::set_var("CRATE_CC_NO_DEFAULTS", "true");

    let mut cc = Build::new();
    cc.flag("-fPIC");
    cc.include(format!("{}/include", LIBMEMFAULTC));

    cc.file(format!("{}/src/crash.c", LIBMEMFAULTC));

    // Build a list of the library that we want to link into the final binary.
    let mut libs = vec![];

    if env::var("CARGO_FEATURE_SYSTEMD").is_ok() {
        // Systemd is not available on macOS. We silently accept the feature so
        // that the rust code can be checked but we don't actually build the C
        // code.
        if cfg!(not(target_os = "macos")) {
            cc.file(format!("{}/src/systemd.c", LIBMEMFAULTC));
            libs.push("libsystemd");
        }
    }

    if env::var("CARGO_FEATURE_SWUPDATE").is_ok() {
        cc.file(format!("{}/src/swupdate.c", LIBMEMFAULTC));
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
        match pkg_config.probe(lib) {
            Ok(lib) => {
                cc.includes(lib.include_paths);
            }
            Err(e) => println!("WARNING - Library {} was not found: {}", lib, e),
        }
    });

    // Build the libmemfaultc library and link tell Cargo to link it in the project
    cc.compile("memfaultc");
    println!("cargo:rustc-link-lib=static=memfaultc");

    // Tell cargo to rebuild the project when any of the C project files changes
    let root_src_dir = format!("{}/src", LIBMEMFAULTC);
    WalkDir::new(root_src_dir)
        .into_iter()
        .filter_map(Result::ok)
        .filter_map(
            |e| match e.path().extension().and_then(std::ffi::OsStr::to_str) {
                Some("c") => Some(e),
                Some("h") => Some(e),
                _ => None,
            },
        )
        .for_each(|e| println!("cargo:rerun-if-changed={}", e.path().display()));

    println!("cargo:rerun-if-changed=build.rs");
}
