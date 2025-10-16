//! Main task that runs the USB transport layer.

use embassy_usb::{
    class::cdc_acm::{Sender, State},
    driver::Driver,
    Config,
};

use static_cell::{ConstStaticCell, StaticCell};

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

/// Builds the USB class and runs both the logger and USB.
/// Requires the USB driver provided by the HAL and the maximum packet size
/// allowed in the device.
/// The user may provide an optional USB configuration to set the VID, PID and
/// other information of the USB device. If none is provided a default
/// configuration will be set.
pub async fn run<D: Driver<'static>>(driver: D, size: usize, config: Config<'static>) {
    use embassy_usb::{class::cdc_acm::CdcAcmClass, Builder};

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
    let class = CdcAcmClass::new(&mut builder, state, size as u16);

    // Build the USB.
    let mut usb = builder.build();

    // Get the sender.
    let (sender, _) = class.split();

    // Run both futures concurrently.
    embassy_futures::join::join(usb.run(), logger(sender, size)).await;
}

/// Runs the logger task.
#[allow(unused_labels)]
pub async fn logger<'d, D: Driver<'d>>(mut sender: Sender<'d, D>, size: usize) {
    use embassy_time::{Duration, Timer};

    use embassy_usb::driver::EndpointError;

    // Get a reference to the controller.
    let controller = &super::controller::CONTROLLER;

    'main: loop {
        // Wait for the device to be connected.
        sender.wait_connection().await;

        // Set the controller as enabled.
        controller.enable();

        // Begin sending the data.
        'data: loop {
            // Wait for new data.
            let (buf_idx, buffer) = 'select: loop {
                // Get a flushing buffer
                if let Some(pair) = controller.get_flushing() {
                    break pair;
                }
                // Wait the timeout.
                // TODO : Make this configurable.
                Timer::after(Duration::from_millis(100)).await;
            };

            // Get an iterator over the data of the buffer.
            let chunks = buffer.data[..buffer.cursor].chunks(size);

            for chunk in chunks {
                // Send the data.
                if let Err(EndpointError::Disabled) = sender.write_packet(chunk).await {
                    // Reset the buffer as its contents' integrity is gone.
                    // TODO: Why was there no actual reset of the buffer?

                    // Disable the controller.
                    controller.disable();

                    continue 'main;
                }
            }

            // Reset the buffer as it has been transmitted.
            controller.reset_buffer(buf_idx);
        }
    }
}
