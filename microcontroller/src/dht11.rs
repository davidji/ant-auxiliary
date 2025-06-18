
use core::result::Result;
use defmt::{ info, warn, Format };

use futures::{select_biased, FutureExt};
use rtic_monotonics::Monotonic;
use rtic_sync::{make_signal, signal::{ Signal, SignalReader, SignalWriter }};
use embedded_hal::digital::{ InputPin, OutputPin };
use stm32f4xx_hal::{ gpio::{ Edge, ExtiPin }, pac::EXTI, syscfg::SysCfg};


#[derive(Clone, Copy, Format)]
struct Packet(u8, [u8;5]);

impl Packet {
    const FIRST: u8 = 40; // MSB first: network data order
    const LAST: u8 = 0;

    const fn new() -> Self {
        Packet(Self::FIRST, [0;5])
    }

    fn append(&mut self, value: bool) {
        self.0 -= 1;
        let byte = (self.0>>3) as usize;
        self.1[byte] = self.1[byte]<<1 | match value { false => 0, true => 1 };
    }

    fn complete(&self) -> bool {
        self.0 == Self::LAST
    }

    fn decode(self) -> Result<TempResponse, ReadError> {
        let humidity = self.word_n(3);
        let temperature = self.word_n(1);
        let crc = self.1[4]
            .wrapping_add(self.1[3])
            .wrapping_add(self.1[2])
            .wrapping_add(self.1[1]);

        if crc == self.1[0] {
            Result::Ok(TempResponse { degrees_c: temperature as i32, humidity: humidity as i32 })        
        } else {
            info!("checksum {} != {} for {}", crc, self.1[0], self.1);
            Result::Err(ReadError::Checksum)
        }
    }

    fn byte_n(self, n: usize) -> u16 {
        self.1[n] as u16
    }

    fn word_n(self, n: usize) -> u16 {
        self.byte_n(n+1)
    }

}

use crate::{ 
    Duration,
    Mono,
    proto::TempResponse };
/*
This works purely on falling edge. The time between the current falling
edge and the previous one gets measured. The interrupt handler waits
for an interval that corresponds to a 'response' of ~160uS. Then
it transitions into the Data state. In the data state, there's a fixed
low of 50uS, followed by a high of 26-28uS for a 0, or 70uS for a 1,
so this looks for an interval of ~80uS or ~120uS respectively
 */
#[derive(Clone, Copy, Format)]
enum InputState {
    Standby,
    Data(Packet)
}

pub struct Dht11Writer<'a, PIN: ExtiPin> {
    timestamp: u32,
    state: InputState,
    writer: SignalWriter<'a, Packet>,
    pin: PIN,
    buckets: [u16;20]
}

struct DurationRange {
    min: u32,
    max: u32,
}

impl DurationRange {
    const fn micros(min: u32, max: u32) -> Self {
        DurationRange { min, max }
    }

    fn contains(self, value: u32) -> bool {
        value > self.min && value < self.max
    }
}

const TIMEOUT: Duration = Duration::micros(2*(160 + 40*120));
const INITIATE: u32 = 18000;
const RESPONSE: DurationRange = DurationRange::micros(150, 170);
const DATA: DurationRange = DurationRange::micros(50, 150);
const DATA_VALUE: u32 = 100;


impl <'a, PIN: ExtiPin> Dht11Writer<'a, PIN> {
    // call on a falling edge.
    // This seems to have an execution time of 30uS - I.e. 3K clock cycles.
    // That is suprisingly high, but it's only half as long as the shortest
    // time between falling edges, so there's still some point in making
    // this interrupt driven.
    pub fn edge(&mut self) {

        let now = Mono::now().duration_since_epoch().to_micros() as u32;
        let interval = now - self.timestamp;
        let bucket = (interval/10) as usize;
        if bucket < 20 {
            self.buckets[bucket] += 1;
        }
        self.timestamp = now;
        let initial = self.state;
        self.state = self.updated(interval, initial);

        // debug!("transition {} => {}", initial, self.state);

        self.pin.clear_interrupt_pending_bit();

    }

    fn updated(&mut self, interval: u32, initial: InputState) -> InputState {
        use InputState::*;

        match initial {
            // Either this is initiate, or the interval between reads
            Standby if interval > INITIATE => Standby,
            Standby if RESPONSE.contains(interval) => Data(Packet::new()),
            Data(mut packet)  if DATA.contains(interval) => {
                packet.append(interval > DATA_VALUE);
                if packet.complete() {
                    self.writer.write(packet);
                    Standby
                } else {
                    Data(packet)
                }
            },
            _ => {
                warn!("unexpected interval {}", interval);
                Standby
            }
        }
    }   
}


#[derive(Clone, Copy, Format)]
pub enum ReadError {
    Checksum,
    Timeout,
    Busy
}


pub struct Dht11Reader<'a, PIN> {
    reader: SignalReader<'a, Packet>,
    pin: Option<PIN>,
}

impl <'a, PIN> Dht11Reader<'a, PIN>
where PIN: ExtiPin + InputPin + OutputPin {

    pub async fn read(&mut self) -> Result<TempResponse, ReadError> {
        match self.pin.take() {
            Some(mut pin) => {
                pin.set_low().unwrap();
                Mono::delay(Duration::millis(20)).await;
                pin.set_high().unwrap();
                let result = select_biased! {
                    packet = self.reader.wait_fresh().fuse() => packet.decode(),
                    _ = Mono::delay(TIMEOUT).fuse() => Result::Err(ReadError::Timeout),
                };
                self.pin.replace(pin);
                result
            },
            None => Result::Err(ReadError::Busy)
        }
    }
}

// altpin: ALTPIN is a hack: it must be another pin which shares the same EXTI. it's
// used to clear the interrupt, without needing a reference to the output pin, which
// can't be shared between an interrupt and an async task.
pub fn make<PIN: ExtiPin + OutputPin, ALTPIN: ExtiPin>(
        pin: PIN,
        altpin: ALTPIN,
        syscfg: &mut SysCfg, 
        exti: &mut EXTI) -> (Dht11Writer<'static, ALTPIN>, Dht11Reader<'static, PIN>)
{
    let (writer, reader) = make_signal!(Packet);

    let mut io = pin;
    io.make_interrupt_source(syscfg);
    io.trigger_on_edge(exti, Edge::Falling);
    io.enable_interrupt(exti);
    
    (
        Dht11Writer { writer, timestamp: 0, state: InputState::Standby, pin: altpin, buckets: [0;20] },
        Dht11Reader { reader, pin : Option::Some(io) },
    )
}
