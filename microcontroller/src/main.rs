
#![no_main]
#![no_std]
#[allow(non_snake_case)]

mod dht11;
mod fan;
mod frequency;
mod light;
mod network;
mod seed;
mod shell;
mod serial;
mod statistics;

pub mod proto {
    #![allow(clippy::all)]
    #![allow(nonstandard_style, unused, irrefutable_let_patterns)]
    include!(concat!(env!("OUT_DIR"), "/aux.rs"));
}

use network::{ NetworkStack, NetworkChannelStorage, RecvChannel };

use panic_probe as _;
use defmt_rtt as _;

use cortex_m::asm::delay;
use stm32f4xx_hal::{
    gpio::{
        gpioa::{ PA1, PA2 }, 
        gpioc::{ 
            PC2, // used for with PA2, to clear EXTI2 interrupts */ 
            PC13, // led 
        },
        Input,
        Output, 
        OpenDrain,
    }, 
    prelude::*,
    pac::{ TIM3 },
    rcc,
    otg_fs::{UsbBus, UsbBusType, USB}
};

use usb_device::prelude::*;
use rtic_monotonics::{
    fugit, 
    Monotonic, 
    stm32_tim2_monotonic 
};

use smoltcp::{
    iface::{ SocketStorage },
};


const MONO_RATE: u32 = 1_000_000;
stm32_tim2_monotonic!(Mono, MONO_RATE);
impl frequency::Proportion<u32> for u32 {}
type Duration = fugit::Duration<u64, 1, MONO_RATE>;
type Instant = fugit::Instant<u64, 1, MONO_RATE>;
impl frequency::Value<Duration, u32> for Duration {} 

const CHANNEL_CAPACITY: usize = 2*network::MTU as usize;
const CHANNELS: usize = 2;
const SOCKETS: usize = CHANNELS + 1; // +1 for the dhcp socket

#[rtic::app(device = stm32f4xx_hal::pac, dispatchers = [ EXTI4, EXTI9_5, EXTI15_10 ])]
mod app {

    use proto::{ 
        FanRequest,
        Request, 
        Request_::Peripheral as RequestPeripheral, 
        Response, 
        Response_::Peripheral as ResponsePeripheral, 
    };
    use rtic_sync::{
        make_channel, 
        make_signal,
    };

    use stm32f4xx_hal::{adc::{config::AdcConfig, Adc}, timer::PwmChannel };

    use crate::{ 
        frequency::{ Bounds, Frequency, Ratio }, 
        network::SendChannel, proto::{LightRequest, TempRequest, TempResponse}, shell::TaskResponses,
    };

    use defmt::{ debug, warn };

    use super::*;

  
    impl network::IntoInstant for Instant {
        fn into_instant(self) -> smoltcp::time::Instant {
            let time = self.duration_since_epoch().to_micros();
            smoltcp::time::Instant::from_micros(time as i64)
        }
    }
    
    #[shared]
    struct Shared {
        usb: UsbDevice<'static, UsbBusType>,
        network: NetworkStack<'static, Mono>,
        temp: Option<TempResponse>,
    }

