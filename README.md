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
  https://github.com/memfault/memfault-linux-sdk/tree/master/meta-memfault/recipes-memfault/memfaultd/files/memfaultd

## Prerequisites

Even though support for a broader diversity of setups is planned, this first
versions of our SDK makes the following assumptions:

- Your project uses [Yocto][yocto-homepage] as a build system.
- It uses [`systemd`][systemd-homepage] as an init system.
- It uses [SWUpdate][swupdate-homepage] for OTA (optional if you don't plan to
  integrate with OTA).

If your project diverges from these assumptions, please [get in
touch][get-in-touch]. It will likely still work without major changes.

[get-in-touch]: https://memfault.com/contact/

## Getting Started

Take a look at our [getting-started guide][docs-getting-started] to set up your
integration.

OTA/Release Management is currently fully supported through an off-the-shelf
integration with the SWUpdate agent. Read more about it in the [OTA integration
guide][docs-ota].

[systemd-homepage]: https://systemd.io/
[swupdate-homepage]: https://swupdate.org/
[yocto-homepage]: https://www.yoctoproject.org/
[docs-getting-started]:
  https://docs.memfault.com/docs/linux/linux-getting-started-guide
[docs-ota]:
  https://docs.memfault.com/docs/linux/linux-releases-integration-guide

## Documentation and Features

- Detailed documentation for the Memfault Linux SDK can be found in our online
  docs: see the [introduction][docs-linux-introduction] and the [getting-started
  guide][docs-linux-getting-started].
- Visit our [features overview][docs-platform] for a generic introduction to all
  the major features of the Memfault platform.

[docs-platform]: https://docs.memfault.com/docs/platform/introduction/
[docs-linux-introduction]: https://docs.memfault.com/docs/linux/introduction
[docs-linux-getting-started]:
  https://docs.memfault.com/docs/linux/linux-getting-started-guide

An integration example can be found under
[`/meta-memfault-example`](/meta-memfault-example). The central part of the SDK
lives in a Yocto layer in [`/meta-memfault`](/meta-memfault).

### OTA Updates

To provide OTA Updates, the Memfault Cloud implements an API endpoint compatible
with the [hawkBit DDI API][hawkbit-ddi]. Various clients are available, but
`memfaultd` supports [SWUpdate][swupdate-homepage] out of the box and is able to
configure it to talk to our hawkBit DDI-compatible endpoint.

[hawkbit-homepage]: https://www.eclipse.org/hawkbit/
[hawkbit-ddi]: https://www.eclipse.org/hawkbit/apis/ddi_api/
[swupdate-homepage]: https://swupdate.org/

### Metrics (available with next release)

The Memfault Linux SDK relies on [`collectd`][collectd-homepage] for the
collection and transmission of metrics. Application metrics can be sent to
`collectd` by means of [`statsd`][statsd-homepage].

[collectd-homepage]: https://collectd.org/
[statsd-homepage]: https://github.com/statsd/statsd

### Reboot Reason Tracking, Crash Reports and Logs

These features are fully in the domain of `memfaultd`. Note that crash reports
and logs are upcoming features, while reboot reason tracking is supported today.
