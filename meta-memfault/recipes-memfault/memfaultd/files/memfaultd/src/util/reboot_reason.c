//! @file
//!
//! Copyright (c) Memfault, Inc.
//! See License.txt for details
//!
//! @brief
//!

#include "memfault/util/reboot_reason.h"

bool memfaultd_is_reboot_reason_valid(eMemfaultRebootReason reboot_reason) {
  switch (reboot_reason) {
    case kMfltRebootReason_Unknown:
    case kMfltRebootReason_UserShutdown:
    case kMfltRebootReason_UserReset:
    case kMfltRebootReason_FirmwareUpdate:
    case kMfltRebootReason_LowPower:
    case kMfltRebootReason_DebuggerHalted:
    case kMfltRebootReason_ButtonReset:
    case kMfltRebootReason_PowerOnReset:
    case kMfltRebootReason_SoftwareReset:
    case kMfltRebootReason_DeepSleep:
    case kMfltRebootReason_PinReset:
    case kMfltRebootReason_UnknownError:
    case kMfltRebootReason_Assert:
    case kMfltRebootReason_WatchdogDeprecated:
    case kMfltRebootReason_BrownOutReset:
    case kMfltRebootReason_Nmi:
    case kMfltRebootReason_HardwareWatchdog:
    case kMfltRebootReason_SoftwareWatchdog:
    case kMfltRebootReason_ClockFailure:
    case kMfltRebootReason_KernelPanic:
    case kMfltRebootReason_FirmwareUpdateError:
    case kMfltRebootReason_BusFault:
    case kMfltRebootReason_MemFault:
    case kMfltRebootReason_UsageFault:
    case kMfltRebootReason_HardFault:
    case kMfltRebootReason_Lockup:
      return true;
    default:
      return false;
  }
}

const char *memfaultd_reboot_reason_str(eMemfaultRebootReason reboot_reason) {
  switch (reboot_reason) {
    case kMfltRebootReason_Unknown:
      return "Unknown";
    case kMfltRebootReason_UserShutdown:
      return "UserShutdown";
    case kMfltRebootReason_UserReset:
      return "UserReset";
    case kMfltRebootReason_FirmwareUpdate:
      return "FirmwareUpdate";
    case kMfltRebootReason_LowPower:
      return "LowPower";
    case kMfltRebootReason_DebuggerHalted:
      return "DebuggerHalted";
    case kMfltRebootReason_ButtonReset:
      return "ButtonReset";
    case kMfltRebootReason_PowerOnReset:
      return "PowerOnReset";
    case kMfltRebootReason_SoftwareReset:
      return "SoftwareReset";
    case kMfltRebootReason_DeepSleep:
      return "DeepSleep";
    case kMfltRebootReason_PinReset:
      return "PinReset";
    case kMfltRebootReason_UnknownError:
      return "UnknownError";
    case kMfltRebootReason_Assert:
      return "Assert";
    case kMfltRebootReason_WatchdogDeprecated:
      return "WatchdogDeprecated";
    case kMfltRebootReason_BrownOutReset:
      return "BrownOutReset";
    case kMfltRebootReason_Nmi:
      return "Nmi";
    case kMfltRebootReason_HardwareWatchdog:
      return "HardwareWatchdog";
    case kMfltRebootReason_SoftwareWatchdog:
      return "SoftwareWatchdog";
    case kMfltRebootReason_ClockFailure:
      return "ClockFailure";
    case kMfltRebootReason_KernelPanic:
      return "KernelPanic";
    case kMfltRebootReason_FirmwareUpdateError:
      return "FirmwareUpdateError";
    case kMfltRebootReason_BusFault:
      return "BusFault";
    case kMfltRebootReason_MemFault:
      return "MemFault";
    case kMfltRebootReason_UsageFault:
      return "UsageFault";
    case kMfltRebootReason_HardFault:
      return "HardFault";
    case kMfltRebootReason_Lockup:
      return "Lockup";
    default:
      return "???";
  }
}
