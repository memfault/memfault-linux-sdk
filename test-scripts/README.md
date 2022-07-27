# `test-scripts`

This directory contains E2E tests that exercise the Memfault Linux SDK and may expect the Memfault
platform services to be available and a matching project to be configured. To see how the
environment is built, read [`qemu.py`](./qemu.py).

To run a specific test:

```console
# cd ~/yocto/sources/memfault-linux-sdk/test-scripts && ./<filename>
```

To run all tests:

```console
# pytest-3 ~/yocto/sources/memfault-linux-sdk/test-scripts
```
