//
// Copyright (c) Memfault, Inc.
// See License.txt for details
use super::cargs;
use memfaultc_sys::memfaultd_main;

pub fn main(args: cargs::CArgs) -> i32 {
    unsafe { memfaultd_main(args.argc(), args.argv()) }
}
