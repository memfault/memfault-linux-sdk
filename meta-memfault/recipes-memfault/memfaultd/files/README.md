# `memfaultd`

`memfaultd` is a daemon that runs on your device and collects crash reports and
metrics. It is the core of the
[Memfault Linux SDK](https://github.com/memfault/memfault-linux-sdk/blob/kirkstone/README.md).

## Overview

`memfaultd` supports several features to help you maintain and debug your fleet
of devices:

- **Crash reporting**: When your device crashes, `memfaultd` collects a crash
  report from
  [Linux Coredumps](https://man7.org/linux/man-pages/man5/core.5.html). For more
  information, see the
  [Coredumps documentation](https://docs.memfault.com/docs/linux/coredumps).

- **Metrics**: `memfaultd` collects metrics from your device and uploads them to
  Memfault. For more information, see the
  [Metrics documentation](https://docs.memfault.com/docs/linux/metrics).

- **Reboot reason tracking**: `memfaultd` detects various reboot reasons from
  the system and reports them to the Memfault Dashboard. Users can also provide
  a specific reboot reason before restarting the device. For more information,
  see the
  [Reboot Reason Tracking documentation](https://docs.memfault.com/docs/linux/reboot-reason-tracking).

- **OTA Updates**: `memfaultd` supports [SWUpdate](https://swupdate.org/) out of
  the box and is able to configure it to talk to our hawkBit DDI-compatible
  endpoint. For more information, see the
  [Linux OTA Management documentation](https://docs.memfault.com/docs/linux/ota).

- **Logging**: `memfaultd` collects logs from your system. For more information,
  see the [Logging documentation](https://docs.memfault.com/docs/linux/logging).

And much more! [Register](https://app.memfault.com/register) and get started
today!
