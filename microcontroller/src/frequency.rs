use core::ops::{ Add, Div, Mul };
use defmt::{ trace };

use rtic_monotonics::Monotonic;
use rtic_sync::signal::{ SignalWriter };
use stm32f4xx_hal::{ gpio::{ Edge, ExtiPin }, pac::EXTI, syscfg::SysCfg};

pub trait Proportion<P> : Add<P, Output = P> + Copy + Clone {}
pub trait Value<V, P>: 
    Add<V, Output=V> + 
    Div<P, Output = V> + 
    Mul<P, Output = V> + 
    Div<P, Output = V> +
    Ord {}

#[derive(Clone, Copy)]
pub struct Ratio<R: Proportion<R>> (pub R, pub R);

impl <R: Proportion<R>> Ratio<R> {
    fn apply<V: Value<V, R>>(self, a: V, b: V) -> V {
        let (ar, br) = (self.0, self.1);
        (a*ar + b*br)/(ar + br)
    }
}
#[derive(Clone, Copy)]
pub struct Bounds<V> (pub V, pub V);

impl <V: Ord> Bounds<V> {
    fn apply(self, x: V) -> Option<V> {
        if x > self.0 && x < self.1 {
            Some(x)
        } else {
            None
        }
    }
}


pub struct Frequency<'a, PIN: ExtiPin, CLOCK: Monotonic, R: Proportion<R>> {
    pin: PIN,
    filter: Ratio<R>,
    bounds: Bounds<CLOCK::Duration>,
    previous: Option<CLOCK::Instant>,
    interval: Option<CLOCK::Duration>,
    writer: SignalWriter<'a, CLOCK::Duration>,
}

impl <'a, PIN: ExtiPin, CLOCK: Monotonic, R> Frequency<'a, PIN, CLOCK, R>
where 
    R: Proportion<R>,
    CLOCK::Duration: Value<CLOCK::Duration, R> {
    pub fn new(
        mut pin: PIN, 
        filter: Ratio<R>, 
        bounds: Bounds<CLOCK::Duration>, 
        syscfg: &mut SysCfg, 
        exti: &mut EXTI,
        writer: SignalWriter<'a, CLOCK::Duration>,
    ) -> Self {
        pin.make_interrupt_source(syscfg);
        pin.trigger_on_edge(exti, Edge::Rising);
        pin.enable_interrupt(exti);
        Frequency { pin, filter, bounds, writer, previous: None, interval: None }
    }

    pub fn edge(&mut self) {
        let now = CLOCK::now();
        self.interval = match (self.previous, self.interval) {
            (Some(instant), Some(duration)) => {
                let next: CLOCK::Duration = now - instant;
                self.bounds.apply(self.filter.apply(duration, next))
            },
            (None, Some(duration)) => self.bounds.apply(duration),
            (Some(instant), None) => self.bounds.apply(now - instant),
            (None,None) => None
        };
        self.previous = Some(now);
        self.pin.clear_interrupt_pending_bit();
        match self.interval {
            Some(duration) => {
                trace!("edge: writing"); 
                self.writer.write(duration); 
            },
            None => {
                trace!("edge: clearing");
                self.writer.clear() 
            },
        };
    }
}