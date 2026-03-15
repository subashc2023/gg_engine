/// Time elapsed since the last frame, in seconds.
///
/// `Timestep` is a thin `Copy` wrapper around `f32`. It can be multiplied
/// with speed values directly (`5.0 * dt`) and provides both seconds and
/// milliseconds accessors.
#[derive(Clone, Copy, Debug, PartialEq, PartialOrd)]
pub struct Timestep(f32);

impl Timestep {
    /// Create a timestep from a duration in seconds.
    pub fn from_seconds(seconds: f32) -> Self {
        Self(seconds)
    }

    /// Duration in seconds.
    pub fn seconds(self) -> f32 {
        self.0
    }

    /// Duration in milliseconds.
    pub fn millis(self) -> f32 {
        self.0 * 1000.0
    }
}

impl From<f32> for Timestep {
    fn from(seconds: f32) -> Self {
        Self(seconds)
    }
}

impl From<Timestep> for f32 {
    fn from(ts: Timestep) -> Self {
        ts.0
    }
}

impl std::ops::Mul<Timestep> for f32 {
    type Output = f32;
    fn mul(self, rhs: Timestep) -> f32 {
        self * rhs.0
    }
}

impl std::ops::Mul<f32> for Timestep {
    type Output = f32;
    fn mul(self, rhs: f32) -> f32 {
        self.0 * rhs
    }
}

impl std::fmt::Display for Timestep {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{:.3}ms", self.millis())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn seconds_and_millis() {
        let ts = Timestep::from_seconds(0.016);
        assert!((ts.seconds() - 0.016).abs() < f32::EPSILON);
        assert!((ts.millis() - 16.0).abs() < 0.01);
    }

    #[test]
    fn mul_with_float() {
        let ts = Timestep::from_seconds(0.5);
        assert!((5.0_f32 * ts - 2.5).abs() < f32::EPSILON);
        assert!((ts * 5.0 - 2.5).abs() < f32::EPSILON);
    }

    #[test]
    fn from_conversions() {
        let ts: Timestep = 0.016_f32.into();
        let val: f32 = ts.into();
        assert!((val - 0.016).abs() < f32::EPSILON);
    }
}
