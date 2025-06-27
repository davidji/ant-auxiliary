
use core::{result::Result, u32};
use defmt::{ debug, error, Format };

use futures::{select_biased, FutureExt};
use rtic_monotonics::Monotonic;
use rtic_sync::{make_signal, signal::{ SignalReader, SignalWriter }};
use embedded_hal::digital::{ InputPin, OutputPin };
use stm32f4xx_hal::{ gpio::{ Edge, ExtiPin }, pac::{ EXTI }, syscfg::SysCfg};

use crate::statistics::StatsAccumulator;

#[derive(Clone, Copy, Format)]
pub struct Packet(u8, [u8;5]);

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
        let humidity_percent = self.byte_n(3) as f32 + (self.byte_n(4) as f32 / 10.0);
        let temperature_celsius = self.word_n(1) as f32 + (self.byte_n(2) as f32 / 10.0);
        let crc = self.1[4]
            .wrapping_add(self.1[3])
            .wrapping_add(self.1[2])
            .wrapping_add(self.1[1]);

        if crc == self.1[0] {
            Result::Ok(TempResponse { temperature_celsius, humidity_percent })        
        } else {
            error!("checksum {} != {} for {}", crc, self.1[0], self.1);
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
so this looks for an interval of ~77uS or ~120uS respectively
 */
#[derive(Clone, Copy, Format)]
pub enum InputState {
    Standby,
    Data(Packet),
    Error
}

#[derive(Clone, Copy, Format)]
pub enum ReadError {
    Checksum,
    Timeout,
    Timing(InputState, u32),
    Busy
}

#[derive(Clone, Copy, Format)]
pub struct Statistics {
    pub interrupt: StatsAccumulator<u32,f32>,
    pub response: StatsAccumulator<u32,f32>,
    pub zero: StatsAccumulator<u32,f32>,
    pub one: StatsAccumulator<u32,f32>,
}

pub struct Dht11Writer<'a, PIN: ExtiPin> {
    timestamp: u32,
    state: InputState,
    writer: SignalWriter<'a, (Statistics, Result<Packet, ReadError>)>,
    pin: PIN,
    statistics: Statistics,
}

struct DurationRange {
    min: u32,
    max: u32,
    med: u32,
}

impl DurationRange {
    const fn micros(min: u32, max: u32) -> Self {
        DurationRange { min, max, med: (min+max)/2 }
    }

    fn contains(self, value: u32) -> bool {
        value > self.min && value < self.max
    }
}

const TIMEOUT: Duration = Duration::micros(2*(160 + 40*120));
const INITIATE: u32 = 18000;
const RESPONSE: DurationRange = DurationRange::micros(150, 180);
const DATA: DurationRange = DurationRange::micros(50, 150);

impl <'a, PIN: ExtiPin> Dht11Writer<'a, PIN> {
    // call on a falling edge.
    // This is called at ~100uS intervals, so it needs to take much less than that to execute.
    // The execution time with opt-level = 2 or "s" is ~4uS, and works reliably. 
    // opt-level = 0 does not work.
    pub fn falling_edge(&mut self) {

        let before = Mono::now().ticks() as u32;
        // deal with wrapping
        let interval = if before < self.timestamp { u32::MAX - self.timestamp + before } else  { before - self.timestamp };

        self.timestamp = before;
        let initial = self.state;
        self.state = self.updated(interval, initial);

        let after = Mono::now().ticks() as u32;
        self.statistics.interrupt.add((after - before) as f32);

        self.pin.clear_interrupt_pending_bit();

    }
    
    fn updated(&mut self, interval: u32, initial: InputState) -> InputState {
        use InputState::*;

        match initial {
            // Either this is initiate, or the interval between reads
            Error | Standby if interval > INITIATE => Standby,
            Standby if RESPONSE.contains(interval) => {
                self.statistics.response.add(interval as f32);
                Data(Packet::new())
            },
            Data(mut packet)  if DATA.contains(interval) => {
                let value = interval > DATA.med;
                if value { &mut self.statistics.one } else { &mut self.statistics.zero }.add(interval as f32);
                packet.append(value);
                if packet.complete() {
                    self.writer.write((self.statistics, Result::Ok(packet)));
                    Standby
                } else {
                    Data(packet)
                }
            },
            Standby | Data(_) => {
                self.writer.write((self.statistics, Result::Err(ReadError::Timing(initial, interval))));
                Error
            },
            Error => Error,
        }
    }

}



pub struct Dht11Reader<'a, PIN> {
    reader: SignalReader<'a, (Statistics, Result<Packet, ReadError>)>,
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
                    (statistics, result) = self.reader.wait_fresh().fuse() => {
                        debug!("statistics: {}", statistics);
                        match result {
                            Result::Ok(packet) => packet.decode(),
                            Result::Err(err) => Result::Err(err),
                        }
                    },
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
    let (writer, reader) = 
        make_signal!((Statistics, Result<Packet, ReadError>));

    let mut io = pin;
    io.make_interrupt_source(syscfg);
    io.trigger_on_edge(exti, Edge::Falling);
    io.enable_interrupt(exti);
    
    (
        Dht11Writer { 
            writer, 
            timestamp: 0, 
            state: InputState::Standby, 
            pin: altpin, 
            statistics: Statistics {
                interrupt: StatsAccumulator::new(),
                zero: StatsAccumulator::new(),
                one: StatsAccumulator::new(),
                response: StatsAccumulator::new(), 
            },
        },
        Dht11Reader { reader, pin : Option::Some(io) },
    )
}
