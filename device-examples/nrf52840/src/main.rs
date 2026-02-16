#![no_std]
#![no_main]

use defmt::info;
use embassy_executor::Spawner;
use embedded_hal_async::delay::DelayNs;
use embassy_nrf::peripherals::USBD;
use embassy_nrf::usb::Driver;
use embassy_nrf::usb::vbus_detect::HardwareVbusDetect;
use embassy_nrf::{Peri, bind_interrupts, peripherals, usb};
use embassy_time::Instant;
use embassy_usb::Config;

use defmt as _;
use panic_halt as _;

bind_interrupts!(struct Irqs {
    USBD => usb::InterruptHandler<peripherals::USBD>;
    CLOCK_POWER => usb::vbus_detect::InterruptHandler;
});

#[embassy_executor::task]
async fn defmtusb_wrapper(usbd: Peri<'static, USBD>){
    let driver = Driver::new(usbd, Irqs, HardwareVbusDetect::new(Irqs));

    let mut config = Config::new(0x16c0, 0x27dd);
    config.serial_number = Some("defmt");
    config.max_packet_size_0 = 64;
    config.composite_with_iads = true;
    config.device_class = 0xEF;
    config.device_sub_class = 0x02;
    config.device_protocol = 0x01;

    defmt_embassy_usbserial::run(driver, config).await;
}

#[embassy_executor::main]
async fn main(spawner: Spawner) {
    let mut delay = embassy_time::Delay;
    let config = embassy_nrf::config::Config::default();
    let p = embassy_nrf::init(config);
    spawner.must_spawn(defmtusb_wrapper(p.USBD));

    info!("Starting loop");

    loop {
        info!("Hello, world!  {=u64:tms}", Instant::now().as_millis());
        delay.delay_ms(100).await;
    }
}
