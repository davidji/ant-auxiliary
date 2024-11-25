//! CDC-ACM serial port example using cortex-m-rtic.
//! Target board: Blue Pill
#![no_main]
#![no_std]
#![allow(non_snake_case)]

use panic_semihosting as _;
use cortex_m::asm::delay;
use stm32f1xx_hal::{
    gpio::{gpioa::{self, PA0}, Alternate }, 
    pac,
    prelude::*,
    rcc,
    serial, 
    timer::{Ch, Channel, PwmHz, Tim2NoRemap }, 
    usb::{Peripheral, UsbBus, UsbBusType}
};
use usb_device::prelude::*;
use usbd_serial::embedded_io::{ReadReady, WriteReady};

type GrblTx = serial::Tx1;
type GrblRx = serial::Rx1;

#[rtic::app(device = stm32f1xx_hal::pac, dispatchers = [SPI3, UART4, UART5, TIM6, TIM7])]
mod app {
    use super::*;


    #[shared]
    struct Shared {
        usb_dev: UsbDevice<'static, UsbBusType>,
        usb_serial: usbd_serial::SerialPort<'static, UsbBusType>,
    }

    #[local]
    struct Local {
        grbl_tx: GrblTx, 
        grbl_rx: GrblRx,
        fan_pwm: PwmHz<pac::TIM2, Tim2NoRemap, Ch<0>, PA0<Alternate>>,
    }

    #[init(local=[usb_bus:  Option<usb_device::bus::UsbBusAllocator<UsbBusType>> = None])]
    fn init(cx: init::Context) -> (Shared, Local, init::Monotonics) {

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

        let usb_dev = UsbDeviceBuilder::new(
            usb_bus,
            UsbVidPid(0x16c0, 0x27dd),
        )
        .device_class(usbd_serial::USB_CLASS_CDC)
        .strings(&[StringDescriptors::default()
            .manufacturer("paraxial")
            .product("ant pcb maker")
            .serial_number("grbl")])
        .unwrap()
        .build();

        let (grbl_tx, grbl_rx) = grbl_serial(
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

        (Shared { 
            usb_dev, 
            usb_serial }, 
        Local {
            grbl_tx,
            grbl_rx,
            fan_pwm,
        }, init::Monotonics())
    }

    #[task(binds = USB_HP_CAN_TX)]
    fn usb_tx(_: usb_tx::Context) {
        usb::spawn().unwrap();
    }

    #[task(binds = USB_LP_CAN_RX0)]
    fn usb_rx0(_: usb_rx0::Context) {
        usb::spawn().unwrap();
    }

    #[task(shared = [usb_dev, usb_serial])]
    fn usb(cx: usb::Context) {
        let mut usb_dev = cx.shared.usb_dev;
        let mut serial = cx.shared.usb_serial;

        (&mut usb_dev, &mut serial).lock(|usb_dev, serial| {
            if usb_dev.poll(&mut [serial]) {
                if serial.read_ready().unwrap() {
                    grbl_serial_tx::spawn().unwrap();
                }

                if serial.write_ready().unwrap() {
                    grbl_serial_rx::spawn().unwrap();
                }                
            }
        });
    }

    #[task(binds = USART1)] 
    fn grbl_serial_interrupt(_: grbl_serial_interrupt::Context) {
        grbl_serial_rx::spawn().unwrap();
        grbl_serial_tx::spawn().unwrap();
    }

    #[task(local=[grbl_rx], shared = [usb_serial])]
    fn grbl_serial_rx(mut cx: grbl_serial_rx::Context) {
        let grbl_rx = cx.local.grbl_rx;
        if grbl_rx.is_rx_not_empty() {
            cx.shared.usb_serial.lock(|usb_serial| {
                while grbl_rx.is_rx_not_empty() && usb_serial.write_ready().unwrap() {
                    let buf = [grbl_rx.read().unwrap()];
                    usb_serial.write(&buf).unwrap();
                }
            })
        }
    }

    #[task(local=[grbl_tx], shared = [usb_serial])]
    fn grbl_serial_tx(mut cx: grbl_serial_tx::Context) {
        let grbl_tx: &mut serial::Tx<you_must_enable_the_rt_feature_for_the_pac_in_your_cargo_toml::USART1> = cx.local.grbl_tx;
        if grbl_tx.is_tx_empty() {
            cx.shared.usb_serial.lock(|usb_serial: &mut usbd_serial::SerialPort<'_, UsbBus<Peripheral>>| {
                while usb_serial.read_ready().unwrap() && grbl_tx.is_tx_empty() {
                    let mut buf = [0u8; 1];
                    usb_serial.read(&mut buf).unwrap();
                    grbl_tx.write(buf[0]).unwrap();
                }
            });
        }
    }

    #[task(local=[fan_pwm])]
    fn set_fan_speed(cx: set_fan_speed::Context) {

    }
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