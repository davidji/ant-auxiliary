//! CDC-ACM serial port example using cortex-m-rtic.
//! Target board: Blue Pill
#![no_main]
#![no_std]
#![allow(non_snake_case)]

use panic_semihosting as _;
use cortex_m::asm::delay;
use stm32f1xx_hal::{
    gpio::{gpioa::{self, PA0}, gpiob::PB12, Alternate, Output }, 
    pac,
    prelude::*,
    rcc,
    serial, 
    timer::{Ch, Channel, PwmHz, Tim2NoRemap }, 
    usb::{Peripheral, UsbBus, UsbBusType}
};
use usb_device::prelude::*;
use usbd_serial::embedded_io::ReadReady;
use rtic_sync::{
    channel::*,
    make_channel
};
use nb::Error::WouldBlock;
use rtic_monotonics::systick::prelude::*;
systick_monotonic!(Mono, 1_000);

type GrblTx = serial::Tx1;
type GrblRx = serial::Rx1;

const SERIAL_CHANNEL_CAPACITY: usize = 16;
type SerialChannelSender = Sender<'static, u8, SERIAL_CHANNEL_CAPACITY>;
type SerialChannelReceiver = Receiver<'static, u8, SERIAL_CHANNEL_CAPACITY>;

#[rtic::app(device = stm32f1xx_hal::pac, dispatchers = [SPI3, UART4, UART5, TIM6, TIM7])]
mod app {

    use super::*;

    #[shared]
    struct Shared {
        usb_dev: UsbDevice<'static, UsbBusType>,
        usb_serial: usbd_serial::SerialPort<'static, UsbBusType>,
        grbl_tx_sender: SerialChannelSender,
    }

    #[local]
    struct Local {
        grbl_tx: GrblTx, 
        grbl_rx: GrblRx,
        grbl_tx_receiver: SerialChannelReceiver,
        grbl_rx_sender: SerialChannelSender,
        grbl_rx_receiver: SerialChannelReceiver,

        fan_pwm: PwmHz<pac::TIM2, Tim2NoRemap, Ch<0>, PA0<Alternate>>,
        led: PB12<Output>,
    }

    #[init(local=[usb_bus:  Option<usb_device::bus::UsbBusAllocator<UsbBusType>> = None])]
    fn init(cx: init::Context) -> (Shared, Local) {

        let peripherals = cx.device;
        let mut flash = peripherals.FLASH.constrain();
        let rcc = peripherals.RCC.constrain();
      

        let clocks: rcc::Clocks = rcc
            .cfgr
            .use_hse(8.MHz())
            .sysclk(48.MHz())
            .pclk1(24.MHz())
            .freeze(&mut flash.acr);

        assert!(clocks.usbclk_valid());

        let mut gpioa = peripherals.GPIOA.split();

        // BluePill board has a pull-up resistor on the D+ line.
        // Pull the D+ pin down to send a RESET condition to the USB bus.
        // This forced reset is needed only for development, without it host
        // will not reset your device when you upload new firmware.
        let mut usb_dp = gpioa.pa12.into_push_pull_output(&mut gpioa.crh);
        usb_dp.set_low();
        delay(clocks.sysclk().raw() / 100);

        
        cx.local.usb_bus.replace(UsbBus::new(Peripheral {
            usb: peripherals.USB,
            pin_dm: gpioa.pa11,
            pin_dp: usb_dp.into_floating_input(&mut gpioa.crh),
        }));
    
        
        let usb_bus = cx.local.usb_bus.as_ref().unwrap();
        let usb_serial = usbd_serial::SerialPort::new(usb_bus);

        let (grbl_tx, mut grbl_rx) = grbl_serial(
            peripherals.USART1, 
            gpioa.pa9.into_alternate_push_pull(&mut gpioa.crh), 
            gpioa.pa10, 
            clocks);

        let mut afio = peripherals.AFIO.constrain();
        // TIM2
        let fan_pwm_pin = gpioa.pa0.into_alternate_push_pull(&mut gpioa.crl);
        let mut fan_pwm = peripherals
            .TIM2
            .pwm_hz::<Tim2NoRemap, _, _>(fan_pwm_pin, &mut afio.mapr, 25.kHz(), &clocks);
        fan_pwm.enable(Channel::C1);

        let (grbl_tx_sender,grbl_tx_receiver) = make_channel!(u8, SERIAL_CHANNEL_CAPACITY);
        let (grbl_rx_sender, grbl_rx_receiver) = make_channel!(u8, SERIAL_CHANNEL_CAPACITY);
        
        let mut gpiob = peripherals.GPIOB.split();

        Mono::start(cx.core.SYST, 72_000_000);

        blink::spawn().unwrap();
        grbl_serial_tx::spawn().unwrap();
        usb_tx::spawn().unwrap();
        grbl_rx.listen();

        (Shared {
            usb_dev : grbl_usb_device(usb_bus), 
            usb_serial,
            grbl_tx_sender,
         }, 
        Local {
            grbl_tx,
            grbl_rx,
            grbl_tx_receiver,
            grbl_rx_sender,
            grbl_rx_receiver,
            fan_pwm,
            led: gpiob.pb12.into_push_pull_output(&mut gpiob.crh)
        })
    }

