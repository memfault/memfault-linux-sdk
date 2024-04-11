# Memfault Linux SDK Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to
[Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [1.11.0] - 2024-04-10

This release builds upon the session-based metric reporting first shipped in
[1.10.0] as well as adds an exciting new feature: the capture of system logs in
coredumps. This means in addition to everything the Memfault Linux SDK currently
captures in a coredump when a process on your device crashes, a configurable
number of system log lines will also be captured in the coredump and visible on
the Memfault web app! Additionally, we've added a new type of metric report in
addition to session reports and the periodic heartbeat - a daily heartbeat
report. This is intended to allow devices that are not able to report a full set
of periodic heartbeats to Memfault to provide a once-a-day report that
aggregates metrics over a 24 hour window.

### Added

- Log storage persistence configuration - via the new `logs.storage` config
  field users can select whether logs processed by `memfautld` should be written
  to disk or not. If they are not written, creating metrics based on log
  patterns and including logs in coredumps will still function but log files
  will not be uploaded to Memfault. The intention behind this option is to limit
  the number of unnecessary disk writes for systems that are not sending full
  log files to Memfault.
- Daily heartbeats - a once-a-day metric report that aggregates 24 hours of
  metrics captured from your device. It can be enabled with the new
  `metrics.enable_daily_heartbeat` configuration.
- The `memfaultctl start-session/end-session` command has been updated to
  optionally accept Gauge metric readings that will be added to the resulting
  metric report. Readings are accepted in the format `<metric key>=<number>`,
  similar to the `memfaultctl write-attributes` command. Example:
  `memfaultctl end-session camera_recording recording_failed=0`.
- Logs from around the time of the crash are now recorded in coredumps and
  displayed in the Memfault web app. This can be configured via
  `coredumps.log_lines`, which has a default of 100 (meaning the 100 most recent
  log lines will be recorded in a coredump)
- `memfaultd` will now dump all ongoing sessions when it shuts down.
- Adds an internal circlular queue implementation (used by logs in coredumps
  feature).

### Changed

- [Memfault Core Metrics](https://docs.memfault.com/docs/platform/memfault-core-metrics/)
  are always captured in a session metric report, regardless of the
  `captured_metrics` configuration for that session type.
- The internal names of the `MetricReading` variants have been updated make it
  more clear how each type is aggregated.
- Some internal refactoring to centralize the constants for the keys of Memfault
  Core Metrics.
- Session names that conflict with the reserved names of "heartbeat" and
  "daily-heartbeat" are now rejected.
- Starting a session of a type that is already in progress is now a no-op.
  Previously, this reset the metric report for that session type as if the prior
  one had never been started.

### Fixed

- Updated the path in `memfault-core-handler` for `/proc/<pid>/maps` to be an
  absolute path instead of a relative path to keep behavior consistent
  regardless of where the `memfault-core-handler` binary is executed from.
- Some flaky `memfaultd` tests have been updated to consistently pass.
- Battery readings with an SOC % less than 0 or greater than 100 are now
  rejected.
- Empty log files are no longer written to disk nor are they subsequently
  uploaded to Memfault.
- A bug that caused some coredumps from 32-bit machines to be captured
  incorrectly while in thread capture mode.

### Security

- Bump version for external dependency `reqwest` to 0.11.26 to address a
  vulnerability in earlier versions.

## [1.10.0] - 2024-02-15

This release introduces support for session-based metric reporting in the
Memfault Linux SDK! While full support in the Memfault web application is coming
soon, starting with this release users can begin experimenting with how to best
make use of sessions for their devices. Sessions allow for the capture of
metrics aggregated over dynamic spans of time in addition to the periodic
heartbeat. If you are interested in trying out this feature while it is brewing,
please contact us for more details!

### Added

- Support for the `sessions` `memfaultd.conf` field. This configuration allows
  users to define sessions and specify which metrics should be captured for each
  type of session.
- New `memfaultctl` commands `start-session` and `end-session` to start and end
  session metric reports respectively. When `end-session` is called for an
  ongoing session, a MAR file with the aggregated metrics for that session is
  dumped to disk and will be uploaded to Memfault at the next upload interval.
- The `"linux-metric-report"` (formerly `linux-heartbeat`) MAR type now contains
  a `"report_type"` field indicating what type of metric report it is. Currently
  the two types produced by `memfaultd` are `heartbeat` and `session` (with the
  latter including a field for the name of the session)

### Changed

- The MAR type `"linux-heartbeat"` has been renamed `"linux-metric-report"`.
- `memfaultd` can now capture multiple metric reports at once. This enables the
  capture of a session-based metric report without interrupting the capture of
  the periodic heartbeat report.
- Updated the whitespace formatting in the default `memfaultd.service` file for
  more broad drop-in compatibility across `systemd` versions.
- Reboot reasons are now uploaded to the Memfault backend regardless of the
  fleet sampling resolution for the device.
- The preferred version for dependencies of the `memfaultd` bitbake recipe is
  now set with the `?=` operator to avoid conflicts with other recipes in users'
  Yocto build that also use these dependencies.
- `libmemfaultc` is now compiled with the -fPIC flag
- The error message that is printed when `memfaultctl` can't find a PID file for
  `memfaultd` has been modified to make the source of the error more clear.
- The log level for the log output when the last reboot reason file cannot be
  found has been downgraded to `debug!`.

### Fixed

- A bug in which MAR files generated from a pre-1.9.0 version of `memfaultd`
  could not be parsed and uploaded by versions 1.9.0 and 1.9.1.

## [1.9.1] - 2024-1-5

This is a small patch release to fix a bug we discovered in
`memfault-core-handler`

### Fixed

- A bug in `memfault-core-handler` that caused a small number of coredumps in
  which PT_PHDR is not the first entry in the program header table to be
  captured incorrectly when in `threads` mode.

### Changed

- Add recording of `info!`, `warn!` and `error!` logs from during coredump
  capture to a note in the core ELF to allow for better visibility into coredump
  capture errors.

## [1.9.0] - 2023-12-14

We are excited to introduce support for the
[Memfault Core Metrics](https://docs.memfault.com/docs/platform/memfault-core-metrics)
in `memfaultd` with this release. These metrics enable the monitoring of
connectivity, battery usage, and crashiness out-of-the-box with minimal
configuration.

This release also adds the ability to convert logs into metrics _on the edge_.

### Added

- `memfaultd` now supports built-in capture of connectivity, battery, and
  crashiness metrics. See our docs on [Memfault Core Metrics]
  (https://docs.memfault.com/docs/platform/memfault-core-metrics).
  - Crashiness is measured automatically (any hour without a coredump collected
    will count as a crash-free hour).
  - Battery and connectivity are supported via the
    [`battery_monitor`](https://docs.memfault.com/docs/linux/reference-memfaultd-configuration#battery_monitor)
    and
    [`connectivity_monitor`](https://docs.memfault.com/docs/linux/reference-memfaultd-configuration#connectivity_monitor)
    configuration options, and the new `memfaultctl` commands
    [`add-battery-reading`](https://docs.memfault.com/docs/linux/reference-memfaultctl-cli#add-battery-reading),
    [`report-sync-success`](https://docs.memfault.com/docs/linux/reference-memfaultctl-cli#report-sync-success),
    and
    [`report-sync-failure`](https://docs.memfault.com/docs/linux/reference-memfaultctl-cli#report-sync-failure).
- The ability to convert log into metrics. Further details on this new feature
  are provided in our
  [logging guide](https://docs.memfault.com/docs/linux/logging#converting-logs-into-metrics)
- Support for string reboot reasons in addition to the existing reset code
  integers. This allows users to define custom reboot reasons specific to their
  device or domain. (Visualizations of these custom reboot reason in the backend
  will be shipped in the next few days).

### Changed

- The behavior of how Gauge collectd metrics are aggregated on edge. Prior to
  this change, the last reading collected for a gauge metric was what was sent
  to Memfault in the interval's heartbeat. Now, the value sent up for a gauge is
  the average of all readings for that metric within a given heartbeat.
- openssl version bumped to 10.60.0
- Added the
  [recommended settings](https://docs.memfault.com/docs/linux/metrics#application-metrics)
  `DeleteGauges` and `DeleteCounters` to `true` in `meta-memfault-example`'s
  `collectd.conf`. This is so metrics whose readings are sparsely collected do
  not use resources being uploaded in every upload interval. These can be set to
  `false` if alternate behavior is desired (collectd docs
  [here](https://collectd.org/documentation/manpages/collectd.conf.5.shtml#plugin_statsd))

### Fixed

- A bug related to how `memfaultd` collects coredumps that caused a very small
  number of stacktraces to not be displayed properly in the Memfault app.
- Added `openssl` to `meta-memfault-example`'s Fluent Bit bitbake recipe.
- Added `zlib` as a dependency of `memfaultd` in its bitbake recipe. `zlib` is
  required as we use the `zlib` backend for the `flate2` crate.
- Fixed a bug which caused `memfaultd` to create an empty logfile on shutdown.

## [1.8.1] - 2023-11-6

This is a small release to address a bug we discovered in how MAR entries'
estimated size was being calculated.

This bug can cause MAR entries to be deleted before upload when you are using
the logging feature with significant logging activity. We recommend all
customers using the `logging` feature to update to this version.

### Fixed

- Fix the logic for estimating the number of inodes that a MAR entry will take
  on disk.

## [1.8.0] - 2023-10-25

In this release, we have worked on improving coredump captures and now support
different capture modes which will greatly reduce the size of coredumps and make
them more useful (see [our Coredump Guide][docs-coredumps-strategy]).

[docs-coredumps-strategy]:
  https://docs.memfault.com/docs/linux/coredumps#capture-strategy

### Added

- The coredump capture strategy can now be set to capture the thread stacks of
  the crashed program or all the memory regions included in the ELF core file
  generated by the kernel This behavior is defined by
  `coredump.capture_strategy` in `memfaultd.conf`.
- `memfault-core-handler` now captures the program arguments in the coredump
  metadata. They will be displayed in the Memfault Dashboard.
- A device's software type and version can now be set via `memfault-device-info`
  with `MEMFAULT_SOFTWARE_TYPE` and `MEMFAULT_SOFTWARE_VERSION` respectively.
- The device's software type and version are now validated in `memfaultd`.
  Invalid values will trigger an immediate error.
- The max age of an unuploaded MAR entry can now be configured in
  `memfaultd.conf` via `mar.mar_entry_max_age_seconds`. Once a MAR entry's age
  is greater than this value it will be deleted from disk regardless of if it
  has been uploaded or not. The default is set to 7 days.

### Changed

- The default coredump capture strategy is now 32 KB threads. This means that
  capturing the top 32 kilobytes of every thread instead of capturing all memory
  selected in the kernel configuration. The previous default behavior matched
  setting `coredump.capture_strategy.type` to `kernel_selection`.

### Fixed

- A bug in `meta-rust-bin` that would cause build errors when the target
  architecture is the same as the builder
- Bumped the base Docker image to use `ubuntu:jammy` instead of the deprecated
  `ubuntu:kinetic`.
- A bug causing logs of any level to be written to the kernel logs from the
  `memfault-core-handler`

### Removed

- A number of unused dependencies in `memfaultd`'s Bitbake recipe (vim-native,
  cmake-native, zlib).

## [1.7.0] - 2023-09-27

This new release will mostly interest users of Memfault for Linux on systems
that are not directly connected to the Internet and need another way to
transport the data from the device to the cloud.

### Added

- `memfaultd` now supports exporting its data in multiple formats via it's
  built-in HTTP server. The easiest way to use this new feature is via
  `memfaultctl export`. Supported formats are MAR (Zip file), Chunk (MAR file
  encapsulated in a memfault chunk), Chunk-Wrapped (a chunk with an additional
  header containing a signature and the chunk length).

### Changed

- We have rewritten our core handler in Rust. In this release, the core handler
  is iso-functional to the previous release. New features coming soon!

### Fixed

- When building with OpenSSL, `memfaultd` will not try to build rustls-tls. This
  fixes an issue where `memfaultd` would not build on some systems (mips)
  because a dependency of `rustls-tls` (`ring`) does not build on `mips`.

## [1.6.0] - 2023-09-06

We dropped, or made optional, a number of dependencies. Memfault for Linux will
be easier to integrate and run on a wider variety of configuration.

### Changed

- `memfaultd` does not require `systemd` anymore. On Yocto, the `systemd`
  feature will be activated automatically if your distribution includes systemd.
  When `systemd` is not used, `memfaultd` will not be able to detect "user
  triggered" shutdown or reboot. You should use the [`last_reboot_reason` file
  API][reboot-reason-api] to notify `memfaultd` before doing a normal shutdown.
- `memfaultd` will now default to using a Rust TLS library in place of OpenSSL.
  This adds about 800kB to the `memfaultd` binary. If you do have OpenSSL on
  your system and prefer to use it, you can set the `openssl-tls` option (in
  your `PACKAGECONFIG` for the `memfaultd` recipe) to continue using OpenSSL.
- `memfaultd` now supports an `upload_interval` set to 0. When `upload_interval`
  is 0, `memfaultd` will never try to upload data on its own. Data will be
  written to disk and deleted when the size or inode limits are exceeded. You
  can still call `memfaultctl sync` (or send the SIGUSR1 signal) to force an
  immediate upload.

[reboot-reason-api]:
  https://docs.memfault.com/docs/linux/reboot-reason-tracking#device-specific

### Removed

- Cleaned up some of the upload code to remove some dead paths and gain a bit of
  code size.

### Fixed

- Fluent Bit changed their output format in version 2.1. This version of
  `memfaultd` supports both the old and new format.

## [1.5.0] - 2023-07-18

This release introduces [fleet-sampling][fleet-sampling] to the Linux SDK. It
also uniformizes all data transfers to use the MAR (Memfault Archive) format,
including heartbeats messages which were previously uploaded directly by
collectd to the cloud. `memfaultd` now exposes a local http endpoint to receive
metrics and will upload them as MAR files.

Finally it mostly wraps up our transition to Rust. C is only used to parse
coredumps (the last piece we will eventually rewrite) and in a few rare places
where using Rust is not practical or not possible (calling `libsystemd`, using
`libconfig`, and triggering a segfault).

### Upgrading tldr

This release includes a number of changes that will require changes in your
integration.

Specifically:

- collectd now uses a static configuration file (it's not generated by
  `memfaultd` at run-time). You will need to change your `collectd.conf` file.
  See [the metrics guide](https://docs.memfault.com/docs/linux/metrics) for more
  details.
- Build-time and run-time configuration references to `plugins` have all been
  dropped. Refer to the release notes below for more details.

### Added

- `memfaultctl enable-dev-mode` now prints a link to the Memfault dashboard
  where the user can remove server-side limits for this device.
- Memfault SDK for Linux now supports fleet sampling: you can configure on the
  server which devices are sending coredumps, logs and metrics. [Read our Fleet
  Sampling documentation][fleet-sampling] for more information.
- `memfaultd` now exposes a HTTP server to receive data from `collectd`. Metrics
  can be sent at any frequency and `memfaultd` will aggregate them and upload
  them on the heartbeat interval (by default, every 60 minutes). Metrics are
  stored and uploaded using MAR entries.
- The configuration variable `heartbeat_interval_seconds` controls the
  aggregation frequency of metrics.
- All commands now support the `--verbose`/`-V` flag to show extra debug
  information.

### Changed

- The configuration value `refresh_interval_seconds` has been renamed to
  `upload_interval_seconds` to avoid confusion with the new
  `heartbeat_interval_seconds`.
- The names of the `memfaultd` configuration options have been changed to drop
  the `plugin_` prefix. If you are manually setting the `PACKAGECONFIG` variable
  for the `memfaultd` recipe, you will need to update it.

  For example, to enable all features (the default):

  ```
  -PACKAGECONFIG := "plugin_swupdate plugin_collectd plugin_coredump plugin_logging"
  +PACKAGECONFIG := "swupdate collectd coredump logging"
  ```

  We are moving away from the terms "plugins" to "features" to describe the
  different components of the Memfault SDK for Linux. This better represents how
  they are built and enabled.

  Read the [integration-guide][integration-configure-options] for more
  information.

- The names of some configuration options have been changed and you will need to
  edit your configuration file if you changed any of them: `coredump_plugin` is
  now `coredump`, `swupdate_plugin` is now `swupdate`, `reboot_plugin` is now
  `reboot`.

  ```
  -"swupdate_plugin": {
  +"swupdate": {
    "input_file": "/etc/swupdate.cfg",
    "output_file": "/tmp/swupdate.cfg"
  },
  -"reboot_plugin": {
  +"reboot": {
    "last_reboot_reason_file": "/media/last_reboot_reason"
  },
  -"coredump_plugin": {
  +"coredump": {
    "coredump_max_size_kib": 96000,
    "compression": "gzip",
    "rate_limit_count": 5,
    "rate_limit_duration_seconds": 3600
  },
  ```

- Coredumps are now stored and uploaded using MAR files.
- Reboots are now stored and uploaded using MAR files.
- Memfault SDK for Linux is now written mostly in Rust. The use of C code is now
  minimal.
- Our recommended configuration for collectd (`collectd_%.bbappend`) is now in
  `meta-memfault`. It was previously in `meta-memfault-example`.

### Removed

- `memfaultd` will not write the `collectd` configuration file automatically on
  startup. We recommend using a static configuration file instead.
- We have removed external dependency on `libjsonc` and `libuuid` with the
  rewrite in Rust.

### Fixed

- Fixed build issues for some combination of feature flags.
- Bug where `memfaultd` would show a warning about systemd being in an
  unexpected state when restarting.
- Bug where we would show warnings about invalid `memfault-device-info` output
  multiple times.

[fleet-sampling]: https://docs.memfault.com/docs/platform/fleet-sampling/
[integration-configure-options]:
  https://docs.memfault.com/docs/linux/integration-guide#optional-opt-out-of-memfaultd-default-features

## [1.4.0] - 2023-04-25

### tldr

This release includes a number of changes that will require changes in your
project:

- **Edit your `bblayers.conf`** to stop using `meta-rust-bin` layer from the
  rust-embedded GitHub account and use the version provided in the
  `memfault-linux-sdk` repository.
- **Edit `memfault.conf`** to replace `data_dir` by `persist_dir` and carefully
  review `tmp_dir` (which defaults to `persist_dir`) and associated options to
  control maximum usage and minimum headroom. You will most likely need to set
  your own values.
- If you were calling `memfaultd --enable-data-collection` before, you need to
  replace it by `memfaultctl enable-data-collection` now.

### Added

- Memfaultd will now consider the amount of disk space and inodes remaining on
  disk when writing logs, storing coredumps and when cleaning the MAR staging
  area. See new options `tmp_dir_min_headroom_kib`, `tmp_dir_min_inodes` and
  `tmp_dir_max_usage_kib` in the [configuration file][reference-configuration].
- Logging is now rate limited on device (defaults to 500 lines per minute - see
  [`max_lines_per_minute`][reference-logging]).
- We simplified the configuration options relative to data storage. Users are
  now expected to set a `persist_dir` option that must be persisted across
  reboots and a `tmp_dir` option that can be cleared on reboot (a temp
  filesystem in RAM). Refer to [Memfault Integration Guide -
  Storage][integration-storage] for more details.
- Option `logs.compression_level` to set the logs compression level.

### Changed

- Memfault Linux SDK now ships with a version of `meta-rust-bin` using a renamed
  Yocto class `cargo_bin`. This was required due to `meta-rust-bin` being
  incompatible with some poky packages. We will track the upstream bug and
  switch back to upstream `meta-rust-bin` when possible (see meta-rust-bin#135).
- `memfaultd` does not include the commands `enable-dev-mode` and
  `enable-data-collection` anymore (they were deprecated in 1.2.0.)
- We now consider logging to be ready for production use and have turned on
  `plugin_logging` by default.
- Some CMake improvements to build with older versions of GCC.
- Rewrote more `memfaultctl` commands to rust: `trigger-coredump`,
  `show-settings`, `sync`, `write-attributes`, `enable-dev-mode` and
  `enable-data-collection`.

### Removed

- Configuration options `logs.tmp_folder`, `mar.storage_max_usage_kib`,
  `coredump.storage_max_usage_kib` and `coredump.storage_min_headroom_kib` have
  been removed and are replaced by the new options listed above.
- `memfaultd --enable-data-collection` and `--enable-dev-mode` (as well as
  `--disable...`) have been removed.

### Fixed

- Bug causing coredump-handler to not capture coredumps in development mode.
- Bug causing coredump-handler to create a ratelimiter in the wrong place and
  fail the capture when it did not have permission to create the file.
- Fluent-bit connector will drop all logs when data collection is not enabled.
- Fluent-bit recommended configuration now includes a `Retry_Limit`.
- Wait until memfaultd is ready to write PID file.
- Fixed occasional error message `error sending on closed channel` on shutdown.
- Fix bug where `memfaultd` and `memfaultctl` would not properly report their
  version number.
- Show immediate error to the user when `memfaultctl write-attributes` is called
  but data collection is disabled.
- Fix build error when logging was disabled.

[integration-storage]:
  https://docs.memfault.com/docs/linux/integration-guide#configure-memfaultd-storage
[reference-configuration]:
  https://docs.memfault.com/docs/linux/reference-memfaultd-configuration#top-level-etcmemfaultdconf-configuration
[reference-logging]:
  https://docs.memfault.com/docs/linux/reference-memfaultd-configuration#logs

## [1.3.2] - 2023-04-06

### Changed

- The Yocto layer meta-memfault does not depend on swupdate, collectd and
  fluent-bit anymore. Instead these dependencies are added by the memfaultd
  recipe and only when the corresponding plugins are enabled.

### Fixed

- Fix Yocto recipe to always enable network access during compilation and add
  `openssl` as a dependency.
- Updated architecture diagram to include fluent-bit

## [1.3.1] - 2023-03-22

### Added

- Add configuration in `meta-memfault-example` to run on Raspberry Pi 2/3/4.

### Changed

- Log files are now stored compressed on disk to reduce disk usage.
- To upload Memfault MAR entries (including logs), they are now streamed
  directly from disk without writing the MAR zip file to disk. This reduces disk
  I/O (flash wear) and means logs are only written once to disk which is
  optimal.
- Display server error text for Memfault API endpoints. This helps debug
  configuration issues.
- Validate the provided `device_id` and show an error if it will not be accepted
  by Memfault.
- Removed memfaultd dependency on libuboot. It was used to detect OTA reboots
  but we are now configuring swupdate to call `memfaultctl reboot --reason 3`
  after installing an upgrade.

### Fixed

- Fixed consistency of logfiles' Cid/NextCid which will help the Memfault
  dashboard identify discontinuity in the series of logs.
- Fixed the sleep duration displayed after a network error (memfaultd would
  announce sleeping for an hour but it would actually retry sooner).
- Fix a configuration problem where `collectd` logs would not be visible in the
  Memfault Dashboard (logs sent only to syslog are not captured by the default
  configuration - we are now configuring `collectd` to log to the standard
  output which is captured by `journald`).

## [1.3.0] - 2023-03-06

### Added

- Memfault SDK on Linux now supports Memfault archives (MAR), also used in our
  Android SDK. Going forward this is how all data will be stored on disk.
- A local TCP endpoint, compatible with fluent-bit tcp output plugin, is now
  available to capture logs. Logs are written to disk in MAR (Memfault ARchive)
  format and uploaded to Memfault when the device is online. **This feature is
  in technical preview stage and is disabled by default.** See [logging on
  linux][linux-logging] for more information.
- `meta-memfault-example` now includes fluent-bit to demonstrate how to collect
  logs.
- Memfault Linux SDK is now partially written in Rust. Our Yocto layer requires
  cargo and rust 1.65.0. We recommend [meta-rust-bin] from the rust-embedded
  project.
  - 🚧 `memfaultd` in the Linux SDK is currently a mix of C code and Rust.
    Please excuse the noise while we continue construction. 🚧
- Memfault agent can now be built on Linux and macOS systems (`cargo build`).

[meta-rust-bin]: https://github.com/rust-embedded/meta-rust-bin
[linux-logging]: https://docs.memfault.com/docs/linux/logging

### Changed

- `memfaultd` can now capture coredumps of itself.

### Fixed

- Fix bug where we restarted swupdate instead of swupdate.service. This removes
  a warning in the logs.
- Added link to the changelog in the release notes.
- Fix a bug where memfault would ignore SIGUSR1 signal while it was processing
  uploads.
- Fix a bug in the coredump capturing code that would cause a crash in case more
  than 16 warnings got emitted during the capture process. Thanks to
  [@attilaszia](https://github.com/attilaszia) for reporting this issue.

## [1.2.0] - 2022-12-26

### Added

- [memfaultctl] Added a new command `memfaultctl` to interact with `memfaultd`.
  - `memfaultctl trigger-coredump` to force a coredump generation and upload.
  - `memfaultctl request-metrics` to force `collectd` to flush metrics to
    Memfault.
  - `memfaultctl reboot` to save a reboot reason and restart the system.
  - `memfaultctl sync` to process `memfaultd` queue immediately.
  - `memfaultctl write-attributes` to push device attributes to Memfault.
  - 'Developer Mode` to reduce rate limits applied to coredumps during
    development.

### Changed

- Our Docker container now runs on Apple silicon without Rosetta emulation.
- Updated the `memfault-cli` package in the Docker image.
- Added "preferred versions" for `swupdate` and `collectd`.
- Coredumps are now compressed with gzip reducing storage and network usage.
- `memfaultd` is now built with `-g3`.

### Deprecated

- `memfaultd --(enable|disable)-dev-collection` and `memfaultctl -s` are now
  replaced by equivalent commands on `memfaultctl` and will be removed in a
  future version.

### Fixed

- `swupdate` would get in a bad state after reloading `memfaultd`. This is fixed
  by restarting both `swupdate` and `swupdate.socket` units.

## [1.1.0] - 2022-11-10

### Added

- [memfaultd] A new `last_reboot_reason_file` API has been added to enable
  extending the reboot reason determination subsystem. More information can be
  found in [the documentation of this feature][docs-reboots].
- [memfaultd] `memfaultd` will now take care of cleaning up `/sys/fs/pstore`
  after a reboot of the system (but only if the reboot reason tracking plugin,
  `plugin_reboot`, is enabled). Often, [systemd-pstore.service] is configured to
  carry out this task. This would conflict with `memfaultd` performing this
  task. Therefore, [systemd-pstore.service] is automatically excluded when
  including the `meta-memfault` layer. Note that `memfaultd` does not provide
  functionality (yet) to archive pstore files (like [systemd-pstore.service]
  can). If this is necessary for you, the work-around is to create a service
  that performs the archiving and runs before `memfaultd.service` starts up.

### Changed

- [memfaultd] When `memfaultd` would fail to determine the reason for a reboot,
  it would assume that "low power" was reason for the reboot. This makes little
  sense because there are many resets for which `memfaultd` is not able to
  determine a reason. This fallback is now changed to use "unspecified" in case
  the reason could not be determined (either from the built-in detection or
  externally, via the new `last_reboot_reason_file` API). Read the [new
  `last_reboot_reason_file` API][docs-reboots] for more information.
- Various improvements to the QEMU example integration:
  - It can now also be built for `qemuarm` (previously, only `qemuarm64` was
    working).
  - Linux pstore/ramoops subsystems are now correctly configured for the QEMU
    example integration, making it possible to test out the tracking of kernel
    panic reboot reasons using the QEMU device.
- [memfaultd] The unit test set up is now run on `x86_64` as well as `i386` to
  get coverage on a 64-bit architecture as well as a 32-bit one.

### Fixed

- [memfaultd] Building the SDK on 32-bit systems would fail due to compilation
  errors. These are now fixed.
- [collectd] In the example, the statsd plugin would be listening on all network
  interfaces. This is narrowed to only listen on localhost (127.0.0.1).
- [memfaultd] Many improvements to reboot reason tracking:
  - Intermittently, a reboot would erroneously be attributed to "low power".
  - Kernel panics would show up in the application as "brown out reset".
  - Sometimes, multiple reboot events for a single Linux reboot would get
    emitted. The root causes have been found and fixed. Logic has been added
    that tracks the Linux `boot_id` to ensure that at most one reboot reason
    gets emitted per Linux boot.
  - When using the example integration, the reboot reason "firmware update"
    would not be detected after SWUpdate had installed an OTA update. This was
    caused by a mismatch of the `defconfig` file in the example integration and
    the version of SWUpdate that was being compiled. This is now corrected.
- [memfaultd] Fixed a bug in queue.c where an out-of-memory situation could lead
  to the queue's mutex not getting released.
- Improved the reliability of some of the E2E test scripts.

### Known Issues

- When `memfaultd --enable-data-collection` is run and data collection had not
  yet been enabled, it will regenerate the SWUpdate configuration and restart
  the `swupdate.service`. This restart can cause SWUpdate to get into a bad
  state and fail to install OTA updates. This is not a new issue and was already
  present in previous releases. We are investigating this issue. As a
  work-around, the device can be rebooted immediately after running
  `memfaultd --enable-data-collection`.
- The [systemd-pstore.service] gets disabled when including `meta-memfault`,
  even if `plugin_reboot` is disabled. As a work-around, if you need to keep
  [systemd-pstore.service], remove the `systemd_%.bbappend` file from the SDK.

[1.1.0]:
  https://github.com/memfault/memfault-linux-sdk/releases/tag/1.1.0-kirkstone
[systemd-pstore.service]:
  https://www.freedesktop.org/software/systemd/man/systemd-pstore.service.html
[docs-reboots]: https://mflt.io/linux-reboots

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

The server-side issue mentioned below has been resolved in the meantime.

~~Temporarily, our backend processing pipeline is unable to process coredumps
that link to shared objects in a specific style. This affects, in particular,
coredumps coming from devices on the Dunfell release of Yocto.~~

~~A backend fix has already been identified and should be released in the next
few business days. Once released, any previously collected coredumps that are
affected will be reprocessed server-side to address this issue. This will
**not** require any action from your team.~~

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
[1.2.0]:
  https://github.com/memfault/memfault-linux-sdk/releases/tag/1.2.0-kirkstone
[1.3.0]:
  https://github.com/memfault/memfault-linux-sdk/releases/tag/1.3.0-kirkstone
[1.3.1]:
  https://github.com/memfault/memfault-linux-sdk/releases/tag/1.3.1-kirkstone
[1.3.2]:
  https://github.com/memfault/memfault-linux-sdk/releases/tag/1.3.2-kirkstone
[1.4.0]:
  https://github.com/memfault/memfault-linux-sdk/releases/tag/1.4.0-kirkstone
[1.5.0]:
  https://github.com/memfault/memfault-linux-sdk/releases/tag/1.5.0-kirkstone
[1.6.0]:
  https://github.com/memfault/memfault-linux-sdk/releases/tag/1.6.0-kirkstone
[1.7.0]:
  https://github.com/memfault/memfault-linux-sdk/releases/tag/1.7.0-kirkstone
[1.8.0]:
  https://github.com/memfault/memfault-linux-sdk/releases/tag/1.8.0-kirkstone
[1.8.1]:
  https://github.com/memfault/memfault-linux-sdk/releases/tag/1.8.1-kirkstone
[1.9.0]:
  https://github.com/memfault/memfault-linux-sdk/releases/tag/1.9.0-kirkstone
[1.9.1]:
  https://github.com/memfault/memfault-linux-sdk/releases/tag/1.9.1-kirkstone
[1.10.0]:
  https://github.com/memfault/memfault-linux-sdk/releases/tag/1.10.0-kirkstone
[1.11.0]:
  https://github.com/memfault/memfault-linux-sdk/releases/tag/1.11.0-kirkstone
