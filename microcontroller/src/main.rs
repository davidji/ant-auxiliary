//! CDC-ACM serial port example using cortex-m-rtic.
//! Target board: Blue Pill
#![no_main]
#![no_std]
#![allow(non_snake_case)]

mod frequency;
mod network;
mod shell;
mod serial;
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
        gpioa::PA1, 
        gpioc::{ PC13, PC14 },
        Input,
        Output, 
        OpenDrain,
    }, 
    pac::{ TIM2 },
    prelude::*,
    rcc,
    timer::PwmChannel, 
    otg_fs::{UsbBus, UsbBusType, USB}
};

use usb_device::prelude::*;
use rtic_monotonics::systick::prelude::*;

use smoltcp::{
    iface::{ SocketSet, SocketStorage},
    wire::IpAddress,
};


systick_monotonic!(Mono, 1_000);

const CHANNEL_CAPACITY: usize = network::MTU as usize;

const SOCKET_ADDRESS: (IpAddress, u16) = (IpAddress::Ipv4(network::IP_ADDRESS), 1337);
const CHANNELS: usize = 2;

#[rtic::app(device = stm32f4xx_hal::pac, dispatchers = [EXTI4, EXTI9_5, EXTI15_10 ])]
mod app {

    use frequency::Ratio;
    use proto::{ 
        FanRequest, 
        FanRequest_, 
        FanResponse, 
        Request, 
        Request_::Peripheral as RequestPeripheral, 
        Response, 
        Response_::Peripheral as ResponsePeripheral, 
        TempRequest };
    use rtic_sync::make_channel;

    use crate::{ 
        frequency::Frequency, 
        network::SendChannel, 
        proto::TempResponse
    };

    use dht11;

    use super::*;

    impl frequency::Proportion<u32> for u32 {}
    type Duration = rtic_monotonics::fugit::Duration<u32, 1, 1_000>;
    impl frequency::Value<Duration, u32> for Duration {} 


    #[shared]
    struct Shared {
        usb: UsbDevice<'static, UsbBusType>,
        network: NetworkStack<'static>,
    }

