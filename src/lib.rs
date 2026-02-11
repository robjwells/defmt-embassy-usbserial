//! [`defmt`] global logger over USB serial for use with [Embassy].
//!
//! To use this crate spawn the [`run`] task, and use `defmt` as normal. Messages will be sent via
//! USB-CDC to the host, where you should use something such as the [`defmt-print`] CLI tool to
//! print them to your terminal.
//!
//! ## Quickstart
//!
//! Here's an example of using it with [`embassy_rp`], with the general HAL setup elided.
//!
//! ```no_run
//! # #![no_std]
//! # #![no_main]
//! # use embassy_rp::{bind_interrupts, Peri};
//! # use embassy_time::Instant;
//! # use embedded_hal_async::delay::DelayNs;
//! # use panic_halt as _;
//! # bind_interrupts!(struct Irqs {
//! #     USBCTRL_IRQ => embassy_rp::usb::InterruptHandler<embassy_rp::peripherals::USB>;
//! # });
//! #[embassy_executor::task]
//! async fn defmtusb_wrapper(usb: Peri<'static, embassy_rp::peripherals::USB>) {
//!     let driver = embassy_rp::usb::Driver::new(usb, Irqs);
//!     let usb_config = {
//!         let mut c = embassy_usb::Config::new(0x1234, 0x5678);
//!         c.serial_number = Some("defmt");
//!         c.max_packet_size_0 = 64;
//!         c.composite_with_iads = true;
//!         c.device_class = 0xEF;
//!         c.device_sub_class = 0x02;
//!         c.device_protocol = 0x01;
//!         c
//!     };
//!     defmt_embassy_usbserial::run(driver, usb_config).await;
//! }
//! #
//! # #[embassy_executor::main]
//! # async fn main(spawner: embassy_executor::Spawner) {
//! #     let peripherals = embassy_rp::init(Default::default());
//! #     let mut delay = embassy_time::Delay;
//!
//! // Inside your main function.
//! spawner.must_spawn(defmtusb_wrapper(peripherals.USB));
//! loop {
//!     defmt::info!("Hello! {=u64:ts}", Instant::now().as_secs());
//!     delay.delay_ms(1000).await;
//! }
//! # }
//! ```
//!
//! ## Wrapper task
//!
//! A wrapper task is required because this library can't provide a task for you to spawn, since it
//! has to be generic over the USB driver struct. While the quickstart example provides a
//! straightforward example of constructing both the driver and the configuration in this task,
//! ultimately the only requirement is that it awaits [`defmt_embassy_usbserial::run`].
//!
//! Of course, `run` is just an async function whose returned future can be `join`ed, etc.
//!
//! ## Configuration
//!
//! For USB-CDC to be set up properly, you _must_ set the correct values in the configuration
//! struct. If `composite_with_iads` is `true` (the default), you _must_ use the following values
//! as `embassy-usb` will [fail an assertion][eusb-assert] if you do not:
//!
//! | Field | Value |
//! |-------|-------|
//! |`device_class`|`0xEF`|
//! |`device_sub_class`|`0x02`|
//! |`device_protocol`|`0x01`|
//!
//! If `composite_with_iads` is `false`, you do not have to use these exact values: the standard
//! CDC device class code (`device_class`) is `0x02`. You should choose the values appropriate to
//! your application. If your only concern is transporting defmt logs over USB serial, default to
//! the values in the table above.
//!
//! ## Examples
//!
//! Please see the `device-examples/` directory in the repository for device-specific "hello world"
//! examples. These examples have all been tested on real hardware and are known to work.
//!
//! ## Known limitations
//!
//! ### Old or corrupt defmt messages are received after reconnecting
//!
//! If you stop reading the logs from the USB serial port, for example by closing `defmt-print`,
//! when you reconnect you will likely receive part of one old defmt message (and potentially
//! several complete out-out-date messages), and the first up-to-date defmt message will be
//! corrupt.
//!
//! This is ultimately because the internal buffers are not aware of defmt frame boundaries. The
//! first case will occur because the writing task will block part-way through writing an internal
//! buffer to the USB serial port, and continues writing that now-stale buffer when you start
//! reading again. (This may be avoided in a future release by way of a timeout.)
//!
//! The second is because that buffer may end part-way through a defmt message, and the next buffer
//! that is written will likely start part-way through a defmt message. `defmt-print` may
//! explicitly report these frames as malformed, or may silently misinterpret values to be included
//! in a format message.
//!
//! Note as well that ceasing to read from the serial port does not disable defmt logging; it seems
//! that only disconnecting from USB will trigger the event that toggles the logger.
//!
//! ## Acknowledgements
//!
//! Thank you to spcan, the original author of defmtusb. Thanks as well to the friendly and helpful
//! members of the various embedded Rust Matrix rooms.
//!
//! ## License
//!
//! Dual-licensed under the Mozilla Public License 2.0 and the MIT license, at your option.
//!
//! [`defmt`]: https://github.com/knurling-rs/defmt
//! [`defmt-print`]: https://crates.io/crates/defmt-print
//! [`defmt_embassy_usbserial::run`]: crate::task::run
//! [eusb-assert]: https://github.com/embassy-rs/embassy/blob/4bff7cea1a26267ec3671250e954d9d4242fabde/embassy-usb/src/builder.rs#L175-L177
//! [Embassy]: https://embassy.dev
//! [`embassy_rp`]: https://docs.embassy.dev/embassy-rp/git/rp2040/index.html

