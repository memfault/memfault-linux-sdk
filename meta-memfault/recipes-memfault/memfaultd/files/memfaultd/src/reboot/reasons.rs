//
// Copyright (c) Memfault, Inc.
// See License.txt for details
use serde_repr::{Deserialize_repr, Serialize_repr};
use strum_macros::{Display, EnumString, FromRepr};

/// Definitions for reboot reasons
///
/// See the [Memfault docs](https://docs.memfault.com/docs/platform/reference-reboot-reason-ids/)
/// for more details.
#[derive(
    Debug,
    Clone,
    Copy,
    EnumString,
    Display,
    Serialize_repr,
    Deserialize_repr,
    PartialEq,
    Eq,
    FromRepr,
)]
#[repr(u32)]
pub enum RebootReason {
    Unknown = 0x0000,

    //
    // Normal Resets
    //
    UserShutdown = 0x0001,
    UserReset = 0x0002,
    FirmwareUpdate = 0x0003,
    LowPower = 0x0004,
    DebuggerHalted = 0x0005,
    ButtonReset = 0x0006,
    PowerOnReset = 0x0007,
    SoftwareReset = 0x0008,

    /// MCU went through a full reboot due to exit from lowest power state
    DeepSleep = 0x0009,
    /// MCU reset pin was toggled
    PinReset = 0x000A,

    //
    // Error Resets
    //
    /// Can be used to flag an unexpected reset path. i.e NVIC_SystemReset() being called without any
    /// reboot logic getting invoked.
    UnknownError = 0x8000,
    Assert = 0x8001,

    /// Deprecated in favor of HardwareWatchdog & SoftwareWatchdog.
    ///
    /// This way, the amount of watchdogs not caught by software can be easily tracked.
    WatchdogDeprecated = 0x8002,

    BrownOutReset = 0x8003,
    Nmi = 0x8004, // Non-Maskable Interrupt

    // More details about nomenclature in https://mflt.io/root-cause-watchdogs
    HardwareWatchdog = 0x8005,
    SoftwareWatchdog = 0x8006,

    /// A reset triggered due to the MCU losing a stable clock.
    ///
    /// This can happen, for example, if power to the clock is cut or the lock for the PLL is lost.
    ClockFailure = 0x8007,

    /// A software reset triggered when the OS or RTOS end-user code is running on top of identifies
    /// a fatal error condition.
    KernelPanic = 0x8008,

    /// A reset triggered when an attempt to upgrade to a new OTA image has failed and a rollback
    /// to a previous version was initiated
    FirmwareUpdateError = 0x8009,

    // Resets from Arm Faults
    BusFault = 0x9100,
    MemFault = 0x9200,
    UsageFault = 0x9300,
    HardFault = 0x9400,
    /// A reset which is triggered when the processor faults while already
    /// executing from a fault handler.
    Lockup = 0x9401,
}
