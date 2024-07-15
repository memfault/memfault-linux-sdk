# Memfault Linux SDK

Ship hardware products at the speed of software. With Memfault, you can
continuously monitor devices, debug firmware issues, and deploy OTA updates to
your fleet.

## Overview

The Memfault Linux SDK consists of a combination of existing open-source
software and a daemon [`memfaultd`][source-memfaultd] that orchestrates and
configures it. It also implements additional features such as tracking of reboot
reason tracking, and in the future reporting crashes and logs.

<p>
  <figure>
    <img
      src="/README-overview.svg"
      alt="Overview of the Memfault Linux SDK"
    />
    <figcaption>
      Dotted lines represent runtime configuration, and solid lines represent flow
      of data. Faded-out elements represent upcoming features.
    </figcaption>
  </figure>
</p>

To provide core Memfault platform features, the Memfault Linux SDK relies on
well-established, battle-tested open-source software. The daemon `memfaultd`
implements Memfault-specific features and also acts as a configuration agent.

[source-memfaultd]:
  https://github.com/memfault/memfault-linux-sdk/blob/-/meta-memfault/recipes-memfault/memfaultd/files/memfaultd

## Quickstart

To get started in minutes using the QEMU emulator, read our [Quick Start with
QEMU guide][quickstart-qemu].

If you have a Raspberry Pi available, the [Quick Start with Raspberry Pi
guide][quickstart-rpi] will walk you through building a complete system image,
flashing it to an SDCard and updating your device remotely with Memfault OTA.

[quickstart-qemu]: https://docs.memfault.com/docs/linux/quickstart
[quickstart-rpi]: https://docs.memfault.com/docs/linux/quickstart-raspberrypi

## Prerequisites

Even though support for a broader diversity of setups is planned, this first
versions of our SDK makes the following assumptions:

- Your project uses [Yocto][yocto-homepage] as a build system.
- It uses [SWUpdate][swupdate-homepage] for OTA (optional if you don't plan to
  integrate with OTA).

If your project diverges from these assumptions, please [get in
touch][get-in-touch]. It will likely still work without major changes.

[get-in-touch]: https://memfault.com/contact/

## Getting Started

Take a look at our [getting-started guide][docs-linux-getting-started] to set up
your integration.

OTA/Release Management is currently fully supported through an off-the-shelf
integration with the SWUpdate agent. Read more about it in the [OTA integration
guide][docs-linux-ota].

Metrics are also supported through [collectd][collectd-homepage]. Read more
about it in the [Linux Metrics integration guide][docs-linux-metrics].

[swupdate-homepage]: https://swupdate.org/
[yocto-homepage]: https://www.yoctoproject.org/

## Documentation and Features

- Detailed documentation for the Memfault Linux SDK can be found in our online
  docs: see the [introduction][docs-linux-introduction] and the [getting-started
  guide][docs-linux-getting-started].
- Visit our [features overview][docs-platform] for a generic introduction to all
  the major features of the Memfault platform.

[docs-platform]: https://docs.memfault.com/docs/platform/introduction/
[docs-linux-introduction]: https://docs.memfault.com/docs/linux/introduction
[docs-linux-getting-started]: https://mflt.io/linux-getting-started

An integration example can be found under
[`/meta-memfault-example`](/meta-memfault-example). The central part of the SDK
lives in a Yocto layer in [`/meta-memfault`](/meta-memfault).

### OTA Updates

To provide OTA Updates, the Memfault Cloud implements an API endpoint compatible
with the [hawkBit DDI API][hawkbit-ddi]. Various clients are available, but
`memfaultd` supports [SWUpdate][swupdate-homepage] out of the box and is able to
configure it to talk to our hawkBit DDI-compatible endpoint.

Read more about [Linux OTA management using Memfault][docs-linux-ota].

[docs-linux-ota]: https://mflt.io/linux-ota-integration-guide
[hawkbit-homepage]: https://www.eclipse.org/hawkbit/
[hawkbit-ddi]: https://www.eclipse.org/hawkbit/apis/ddi_api/
[swupdate-homepage]: https://swupdate.org/

### Metrics

`memfaultd` can collect system metrics internally or use
[collectd][collectd-homepage] for the collection and transmission of metrics.
Application metrics can be sent to to `memfaultd` directly or through `collectd`
by means of [`statsd`][statsd-homepage].

Read more about [Linux metrics using Memfault][docs-linux-metrics].

[docs-linux-metrics]: https://mflt.io/linux-metrics
[collectd-homepage]: https://collectd.org/
[statsd-homepage]: https://github.com/statsd/statsd

### Crash Reports

To collect and upload user-land coredumps, the `memfaultd` relies the standard
kernel [coredump][man-core] feature, and so does not need to make use of any
additional dependencies. Read more about [coredumps using the Memfault Linux
SDK][docs-linux-coredumps].

[docs-linux-coredumps]: https://mflt.io/linux-coredumps
[man-core]: https://man7.org/linux/man-pages/man5/core.5.html

### Reboot Reason Tracking

Memfault will detect various reboot reasons from the system and report them to
the Memfault Dashboard. Users can also provide a specific reboot reason before
restarting the device. Read more about [reboot reason tracking using
`memfaultd`][docs-reboots].

[docs-reboots]: https://mflt.io/linux-reboots

### Log files

System logs can be captured directly from `journald` or from `fluent-bit` and
forwarded to `memfaultd`.

Logs are compressed, stored locally and uploaded when the device connects to the
Internet.

For more information on how to configure log file support, please refer to [the
linux logging guide][docs-logging]

[docs-logging]: https://docs.memfault.com/docs/linux/logging
