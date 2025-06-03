
use core::ops::Sub;

use rtic_monotonics::{ TimerQueueBasedMonotonic };

use fugit;

struct MonoTimer<const RATE: u32> {

}

impl <const RATE: u32> TimerQueueBasedMonotonic for MonoTimer<RATE> {
    type Instant = fugit::Instant<u64, RATE, 1>;
    type Duration = fugit::Duration<u64, RATE, 1>;
    
    type Backend;

}