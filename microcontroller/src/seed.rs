use hal::{
  adc::{
    config::{SampleTime}, Adc
  }, gpio::{Analog, Pin}, pac::ADC1
};

use defmt::debug;

pub fn seed(adc: &mut Adc<ADC1>, pa3: &mut Pin<'A', 3, Analog>) -> u64 {
    let mut acc: u64 = 0;
    for _ in 1..16 {
        let sample: u64 = adc.convert(pa3, SampleTime::Cycles_480) as u64;
        debug!("sample: {:b}", sample);
        acc = (acc<<4)  | (sample & 0xf);
   }
   
   debug!("seed: {:x}", acc);
   acc
}