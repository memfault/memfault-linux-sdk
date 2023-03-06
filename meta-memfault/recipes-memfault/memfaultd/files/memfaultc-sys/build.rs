//
// Copyright (c) Memfault, Inc.
// See License.txt for details
extern crate cmake;

use std::{env, path::Path};
use std::{io::Write, path::PathBuf};
use walkdir::WalkDir;

struct LinkedLibrary {
    name: &'static str,
    target_os: Option<&'static str>,
}

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
    let mut libs = vec![
        LinkedLibrary {
            name: "libsystemd",
            target_os: Some("linux"),
        },
        LinkedLibrary {
            name: "json-c",
            target_os: None,
        },
        LinkedLibrary {
            name: "zlib",
            target_os: None,
        },
    ];

    // Pass version information to memfaultd
    let mut version_file_path = PathBuf::from(&env::var("CARGO_MANIFEST_DIR").unwrap());
    version_file_path.push("../../../../VERSION");
    let version_content =
        std::fs::read_to_string(version_file_path).unwrap_or_else(|_| String::new());

    let version = version_content
        .lines()
        .map(|l| l.split_once(':'))
        .find_map(|split| match split {
            Some(("VERSION", value)) => Some(value.trim().replace(' ', "")),
            _ => None,
        })
        .unwrap_or_else(|| String::from("dev"));
    // Pass the version to C Code
    config.cflag(format!("-DVERSION={}", version));

    // Save build-time information (eg: VERSION) in a `.rs` file which is
    // included at compile time (see memfaultc-sys/src/lib.rs)
    let dst = Path::new(&env::var("OUT_DIR").unwrap()).join("buildinfo.rs");
    let mut buildtime = std::fs::File::create(dst).expect("cannot create buildtime.txt");
    write!(
        buildtime,
        "pub fn memfaultd_sdk_version() -> &'static str {{ \"{}\" }}",
        version
    )
    .unwrap();

    // Activate optional plugins if required
    if env::var("CARGO_FEATURE_COREDUMP").is_ok() {
        config.define("PLUGIN_COREDUMP", "1");
        libs.push(LinkedLibrary {
            name: "uuid",
            target_os: None,
        });
    }
    if env::var("CARGO_FEATURE_COLLECTD").is_ok() {
        config.define("PLUGIN_COLLECTD", "1");
    }
    if env::var("CARGO_FEATURE_REBOOT").is_ok() {
        config.define("PLUGIN_REBOOT", "1");
        libs.push(LinkedLibrary {
            name: "libubootenv",
            target_os: Some("linux"),
        });
    }
    if env::var("CARGO_FEATURE_SWUPDATE").is_ok() {
        config.define("PLUGIN_SWUPDATE", "1");
        libs.push(LinkedLibrary {
            name: "libconfig",
            target_os: None,
        });
    }
    if env::var("CARGO_FEATURE_LOGGING").is_ok() {
        config.define("PLUGIN_LOGGING", "1");
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
    libs.iter()
        .filter(|lib| {
            // Filter out libraries that have a target_os specified different than current OS
            if let Some(target_os) = lib.target_os {
                if std::env::consts::OS != target_os {
                    return false;
                }
            }
            true
        })
        .for_each(|lib| {
            // This line will print the required Cargo config if the library is found.
            let r = pkg_config.probe(lib.name);
            if let Err(e) = r {
                println!("WARNING - Library {} was not found: {}", lib.name, e);
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
}