#![no_std]

mod controller;
mod task;

use core::{
    cell::UnsafeCell,
    sync::atomic::{AtomicBool, Ordering},
};

pub use task::{logger, run};

static USB_ENCODER: UsbEncoder = UsbEncoder::new();

struct UsbEncoder {
    /// A boolean lock
    ///
    /// Is `true` when `acquire` has been called and we have exclusive access to the
    /// rest of this struct.
    taken: AtomicBool,
    /// Critical section restore state
    ///
    /// Needed to exit a critical section.
    restore: UnsafeCell<critical_section::RestoreState>,
    /// A defmt Encoder for encoding frames
    encoder: UnsafeCell<defmt::Encoder>,
}

unsafe impl Sync for UsbEncoder {}

impl UsbEncoder {
    const fn new() -> Self {
        Self {
            taken: AtomicBool::new(false),
            restore: UnsafeCell::new(critical_section::RestoreState::invalid()),
            encoder: UnsafeCell::new(defmt::Encoder::new()),
        }
    }

    /// Acquire the defmt logger
    ///
    /// This acquires a critical section and begins a defmt frame.
    ///
    /// # Panics
    ///
    /// This will panic if you attempt to acquire the logger re-entrantly.
    fn acquire(&self) {
        // Get in a critical section.
        //
        // SAFETY: Must be paired with a call to release, as it is in the contract of
        // the Logger trait.
        let restore_state = unsafe { critical_section::acquire() };

        // Fail if the logger is acquired re-entrantly, to avoid two places with
        // mutable access to the logger state.
        if self.taken.load(Ordering::Relaxed) {
            panic!("defmt logger taken reentrantly");
        }

        // Set the boolean lock now that we're in a critical section and we know
        // it is not already taken.
        self.taken.store(true, Ordering::Relaxed);

        // SAFETY: Accessing the UnsafeCells is OK because we are in a critical section.
        unsafe {
            // Store the value needed to exit the critical section.
            self.restore.get().write(restore_state);

            // Start the defmt frame.
            let encoder = &mut *self.encoder.get();
            encoder.start_frame(Self::inner);
        }
    }

    /// Release the defmt logger
    ///
    /// This finishes the defmt frame and releases the critical section.
    ///
    /// # Safety
    ///
    /// Must be called exactly once after calling acquire.
    unsafe fn release(&self) {
        // Ensure we are not attempting to release while not in a critical section.
        if !self.taken.load(Ordering::Relaxed) {
            panic!("defmt release outside of critical section.")
        }

        // SAFETY: Accessing the UnsafeCells and finally releasing the critical section
        // is OK because we know we are in a critical section at this point.
        unsafe {
            let encoder = &mut *self.encoder.get();
            encoder.end_frame(Self::inner);

            let restore_state = self.restore.get().read();
            self.taken.store(false, Ordering::Relaxed);
            critical_section::release(restore_state);
        }
    }

    /// Flush the current buffer.
    ///
    /// # Safety
    ///
    /// Must be called after calling `acquire` and before calling `release`.
    unsafe fn flush(&self) {
        // Ideally this would block until everything has been written to the USB serial port.
        // However, this is not possible in a synchronous context, so we do nothing.
    }

    /// Write bytes to the defmt encoder.
    ///
    /// # Safety
    ///
    /// Must be called after calling `acquire` and before calling `release`.
    unsafe fn write(&self, bytes: &[u8]) {
        let encoder = unsafe { &mut *self.encoder.get() };
        encoder.write(bytes, Self::inner)
    }

    fn inner(bytes: &[u8]) {
        // SAFETY: Always called from within a critical section by the defmt logger.
        unsafe {
            controller::CONTROLLER.write(bytes);
        }
    }
}

/// The logger implementation.
#[defmt::global_logger]
struct USBLogger;

unsafe impl defmt::Logger for USBLogger {
    fn acquire() {
        USB_ENCODER.acquire();
    }

    unsafe fn release() {
        unsafe { USB_ENCODER.release() };
    }

    unsafe fn flush() {
        unsafe { USB_ENCODER.flush() };
    }

    unsafe fn write(bytes: &[u8]) {
        unsafe { USB_ENCODER.write(bytes) };
    }
}
