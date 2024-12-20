# Simple Executable for stack unwinding tests

This is a simple executable that can be used to test stack unwinding. The main
goal is to have a simple crashing c program that has multiple stack frames, and
is easily reproducible. Note that this is expected to be built on an x86_64
Linux machine. If it is built on a different architecture, the stack unwinding
may not work as expected for the tests

## Build Instructions

To build the executable, run the following command:

```bash
gcc -s -fasynchronous-unwind-tables -xc simple_exe.c -o simple_exe.elf
```

The included binary was created for and on an x86_64 Linux machine.

## Creating a Coredump

To create the included coredump set your core pattern with the following
command:

```bash
echo "/tmp/core.%e.%p" | sudo tee /proc/sys/kernel/core_pattern
```

Then run the executable, and the coredump will be present in `/tmp`.

## Getting `libc` `.eh_frame` data

The test for stack unwinding requires the `.eh_frame` data for the `libc` that
was used to build the executable. Rather than include all of libc which could be
quite large you can strip everything out with the following command:

```bash
objcopy --only-section=.eh_frame --only-section=.eh_frame_hdr /usr/lib/x86_64-linux-gnu/libc.so.6 libc_ehframe.elf
```
