use core::ops::{Add, Div, Mul};

use rtic_monotonics::Monotonic;
use stm32f4xx_hal::{ gpio::{ Edge, ExtiPin }, pac::EXTI, syscfg::SysCfg};

pub trait Proportion<P> : Add<P, Output = P> + Copy + Clone {}
pub trait Value<V, P>: Add<V, Output=V> + Div<P, Output = V> + Mul<P, Output = V> + Div<P, Output = V> {}

#[derive(Clone, Copy)]
pub struct Ratio<R: Proportion<R>> (pub R, pub R);

impl <R: Proportion<R>> Ratio<R> {
    fn apply<V: Value<V, R>>(self, a: V, b: V) -> V {
        let (ar, br) = (self.0, self.1);
        (a*ar + b*br)/(ar + br)
    }
}

pub struct Frequency<PIN: ExtiPin, CLOCK: Monotonic, R: Proportion<R>> {
    pin: PIN,
    filter: Ratio<R>,
    previous: Option<CLOCK::Instant>,
    interval: Option<CLOCK::Duration>,
}

impl <PIN: ExtiPin, CLOCK: Monotonic, R> Frequency<PIN, CLOCK, R>
where 
    PIN: ExtiPin,
    R: Proportion<R>,
    CLOCK::Duration: Value<CLOCK::Duration, R> {
    pub fn new(mut pin: PIN, filter: Ratio<R>, syscfg: &mut SysCfg, exti: &mut EXTI) -> Self {
        pin.make_interrupt_source(syscfg);
        pin.trigger_on_edge(exti, Edge::Rising);
        pin.enable_interrupt(exti);
        Frequency { pin, filter, previous: None, interval: None }
    }

    pub fn edge(&mut self) {
        let now = CLOCK::now();
        self.interval = match (self.previous, self.interval) {
            (Some(instant), Some(duration)) => {
                let next: CLOCK::Duration = now - instant;
                Some(self.filter.apply(duration, next))
            },
            (None, Some(duration)) => Some(duration),
            (Some(instant), None) => Some(now - instant),
            (None,None) => None
        };
        self.previous = Some(now);
        self.pin.clear_interrupt_pending_bit();
    }
}