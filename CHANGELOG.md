# Memfault Linux SDK Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to
[Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [0.3.1] - 2022-09-05

### Added

- Support for Yocto version 3.1 (code name "Dunfell"). See the
  [`dunfell` branch](https://github.com/memfault/memfault-linux-sdk/tree/dunfell)
  of the repository.

### Changed

- The SDK repository no longer has a `main` branch. The variant of the SDK that
  supports Yocto 4.0 ("Kirkstone") can be found on the
  [branch named `kirkstone`](https://github.com/memfault/memfault-linux-sdk/tree/kirkstone).
  Likewise, the variant of the SDK that supports Yocto 3.1 ("Dunfell) can be
  found on
  [the branch called `dunfell`](https://github.com/memfault/memfault-linux-sdk/tree/dunfell).

## [0.3.0] - 2022-08-31

### Added

- Initial support for collecting metrics using [collectd]. Check out the
  [docs on Metrics for Linux](https://mflt.io/linux-metrics) for more
  information.

[collectd]: https://collectd.org/

## [0.2.0] - 2022-08-10

This is our first public release. Head over to [our Linux
documentation][docs-linux] for an introduction to the Memfault Linux SDK.

[docs-linux]: https://docs.memfault.com/docs/linux/introduction

### Added

- [memfaultd] Now implements exponential back-off for uploads. Requests
  originating from this exponential back-off system do not interfere with the
  regular upload interval.
- [memfaultd] Sets persisted flag to disable data collection and returns
  immediately: `memfaultd --disable-data-collection`.
- [memfaultd] The `builtin.json` configuration file now features a link to
  documentation for reference.
- Improved the top-level `README.md` with a feature and architecture overview.

### Fixed

- [memfaultd] The `--enable-data-collection` flag was not working reliably.
- [memfaultd] A parsing bug going through the output of `memfault-device-info`.

### Known Issues

During start-up of the `memfaultd` service, you may see a log line in the output
of `journalctl --unit memfaultd`:

```
memfaultd.service: Can't open PID file /run/memfaultd.pid (yet?) after start: Operation not permitted
```

This file is only used by `systemd` during service shut-down and its absence
during start-up does not affect the functioning of the daemon. A fix is planned
for a future release. See [this report on the Ubuntu `nginx`
package][nginx-pid-report] for a discussion on the topic.

[nginx-pid-report]: https://bugs.launchpad.net/ubuntu/+source/nginx/+bug/1581864

## [0.1.0] - 2022-07-27

### Added

- [memfaultd] Support reporting reboot reasons.
- [memfaultd] Support OTA updates via SWUpdate.
- A memfaultd layer for Yocto (meta-memfault).
- An example Yocto image using memfaultd and the features above
  (meta-memfault-example).

[0.1.0]: https://github.com/memfault/memfault-linux-sdk/releases/tag/0.1.0
[0.2.0]: https://github.com/memfault/memfault-linux-sdk/releases/tag/0.2.0
[0.3.0]: https://github.com/memfault/memfault-linux-sdk/releases/tag/0.3.0
[0.3.1]:
  https://github.com/memfault/memfault-linux-sdk/releases/tag/0.3.1-kirkstone
