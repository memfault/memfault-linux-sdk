//
// Copyright (c) Memfault, Inc.
// See License.txt for details
#[allow(unused_imports)]
use memfaultc_sys::{memfault_core_handler_main, memfaultctl_main, memfaultd_main};

mod cargs;

pub fn main() {
    let args = cargs::CArgs::new(std::env::args());

    stderrlog::new()
        .module(module_path!())
        .module("memfaultd")
        .verbosity(2)
        .init()
        .unwrap();

    let exit_val = match args.name() {
        "memfaultctl" => unsafe { memfaultctl_main(args.argc(), args.argv()) },
        "memfault-core-handler" => memfault_core_handler(args),
        "memfaultd" => unsafe { memfaultd_main(args.argc(), args.argv()) },
        _ => {
            eprintln!(
                "Unknown command: {}. Should be memfaultd or memfaultctl.",
                args.name()
            );
            1
        }
    };
    std::process::exit(exit_val);
}

#[cfg(all(target_os = "linux", feature = "coredump"))]
fn memfault_core_handler(args: cargs::CArgs) -> i32 {
    unsafe { memfault_core_handler_main(args.argc(), args.argv()) }
}

#[cfg(not(all(target_os = "linux", feature = "coredump")))]
fn memfault_core_handler(_args: cargs::CArgs) -> i32 {
    eprintln!("memfault-core-handler is not supported in this build");
    1
}
