# Memfault Linux SDK Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to
[Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [1.0.0] - 2022-09-28

### Added

- This release is the first one including support for collecting and uploading
  user-land coredumps to the Memfault platform. The coredump plugin is enabled
  by default. Alongside this SDK release, an accompanying [Memfault
  CLI][docs-cli] version 0.11.0 aids in uploading symbol files to Memfault from
  Yocto builds to facilitate making use of the new functionality. Uploading
  symbols is a necessary step in order to use Memfault for coredumps. [Read more
  about coredump support in the Memfault Linux SDK][docs-coredumps].

[docs-coredumps]: https://mflt.io/linux-coredumps
[docs-cli]: https://mflt.io/memfault-cli

### Changed

- Breaking changes in the format of `/etc/memfaultd.conf` (see [the updated
  reference][docs-reference-memfaultd-conf]):
  - The `collectd` top-level key was merged into the `collectd_plugin` top-level
    key. The fields previously in `collectd` that have been moved to
    `collectd_plugin` are:
    - `interval_seconds`
    - `non_memfaultd_chain`
    - `write_http_buffer_size_kib`
  - The `collectd_plugin.output_file` key has been replaced by two new keys:
    - `collectd_plugin.header_include_output_file`: the value of which should be
      included as the first statement in your `/etc/collectd.conf` file, and
    - `collectd_plugin.footer_include_output_file`: to be included as the last
      statement of your `/etc/collectd.conf` file.

[docs-reference-memfaultd-conf]:
  https://docs.memfault.com/docs/linux/reference-memfaultd-configuration/

### Fixed

- A misconfiguration bug whereby setting `collectd.interval_seconds` (now
  `collectd_plugin.interval_seconds`, see the "Changed" section of this release)
  would have no effect if our include file was at the bottom of
  `/etc/collectd.conf`. It happened due to the fact that collectd `Interval`
  statements are evaluated as they appear in source code (see [the author's
  statement][collectd-interval-eval]), only affecting the plugin statements that
  come after it.

[collectd-interval-eval]:
  https://github.com/collectd/collectd/issues/2444#issuecomment-331804766

### Known Issues

Temporarily, our backend processing pipeline is unable to process coredumps that
link to shared objects in a specific style. This affects, in particular,
coredumps coming from devices on the Dunfell release of Yocto.

A backend fix has already been identified and should be released in the next few
business days. Once released, any previously collected coredumps that are
affected will be reprocessed server-side to address this issue. This will
**not** require any action from your team.

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
[1.0.0]:
  https://github.com/memfault/memfault-linux-sdk/releases/tag/1.0.0-kirkstone
