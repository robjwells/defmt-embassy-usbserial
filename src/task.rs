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
    embassy_futures::join::join(usb.run(), logger(sender)).await;
}

/// Runs the logger task.
pub async fn logger<'d, D: Driver<'d>>(mut sender: Sender<'d, D>) {
    use embassy_time::{Duration, Timer};

    use embassy_usb::driver::EndpointError;

    // Get a reference to the controller.
    let controller = &super::controller::CONTROLLER;
    // Only attempt to write what the sender will accept.
    let packet_size = sender.max_packet_size() as usize;

    'main: loop {
        // Wait for the device to be connected.
        sender.wait_connection().await;

        // Set the controller as enabled.
        controller.enable();

        // Continually attempt to write buffered defmt bytes out over USB.
        loop {
            let flush_res = controller
                .flush::<_, EndpointError>(async |bytes| {
                    for chunk in bytes.chunks(packet_size) {
                        sender.write_packet(chunk).await?;
                    }
                    Ok(())
                })
                .await;

            match flush_res {
                Err(EndpointError::Disabled) => {
                    // USB endpoint is now disabled, so disable the controller (and so
                    // not accept any defmt log messages) and wait until reconnected.
                    controller.disable();
                    continue 'main;
                }
                Err(EndpointError::BufferOverflow) => {
                    unreachable!("Sent chunks are limited to Sender max packet size.")
                }
                Ok(()) => (),
            };

            // Wait the timeout.
            // TODO: Make this configurable.
            Timer::after(Duration::from_millis(100)).await;
        }
    }
}