    #[local]
    struct Local {
        grbl_tx: serial::TxTask<'static>, 
        grbl_rx: serial::RxTask<'static>,
        network_recv: [RecvChannel<'static, CHANNEL_CAPACITY>; CHANNELS],
        requests: shell::CommandRequests<'static, CHANNEL_CAPACITY>,
        responses: shell::CommandResponses<'static, CHANNEL_CAPACITY>,
        network_send: [SendChannel<'static, CHANNEL_CAPACITY>; CHANNELS],
        fan: fan::Fan<'static, PwmChannel<TIM3, 0>>,
        fan_freq: Frequency<'static, PA1<Input>, Mono, u32>,
        light: light::Light<PwmChannel<TIM3, 1>>,
        led: PC13<Output>,
        temp_reader: dht11::Dht11Reader<'static, PA2<Output<OpenDrain>>>,
        temp_writer: dht11::Dht11Writer<'static, PC2>,
        temp_responses: TaskResponses<()>,
    }
    


    #[init(local=[
        usb_bus: Option<usb_device::bus::UsbBusAllocator<UsbBusType>> = None,
        ep_memory: [u32; 4096] = [0; 4096],
        grbl_channel_storage: NetworkChannelStorage<CHANNEL_CAPACITY> = NetworkChannelStorage::new(),
        shell_channel_storage: NetworkChannelStorage<CHANNEL_CAPACITY> = NetworkChannelStorage::new(),
        ethernet_in_buffer: [u8; 2048] = [0; 2048],
        ethernet_out_buffer: [u8; 2048] = [0; 2048],
        socket_storage: [SocketStorage<'static>; SOCKETS] = [SocketStorage::EMPTY; SOCKETS]])]
    fn init(cx: init::Context) -> (Shared, Local) {

        let mut peripherals = cx.device;
        let rcc = peripherals.RCC.constrain();
      

        let clocks: rcc::Clocks = rcc
            .cfgr
            .use_hse(25.MHz())
            .sysclk(100.MHz())
            .require_pll48clk()
            .freeze();

        Mono::start(100_000_000);

        let gpioa = peripherals.GPIOA.split();
        let gpioc = peripherals.GPIOC.split();

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
    
        let usb_ethernet: usbd_ethernet::Ethernet<'_, UsbBus<USB>> = network::usb_ethernet(
            usb_bus, 
            cx.local.ethernet_in_buffer, 
            cx.local.ethernet_out_buffer);

        let mut network = NetworkStack::new(
            usb_ethernet, 
            &mut cx.local.socket_storage[..],
            seed::seed(
                &mut Adc::adc1(peripherals.ADC1, true, AdcConfig::default()), 
                &mut gpioa.pa3.into_analog()));
    
        let grbl = serial::Tasks::new(
            peripherals.USART1, 
            gpioa.pa9.into(),
            gpioa.pa10, 
            clocks,
            network.channel(1337, cx.local.grbl_channel_storage));

        let shell_channel = network.channel(1338, cx.local.shell_channel_storage);

        let (fan_freq_writer, fan_freq_reader) = make_signal!(Duration);

        let (_, (fan_pwm, light_pwm, ..)) = peripherals.TIM3.pwm_hz(25.kHz(), &clocks);
        let mut fan_pwm = fan_pwm.with(gpioa.pa6);
        let mut light_pwm = light_pwm.with(gpioa.pa7);
        fan_pwm.enable();
        light_pwm.enable();

        let mut syscfg = peripherals.SYSCFG.constrain();
        let fan_freq = Frequency::new(
            gpioa.pa1.into_pull_up_input(), 
            Ratio(9,1), 
            Bounds(Duration::micros(10), Duration::secs(1)), 
            &mut syscfg, 
            &mut peripherals.EXTI,
            fan_freq_writer,
        );

        blink::spawn().unwrap();
        grbl_serial_tx::spawn().unwrap();
        usb_send::spawn().unwrap();
        responses::spawn().unwrap();
        requests::spawn().unwrap();
        temp::spawn().unwrap();

        let (response_sender, 
             response_receiver) = make_channel!(
                shell::Message, 
                { shell::MESSAGE_CAPACITY });

        let (temp_writer, temp_reader) = dht11::make(
            gpioa.pa2.into_open_drain_output(),
            gpioc.pc2,
            &mut syscfg, &mut peripherals.EXTI);

        (Shared {
            usb : usb_device(usb_bus),
            network,
            temp: Option::None,
         }, 
         Local {
            grbl_tx: grbl.tx,
            grbl_rx: grbl.rx,
            network_recv: [grbl.net.recv, shell_channel.net.recv],
            network_send : [grbl.net.send, shell_channel.net.send],
            requests: shell::CommandRequests {
                requests: shell_channel.app.recv,
                responses: response_sender.clone()
            },
            responses: shell::CommandResponses {
                sender: shell_channel.app.send,
                messages: response_receiver,
            },
            fan: fan::Fan::new(fan_pwm, response_sender.clone(), fan_freq_reader),
            light: light::Light::new(light_pwm, response_sender.clone()),
            fan_freq,
            led: gpioc.pc13.into_push_pull_output(),
            temp_reader,
            temp_writer,
            temp_responses: shell::TaskResponses { 
                    task: (), 
                    responses: response_sender.clone()
                },
        })
    }

    #[task(local = [ led ])]
    async fn blink(cx: blink::Context) {
        loop {
            Mono::delay(1000.millis().into()).await;
            cx.local.led.toggle();
        }
    }

    #[task(binds = OTG_FS, shared = [ usb, network ], local = [ network_recv ], priority=2)]
    fn usb_recv(cx: usb_recv::Context) {
        let channels = cx.local.network_recv;
        let mut shared = (cx.shared.usb, cx.shared.network);

        shared.lock(|usb, network| {
            if usb.poll(&mut [&mut network.ethernet]) {
                network.try_recv(channels);
            }
        });
    }

    #[task(shared = [ usb, network ], local = [ network_send ], priority=2)]
    async fn usb_send(cx: usb_send::Context) {
        let channels = cx.local.network_send;
        let mut shared = (cx.shared.usb, cx.shared.network);

        loop {
            shared.lock(|usb, network| {
                usb.poll(&mut [&mut network.ethernet]);
                network.try_send(channels);
            });

            Mono::delay(500.micros().into()).await;
        }
    }


    #[task(binds = USART1, local=[ grbl_rx ], priority=2)] 
    fn grbl_serial_interrupt(cx: grbl_serial_interrupt::Context) {
        let grbl_rx = cx.local.grbl_rx;
        grbl_rx.receive();
    }

    #[task(local = [ grbl_tx])]
    async fn grbl_serial_tx(cx: grbl_serial_tx::Context) {
        let grbl_tx = cx.local.grbl_tx;
        loop {
            grbl_tx.send().await;
        }
    }

    #[task(local = [requests])]
    async fn requests(cx: requests::Context) {
        loop {
            match cx.local.requests.receive().await {
                Request { peripheral: Some(RequestPeripheral::Fan(request)) } => {
                    let _ = fan_request::spawn(request);
                },
                Request { peripheral: Some(RequestPeripheral::Temp(request)) } => {
                    let _  = temp_request::spawn(request);
                },
                Request { peripheral: Some(RequestPeripheral::Light(request)) } => {
                    let _ = light_request::spawn(request);
                },
                Request { peripheral: None } => {
                    warn!("No peripheral specified")
                }
            };
        }
    }

    #[task(local = [responses])]
    async fn responses(cx: responses::Context) {
        debug!("response relay starting");
        cx.local.responses.send().await;
        debug!("response relay exit");
    }


    #[task(local=[temp_reader], shared=[temp])]
    async fn temp(mut cx: temp::Context) {
        let temp = cx.local.temp_reader;
        Mono::delay(Duration::secs(1)).await;
        loop {
            let start = Mono::now();
            match temp.read().await {
                Ok(response) => {
                    cx.shared.temp.lock(|current| {
                        current.replace(response)
                    });
                },
                Err(err) => {
                    warn!("Error reading temperature {}", err);
                }
            }
            Mono::delay_until(start + Duration::secs(1)).await;
        }
    }

    #[task(local = [ temp_responses], shared = [temp])]
    async fn temp_request(mut cx: temp_request::Context, _: TempRequest) {
        let temp = cx.local.temp_responses;
        let current = cx.shared.temp.lock(|current| current.clone());
        temp.responses.send(Response {
            peripheral: Some(ResponsePeripheral::Temp(current.unwrap_or(TempResponse::default())))
        }).await.unwrap();
    }


    #[task(binds=EXTI2, local=[temp_writer], priority=4)]
    fn temp_falling_edge(cx: temp_falling_edge::Context) {
        cx.local.temp_writer.falling_edge();
    }

    #[task(local=[fan])]
    async fn fan_request(cx: fan_request::Context, request: FanRequest) {
        cx.local.fan.process(request).await;
    }

    #[task(binds = EXTI1, local = [fan_freq])]
    fn fan_freq_edge(cx: fan_freq_edge::Context) {
        cx.local.fan_freq.edge();
    }

    #[task(local = [light])]
    async fn light_request(cx: light_request::Context, request: LightRequest) {
        cx.local.light.process(request).await;       
    }
}

fn usb_device(usb_bus: &usb_device::bus::UsbBusAllocator<UsbBus<USB>>) -> UsbDevice<'_, UsbBus<USB>> {
    UsbDeviceBuilder::new(
        usb_bus,
        UsbVidPid(0x16c0, 0x27dd),
    )
    .device_class(usbd_ethernet::USB_CLASS_CDC)
    .strings(&[StringDescriptors::default()
        .manufacturer("paraxial")
        .product("pcb-mill")
        .serial_number("aux")])
    .unwrap()
    .build()
}


