
use defmt::Format;
use micromath::F32Ext;


#[derive(Clone, Copy)]
pub struct StatsAccumulator<COUNT, VALUE> {
    count: COUNT,
    mean: VALUE,
    sum_of_squares_of_deltas: VALUE,
    min: VALUE,
    max: VALUE,
}



impl StatsAccumulator<u32, f32>{
    pub fn new() -> Self {
        StatsAccumulator {
            count: 0,
            mean: 0.0,
            sum_of_squares_of_deltas: 0.0.into(),
            min: f32::INFINITY.into(),
            max: f32::NEG_INFINITY.into(),
        }
    }

    pub fn add(&mut self, value: f32) {
        self.count += 1;
        if self.count == 1 {
            self.mean = value;
            self.min = value;
            self.max = value;
        } else {
            let delta = value - self.mean;
            self.mean += delta / self.count as f32;
            self.sum_of_squares_of_deltas += delta * (value - self.mean);
            if value < self.min {
                self.min = value;
            }
            if value > self.max {
                self.max = value;
            }
        }
    }

    #[inline]
    pub fn count(&self) -> u32 {
        self.count
    }

    #[inline]
    pub fn mean(&self) -> f32 {
        self.mean
    }

    #[inline]
    pub fn min(&self) -> f32 {
        self.min
    }

    #[inline]
    pub fn max(&self) -> f32 {
        self.max
    }

    pub fn variance(&self) -> Option<f32> {
        if self.count > 1 {
            Some(self.sum_of_squares_of_deltas / (self.count as f32 - 1.0))
        } else {
            None
        }
    }

    pub fn stddev(&self) -> Option<f32> {
        self.variance().map(|v| v.sqrt())
    }
}

impl Format for StatsAccumulator<u32, f32> {
    fn format(&self, f: defmt::Formatter) {
        defmt::write!(
            f,
            "StatsAccumulator(count: {}, mean: {}, min: {}, max: {}, variance: {:?}, stddev: {:?})",
            self.count(),
            self.mean(),
            self.min(),
            self.max(),
            self.variance(),
            self.stddev()
        );
    }
}
