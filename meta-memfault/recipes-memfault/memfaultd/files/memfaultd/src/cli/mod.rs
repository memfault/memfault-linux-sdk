//
// Copyright (c) Memfault, Inc.
// See License.txt for details
mod cargs;
mod memfault_core_handler;
mod memfaultctl;
mod memfaultd;

pub fn main() {
    stderrlog::new()
        .module(module_path!())
        .module("memfaultd")
        .verbosity(2)
        .init()
        .unwrap();

    let args = cargs::CArgs::new(std::env::args());

    let exit_code = match args.name() {
        "memfault-core-handler" => memfault_core_handler::main(args),
        "memfaultctl" => memfaultctl::main(),
        "memfaultd" => memfaultd::main(args),
        _ => {
            eprintln!(
                "Unknown command: {}. Should be memfaultd or memfaultctl.",
                args.name()
            );
            1
        }
    };

    std::process::exit(exit_code);
}
