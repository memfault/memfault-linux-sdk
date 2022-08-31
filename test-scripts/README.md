# `test-scripts`

This directory contains E2E tests that exercise the Memfault Linux SDK and may
expect the Memfault platform services to be available and a matching project to
be configured. To see how the environment is built, read [`qemu.py`](./qemu.py).

To run a specific test:

```console
# pytest ~/yocto/sources/memfault-linux-sdk/test-scripts/01_qemu_boot_test.py
```

To run all tests:

```console
# pytest ~/yocto/sources/memfault-linux-sdk/test-scripts
```

> NOTE: some of these tests access the Memfault API and require a project with
> certain configurations (documented in each test). Tests that access the API
> expect the following environment variables to be set:
> `MEMFAULT_E2E_API_BASE_URL` -- the base URL of the Memfault API, i.e.
> https://app.memfault.com > `MEMFAULT_E2E_ORGANIZATION_SLUG` -- the
> organization slug `MEMFAULT_E2E_PROJECT_SLUG` -- the slug of the test project
> `MEMFAULT_E2E_USER_EMAIL` -- the test user account's email address
> `MEMFAULT_E2E_USER_PASSWORD` -- the test user account's password
