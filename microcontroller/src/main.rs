//! CDC-ACM serial port example using cortex-m-rtic.
//! Target board: Blue Pill
#![no_main]
#![no_std]
#![allow(non_snake_case)]

mod rpc;

use panic_semihosting as _;
use cortex_m::asm::delay;
use stm32f4xx_hal::{
    gpio::{ gpioa::{self, PA1}, Output }, 
    pac::{ self, TIM2 },
    prelude::*,
    rcc,
    serial, 
    timer::PwmChannel, 
    otg_fs::{UsbBus, UsbBusType, USB}
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

const SERIAL_CHANNEL_CAPACITY: usize = 128;
type SerialChannelSender = Sender<'static, u8, SERIAL_CHANNEL_CAPACITY>;
type SerialChannelReceiver = Receiver<'static, u8, SERIAL_CHANNEL_CAPACITY>;


#[rtic::app(device = stm32f4xx_hal::pac, dispatchers = [SPI2, SPI3])]
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
        fan_pwm: PwmChannel<TIM2, 0>,
        led: PA1<Output>,
    }

    #[init(local=[usb_bus: Option<usb_device::bus::UsbBusAllocator<UsbBusType>> = None,
        ep_memory: [u32; 1024] = [0; 1024]])]
    fn init(cx: init::Context) -> (Shared, Local) {

        let peripherals = cx.device;
        let rcc = peripherals.RCC.constrain();
      

        let clocks: rcc::Clocks = rcc
            .cfgr
            .use_hse(25.MHz())
            .sysclk(84.MHz())
            .require_pll48clk()
            .freeze();

        let gpioa = peripherals.GPIOA.split();

        // BluePill board has a pull-up resistor on the D+ line.
        // Pull the D+ pin down to send a RESET condition to the USB bus.
        // This forced reset is needed only for development, without it host
        // will not reset your device when you upload new firmware.
        let mut usb_dp = gpioa.pa12.into_push_pull_output();
        usb_dp.set_low();
        delay(clocks.sysclk().raw() / 100);

        cx.local.usb_bus.replace(UsbBus::new(USB {
            usb_global: peripherals.OTG_FS_GLOBAL,
            usb_device: peripherals.OTG_FS_DEVICE,
            usb_pwrclk: peripherals.OTG_FS_PWRCLK,
            pin_dm: gpioa.pa11.into(),
            pin_dp: usb_dp.into(),
            hclk: clocks.hclk(),
        }, cx.local.ep_memory));
    
        
        let usb_bus = cx.local.usb_bus.as_ref().unwrap();
    
        let usb_serial = usbd_serial::SerialPort::new(usb_bus);

        let (grbl_tx, mut grbl_rx) = grbl_serial(
            peripherals.USART1, 
            gpioa.pa9.into(),
            gpioa.pa10, 
            clocks);


        // TIM2
        let (_, (fan_pwm, ..)) = peripherals
            .TIM2.pwm_hz(25.kHz(), &clocks);
        let mut fan_pwm = fan_pwm.with(gpioa.pa0);
        fan_pwm.enable();

        let (grbl_tx_sender,grbl_tx_receiver) = make_channel!(u8, SERIAL_CHANNEL_CAPACITY);
        let (grbl_rx_sender, grbl_rx_receiver) = make_channel!(u8, SERIAL_CHANNEL_CAPACITY);

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
            led: gpioa.pa1.into_push_pull_output()
        })
    }

    #[task(local = [ led ])]
    async fn blink(cx: blink::Context) {
        loop {
            Mono::delay(1000.millis()).await;
            cx.local.led.toggle();
        }
    }

    #[task(binds = OTG_FS, shared = [usb_dev, usb_serial, grbl_tx_sender])]
    fn usb_hp(cx: usb_hp::Context) {
        let mut shared = (
            cx.shared.usb_dev, 
            cx.shared.usb_serial,
            cx.shared.grbl_tx_sender);
        shared.lock(|dev, serial, sender| {
            if dev.poll(&mut [serial]) {
                usb_serial_read(serial, sender);
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
        cx.local.fan_pwm.set_duty(duty);
    }
}

fn grbl_usb_device(usb_bus: &usb_device::bus::UsbBusAllocator<UsbBus<USB>>) -> UsbDevice<'_, UsbBus<USB>> {
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
    tx: gpioa::PA9,
    rx: gpioa::PA10,
    clocks: rcc::Clocks) -> (GrblTx, GrblRx) {
      // Create an interface struct for USART1 with 115200 Baud
      let grbl_serial: serial::Serial<pac::USART1> = serial::Serial::new(
          usart,
          (tx, rx),
          serial::Config::default()
              .baudrate(115200.bps())
              .parity_none(),
          &clocks).unwrap();

    return grbl_serial.split();
}

fn usb_serial_read(
    serial: &mut usbd_serial::SerialPort<'static, UsbBusType>, 
    sender: &mut SerialChannelSender) {

    let mut buf = [0u8; 1];
    while serial.read_ready().unwrap() && !sender.is_full() {
        match serial.read(&mut buf) {
            Ok(_) => sender.try_send(buf[0]).unwrap(),
            Err(_) => panic!("Error writing to USB serial"),
        };
    }

}
