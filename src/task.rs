//! Main task that runs the USB transport layer.

use embassy_time::{Duration, Timer};
use embassy_usb::{
    Builder, Config,
    class::cdc_acm::{CdcAcmClass, ControlChanged, Sender, State},
    driver::{Driver, EndpointError},
};

use static_cell::{ConstStaticCell, StaticCell};

// TODO: Document the RAM usage of these buffers.

/// Config descriptor buffer
static CONFIG_DESCRIPTOR_BUF: ConstStaticCell<[u8; 256]> = ConstStaticCell::new([0u8; 256]);

/// BOS descriptor buffer
static BOS_DESCRIPTOR_BUF: ConstStaticCell<[u8; 256]> = ConstStaticCell::new([0u8; 256]);

/// MSOS descriptor buffer
static MSOS_DESCRIPTOR_BUF: ConstStaticCell<[u8; 256]> = ConstStaticCell::new([0u8; 256]);

/// Control buffer
static CONTROL_BUF: ConstStaticCell<[u8; 256]> = ConstStaticCell::new([0u8; 256]);

/// CDC ACM state.
static STATE: StaticCell<State> = StaticCell::new();

/// Run the USB driver and defmt logger tasks.
///
/// This function builds the USB device with the provided driver and configuration, and awaits both
/// it and the function that writes out buffered defmt messages over USB.
///
/// Along with the usb driver implementation, users must pass a USB configuration that is properly
/// set for USB-CDC. See [the library documentation][crate] for details about the requirements.
pub async fn run<D: Driver<'static>>(driver: D, config: Config<'static>) {
    // Create the state of the CDC ACM device.
    let state: &'static mut State<'static> = STATE.init(State::new());

    // Create the USB builder.
    let mut builder = Builder::new(
        driver,
        config,
        CONFIG_DESCRIPTOR_BUF.take(),
        BOS_DESCRIPTOR_BUF.take(),
        MSOS_DESCRIPTOR_BUF.take(),
        CONTROL_BUF.take(),
    );

    // Create the class on top of the builder.
    let packet_size = config.max_packet_size_0 as u16;
    let class = CdcAcmClass::new(&mut builder, state, packet_size);

    // Build the USB.
    let mut usb = builder.build();

    // Get the sender.
    let (sender, _, ctrl) = class.split_with_control();

    // Run both futures concurrently.
    embassy_futures::join::join(usb.run(), logger(sender, ctrl)).await;
}

/// USB logger task that writes messages out over USB.
pub async fn logger<'d, D: Driver<'d>>(mut sender: Sender<'d, D>, ctrl: ControlChanged<'d>) {
    // Get a reference to the controller.
    let mut consumer = super::controller::RING_BUFFER.consumer();

    'main: loop {
        // Wait for the device to be connected.
        sender.wait_connection().await;

        // If we don't wait for both DTR and RTS before sending data, we may send data before the
        // host is ready to receive it, which will cause the host to drop the data.
        // Continually attempt to write buffered defmt bytes out over USB.
        loop {
            while !(sender.dtr() && sender.rts()) {
                ctrl.control_changed().await;
                Timer::after(Duration::from_millis(10)).await;
            }

            // Wait for data to be available.
            let readable = consumer.readable_bytes().await;
            use embedded_io_async::Write;
            match sender.write(&readable).await {
                Err(EndpointError::Disabled) => {
                    // USB endpoint is now disabled. Wait for reconnection and
                    // hope we're using rzcobs encoding.
                    continue 'main;
                }
                Err(EndpointError::BufferOverflow) => {
                    unreachable!("Sent chunks are limited to Sender max packet size.")
                }
                Ok(bytes_written) => {
                    // Mark the bytes as consumed.
                    readable.consume(bytes_written);
                }
            }
        }
    }
}
