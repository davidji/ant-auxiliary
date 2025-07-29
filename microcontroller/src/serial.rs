
use defmt::{ warn };
use hal::{
    gpio::gpioa,
    pac,
    prelude::*,
    rcc,
    serial,
};

use nb::Error::WouldBlock;
use rtic_sync::channel::*;
use rtic_monotonics::Monotonic;
use crate::Mono;
use crate::network::{ NetworkChannel, NetworkEndpoint };

pub type GrblTx = serial::Tx1;
pub type GrblRx = serial::Rx1;

const CHANNEL_CAPACITY: usize = 2*crate::network::MTU as usize;

pub struct TxTask<'a> {
    tx: GrblTx, 
    receiver: Receiver<'a, u8, CHANNEL_CAPACITY>,
}

impl <'a> TxTask<'a> {
    pub async fn send(&mut self) {
        match self.receiver.recv().await {
            Ok(data) => loop {
                match self.tx.write(data) {
                    Ok(_) => break,
                    Err(WouldBlock) => Mono::delay(1.millis().into()).await,
                    Err(_) => panic!("Error writing to GRBL serial")
                }
            },
            Err(_) => Mono::delay(5.millis().into()).await
        };
    }
}

pub struct RxTask<'a> {
    rx: GrblRx,
    sender: Sender<'a, u8, CHANNEL_CAPACITY>,
}


impl <'a> RxTask<'a> {
    pub fn receive(&mut self) {
        while self.rx.is_rx_not_empty() {
            match self.rx.read() {
                Ok(data) => self.sender.try_send(data).unwrap(),
                Err(WouldBlock) => { },
                Err(nb::Error::Other(e)) => {
                    warn!("Error reading from GRBL serial: {:?}", match e {
                        serial::Error::Overrun=>"Overrun",
                        serial::Error::Noise=>"Noise",
                        serial::Error::Parity=>"Parity",
                        serial::Error::FrameFormat=>"Framing",
                        serial::Error::Other=>"Other",
                        _ => "Unknown",
                    });
                    break;
                }
            }
        }
    }

    pub fn listen(&mut self) {
        self.rx.listen();
    }
}

pub fn grbl_serial(
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

pub struct Tasks<'a> {
    pub tx: TxTask<'a>,
    pub rx: RxTask<'a>,
    pub net: NetworkEndpoint<'a, CHANNEL_CAPACITY>
}

impl <'a> Tasks<'a> {

    pub fn new(
    usart: pac::USART1,
    tx: gpioa::PA9,
    rx: gpioa::PA10,
    clocks: rcc::Clocks,
    channel: NetworkChannel<'a, CHANNEL_CAPACITY>) -> Tasks<'a> {

        let (tx, mut rx) = grbl_serial(usart, tx, rx, clocks);
        rx.listen();
        Tasks {
            tx: TxTask { tx, receiver: channel.app.recv},
            rx: RxTask { rx, sender: channel.app.send},
            net: channel.net
        }
    }
}