    #[task(local = [ led ])]
    async fn blink(cx: blink::Context) {
        loop {
            Mono::delay(1000.millis()).await;
            cx.local.led.toggle();
        }
    }

    #[task(binds = USB_HP_CAN_TX, shared = [usb_dev, usb_serial, grbl_tx_sender])]
    fn usb_hp(cx: usb_hp::Context) {
        // There's a lot of duplicated code in the two interrupt handlers
        // but we _must_ poll the usb device in the interrupt
        // handler, in order to clear the interrupt. If you could somehow
        // unbundle that, things would be a lot better.
        // It's also not completely obvious this is safe: I don't think the
        // lock is un-interruptable, so maybe another interrupt starts
        // while the local is still held, and we end up in a deadlock.
        // This doesn't seem to ever happen though.
        let dev = cx.shared.usb_dev;
        let serial = cx.shared.usb_serial;
        let sender = cx.shared.grbl_tx_sender;
        (dev, serial, sender).lock(|dev, serial, sender| {
            if dev.poll(&mut [serial]) {
                usb_serial_io(serial, sender);
            }
        });
    }

    #[task(binds = USB_LP_CAN_RX0, shared = [usb_dev, usb_serial, grbl_tx_sender])]
    fn usb_lp(cx: usb_lp::Context) {
        let dev = cx.shared.usb_dev;
        let serial = cx.shared.usb_serial;
        let sender = cx.shared.grbl_tx_sender;
        (dev, serial, sender).lock(|dev, serial, sender| {
            if dev.poll(&mut [serial]) {
                usb_serial_io(serial, sender);
            }
        });
    }

    #[task(shared = [usb_dev, usb_serial], local = [grbl_rx_receiver])]
    async fn usb_tx(cx: usb_tx::Context) {
        let mut shared = (cx.shared.usb_dev, cx.shared.usb_serial);
        loop {
            match cx.local.grbl_rx_receiver.recv().await {
                Ok(data) => 
                    while ! shared.lock(|dev, serial| {
                        match serial.write(&[data]) {
                            Ok(_) => true,
                            Err(UsbError::WouldBlock) => { dev.poll(&mut [serial]); false },
                            Err(_) => panic!("Error writing to GRBL serial")
                        }
                    }) {
                        Mono::delay(5.millis()).await;
                    },
                Err(_) => Mono::delay(5.millis()).await
            }
        }
    }

    #[task(binds = USART1, local=[grbl_rx, grbl_rx_sender])] 
    fn grbl_serial_interrupt(cx: grbl_serial_interrupt::Context) {
        let grbl_rx = cx.local.grbl_rx;
        while grbl_rx.is_rx_not_empty() {
            match grbl_rx.read() {
                Ok(data) => cx.local.grbl_rx_sender.try_send(data).unwrap(),
                Err(_) => panic!("Error reading from GRBL serial")
            }
        }
        
    }

    #[task(local = [ grbl_tx, grbl_tx_receiver])]
    async fn grbl_serial_tx(cx: grbl_serial_tx::Context) {
        let grbl_tx = cx.local.grbl_tx;
        loop {
            match cx.local.grbl_tx_receiver.recv().await {
                Ok(data) => loop {
                    match grbl_tx.write(data) {
                        Ok(_) => break,
                        Err(WouldBlock) => Mono::delay(5.millis()).await,
                        Err(_) => panic!("Error writing to GRBL serial")
                    }
                },
                Err(_) => Mono::delay(5.millis()).await
            };
        }
    }
 
    #[task(local=[fan_pwm])]
    async fn set_fan_speed(cx: set_fan_speed::Context, duty: u16) {
        cx.local.fan_pwm.set_duty(Channel::C1, duty);
    }
}

fn grbl_usb_device(usb_bus: &usb_device::bus::UsbBusAllocator<UsbBus<Peripheral>>) -> UsbDevice<'_, UsbBus<Peripheral>> {
    UsbDeviceBuilder::new(
        usb_bus,
        UsbVidPid(0x16c0, 0x27dd),
    )
    .device_class(usbd_serial::USB_CLASS_CDC)
    .strings(&[StringDescriptors::default()
        .manufacturer("paraxial")
        .product("ant pcb maker")
        .serial_number("grbl")])
    .unwrap()
    .build()
}

fn grbl_serial(
    usart: pac::USART1,
    tx: gpioa::PA9<Alternate>,
    rx: gpioa::PA10,
    clocks: rcc::Clocks) -> (GrblTx, GrblRx) {
      // Create an interface struct for USART1 with 115200 Baud
      let grbl_serial: serial::Serial<pac::USART1> = serial::Serial::new(
          usart,
          (tx, rx),
          serial::Config::default()
              .baudrate(115200.bps())
              .wordlength_8bits()
              .parity_none()
              .stopbits(serial::StopBits::STOP1),
          &clocks);

    return grbl_serial.split();
}

fn usb_serial_io(
    serial: &mut usbd_serial::SerialPort<'static, UsbBusType>, 
    sender: &mut SerialChannelSender) {
        
    while serial.read_ready().unwrap() && !sender.is_full() {
         let mut buf = [0u8; 1];
         match serial.read(&mut buf) {
            Ok(_) => sender.try_send(buf[0]).unwrap(),
            Err(UsbError::WouldBlock) => break,
            Err(_) => panic!("Error writing to USB serial"),
        };
    }

}