    #[local]
    struct Local {
        grbl_tx: serial::TxTask<'static>, 
        grbl_rx: serial::RxTask<'static>,
        network_recv: [RecvChannel<'static, CHANNEL_CAPACITY>; CHANNELS],
        requests: shell::CommandRequests<'static, CHANNEL_CAPACITY>,
        responses: shell::CommandResponses<'static, CHANNEL_CAPACITY>,
        network_send: [SendChannel<'static, CHANNEL_CAPACITY>; CHANNELS],
        fan_pwm: shell::TaskResponses<PwmChannel<TIM2, 0>>,
        fan_freq: Frequency<PA1<Input>, Mono, u32>,
        led: PC13<Output>,
        temp: shell::TaskResponses<dht11::Dht11<PC14<Output<OpenDrain>>>>,
    }
    
    fn now() -> smoltcp::time::Instant {
        let time = Mono::now().duration_since_epoch().ticks();
        smoltcp::time::Instant::from_millis(time as i64)
    }

    #[init(local=[
        usb_bus: Option<usb_device::bus::UsbBusAllocator<UsbBusType>> = None,
        ep_memory: [u32; 4096] = [0; 4096],
        grbl_channel_storage: NetworkChannelStorage<CHANNEL_CAPACITY> = NetworkChannelStorage::new(),
        shell_channel_storage: NetworkChannelStorage<CHANNEL_CAPACITY> = NetworkChannelStorage::new(),
        ethernet_in_buffer: [u8; 2048] = [0; 2048],
        ethernet_out_buffer: [u8; 2048] = [0; 2048],
        socket_storage: [SocketStorage<'static>; CHANNELS] = [SocketStorage::EMPTY; CHANNELS]])]
    fn init(cx: init::Context) -> (Shared, Local) {

        let mut peripherals = cx.device;
        let rcc = peripherals.RCC.constrain();
      

        let clocks: rcc::Clocks = rcc
            .cfgr
            .use_hse(25.MHz())
            .sysclk(100.MHz())
            .require_pll48clk()
            .freeze();

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
    
        let mut usb_ethernet = network::usb_ethernet(
            usb_bus, 
            cx.local.ethernet_in_buffer, 
            cx.local.ethernet_out_buffer);
        let interface = network::interface(&mut usb_ethernet);

        let mut network = NetworkStack {
            ethernet: usb_ethernet,
            interface,
            sockets:SocketSet::new(&mut cx.local.socket_storage[..])
        };
    
        let grbl = serial::Tasks::new(
            peripherals.USART1, 
            gpioa.pa9.into(),
            gpioa.pa10, 
            clocks,
            network.channel(cx.local.grbl_channel_storage));

        let shell_channel = network.channel(cx.local.shell_channel_storage);

        // TIM2
        let (_, (fan_pwm, ..)) = peripherals.TIM2.pwm_hz(25.kHz(), &clocks);
        let mut fan_pwm = fan_pwm.with(gpioa.pa0);
        fan_pwm.enable();

        let mut syscfg = peripherals.SYSCFG.constrain();
        let fan_freq = Frequency::new(
            gpioa.pa1.into_pull_up_input(), Ratio(9,1), &mut syscfg, &mut peripherals.EXTI);

        Mono::start(cx.core.SYST, 100_000_000);

        blink::spawn().unwrap();
        grbl_serial_tx::spawn().unwrap();
        usb_send::spawn().unwrap();
        responses::spawn().unwrap();
        requests::spawn().unwrap();

        let (response_sender, 
             response_receiver) = make_channel!(
                shell::Message, 
                { shell::MESSAGE_CAPACITY });

        (Shared {
            usb : usb_device(usb_bus),
            network
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
            fan_pwm: shell::TaskResponses { 
                task: fan_pwm, 
                responses: response_sender.clone() 
            },
            fan_freq,
            led: gpioc.pc13.into_push_pull_output(),
            temp: shell::TaskResponses {
                task: dht11::Dht11::new(gpioc.pc14.into_open_drain_output()),
                responses: response_sender.clone(),
            },
        })
    }

    #[task(local = [ led ])]
    async fn blink(cx: blink::Context) {
        loop {
            Mono::delay(1000.millis()).await;
            cx.local.led.toggle();
        }
    }

    #[task(binds = EXTI1, local = [fan_freq])]
    fn fan_freq_edge(cx: fan_freq_edge::Context) {
        cx.local.fan_freq.edge();
    }

    #[task(binds = OTG_FS, shared = [ usb, network ], local = [ network_recv ])]
    fn usb_recv(cx: usb_recv::Context) {
        let channels = cx.local.network_recv;
        let mut shared = (cx.shared.usb, cx.shared.network);

        shared.lock(|usb, network| {
            if usb.poll(&mut [&mut network.ethernet]) {
                network.try_recv(now(), channels);
            }
        });
    }

    #[task(shared = [ usb, network ], local = [ network_send ])]
    async fn usb_send(cx: usb_send::Context) {
        let channels = cx.local.network_send;
        let mut shared = (cx.shared.usb, cx.shared.network);

        loop {
            shared.lock(|usb, network| {
                usb.poll(&mut [&mut network.ethernet]);
                network.try_send(now(), channels);
            });

            Mono::delay(1.millis()).await;
        }
    }


    #[task(binds = USART1, local=[ grbl_rx ], priority=3)] 
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
        match cx.local.requests.receive().await {
            Request { peripheral: Some(RequestPeripheral::Fan(request)) } => {
                let _ = fan::spawn(request);
            },
            Request { peripheral: Some(RequestPeripheral::Temp(request)) } => {
                let _ = temp::spawn(request);
            },
            Request { peripheral: None } => {}
        };
    }

    #[task(local = [ responses ])]
    async fn responses(cx: responses::Context) {
        cx.local.responses.send().await;
    }


    #[task(local=[temp])]
    async fn temp(cx: temp::Context, _: TempRequest) {
        let temp = cx.local.temp;
        temp.responses.send(Response {
            peripheral: Some(ResponsePeripheral::Temp(TempResponse {

            }))
        }).await.unwrap();
    }

    #[task(local=[fan_pwm])]
    async fn fan(cx: fan::Context, request: FanRequest) {
        let fan_pwm = cx.local.fan_pwm;
        match request {
            FanRequest { command: Some(FanRequest_::Command::Set(set)) } => { 
                fan_pwm.task.set_duty(set.duty as u16);
            },
            FanRequest { command: _ } => { }
        }

        fan_pwm.responses.send(Response { peripheral: Some(ResponsePeripheral::Fan(FanResponse {
            duty: fan_pwm.task.get_duty() as i32
        })) }).await.unwrap();
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


