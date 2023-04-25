//
// Copyright (c) Memfault, Inc.
// See License.txt for details
use super::cargs;

#[cfg(all(target_os = "linux", feature = "coredump"))]
pub fn main(args: cargs::CArgs) -> i32 {
    unsafe { memfaultc_sys::memfault_core_handler_main(args.argc(), args.argv()) }
}

#[cfg(not(all(target_os = "linux", feature = "coredump")))]
pub fn main(_args: cargs::CArgs) -> i32 {
    eprintln!("memfault-core-handler is not supported in this build");
    1
}
