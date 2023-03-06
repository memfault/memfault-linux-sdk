//
// Copyright (c) Memfault, Inc.
// See License.txt for details
use libc::c_char;
use libc::c_int;
use std::ffi::CString;
use std::path::Path;

pub struct CArgs {
    argv: Vec<CString>,
    argv_ptr: Vec<*const libc::c_char>,
}

impl CArgs {
    pub fn new(args: impl IntoIterator<Item = String>) -> Self {
        let argv: Vec<_> = args
            .into_iter()
            .map(|arg| CString::new(arg.as_str()).unwrap())
            .collect();
        let argv_ptr: Vec<_> = argv
            .iter()
            .map(|arg| arg.as_ptr())
            .chain(std::iter::once(std::ptr::null()))
            .collect();
        Self { argv, argv_ptr }
    }

    /// Returns the number of arguments, ie the C language's `argc`.
    pub fn argc(&self) -> c_int {
        self.argv.len() as c_int
    }

    /// Returns the C language's `argv` (`*const *const c_char`).
    pub fn argv(&self) -> *const *const c_char {
        self.argv_ptr.as_ptr()
    }

    /// Returns the name of the command invoked by the user - removing any path information.
    pub fn name(&self) -> &str {
        Path::new(self.argv[0].to_str().unwrap())
            .file_name()
            .unwrap()
            .to_str()
            .unwrap()
    }
}
