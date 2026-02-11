# defmtusb

`defmtusb` lets you read your [Embassy] firmware's [`defmt`] log messages over USB serial.

[`defmt`]: https://github.com/knurling-rs/defmt
[Embassy]: https://embassy.dev/

## Quickstart

This is the easiest method, for when you are not otherwise using the USB peripheral
in your firmware (to, for example, act as a keyboard).

Add `defmtusb` to your dependencies in your `Cargo.toml` file. In your firmware, create
an Embassy task that constructs your HAL-specific USB driver and an approriate USB
configuration. For example, using `embassy-rp` and with general firmware setup elided:

```rust
use embassy_rp::{bind_interrupts, Peri};

bind_interrupts!(struct Irqs {
    USBCTRL_IRQ => embassy_rp::usb::InterruptHandler<embassy_rp::peripherals::USB>;
});

#[embassy_executor::task]
async fn defmtusb_wrapper(usb: Peri<'static, embassy_rp::peripherals::USB>) {
    let driver = embassy_rp::usb::Driver::new(usb, Irqs);
    let usb_config = {
        let mut c = embassy_usb::Config::new(0x1234, 0x5678);
        c.serial_number = Some("defmt");
        c.max_packet_size_0 = 64;
        c.composite_with_iads = true;
        c.device_class = 0xEF;
        c.device_sub_class = 0x02;
        c.device_protocol = 0x01;
        c
    };
    defmt_embassy_usbserial::run(driver, usb_config).await;
}
```

In your main function, pass in the USB peripheral and spawn the task:

```rust
spawner.must_spawn(defmtusb_wrapper(peripherals.USB));
```

Now you can use the `defmt` logging macros as you'd expect.

```rust
loop {
    defmt::info!("Hello! {=u64:ts}", Instant::now().as_secs());
    delay.delay_ms(1000).await;
}
```

On the host side, use [`defmt-print`] to decode and print the messages.

[`defmt-print`]: https://crates.io/crates/defmt-print

## Complex USB setups

(Note: This section has not yet been updated since the fork from micro-rust/defmtusb.)

If you intend to create a variety of endpoints in the USB and use them, you can
create them and then simply pass a CDC ACM `Sender` and `ControlChanged` to the
`logger` task in `defmtusb`.


```rust
#[task]
async fn logger_wrapper(usb: USB) {
    // Create the USB driver.
    let driver = Driver::new(usb, Irqs);

    // Create the different interfaces and endpoints.
    ...

    // Create the CDC ACM class.
    let cdc = CdcAcmClass::new(&mut builder, &mut state, <max_packet_size>);

    // Split to get the sender only.
    let (sender, _rx, ctrl) = class.split_with_control();

    // Run only the logging function.
    defmtusb::logger(sender, ctrl).await;
}
```

## Contributing

Any contribution intentionally submitted for inclusion in the work by you shall
be licensed under either the MIT License or the Mozilla Public License Version
2.0, without any additional terms and conditions.

## License

This work is licensed, at your option, under the

 - [MIT License](/LICENSE-MIT)
 - [Mozilla Public License Version 2.0](/LICENSE-MPL)
 
