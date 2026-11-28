//! Reusable, allocation-free acoustic synthesis primitives for procedural effects.
//!
//! These components model useful signal behaviours (band-limited noise, transient excitation,
//! and damped resonant modes). They are deliberately small building blocks rather than claims of
//! complete physical simulation.

use std::f32::consts::TAU;

/// Material-shaped response controls for a short contact event.
#[derive(Clone, Copy, Debug)]
pub(super) struct AcousticProfile {
    frequencies: [f32; 3],
    decays: [f32; 3],
    friction: f32,
    body: f32,
    contact: f32,
    ring: f32,
}

impl AcousticProfile {
    pub(super) fn new(
        frequencies: [f32; 3],
        decays: [f32; 3],
        friction: f32,
        body: f32,
        contact: f32,
        ring: f32,
    ) -> Self {
        Self {
            frequencies: frequencies.map(|value| value.max(1.0)),
            decays: decays.map(|value| value.clamp(0.005, 0.8)),
            friction: friction.clamp(0.0, 1.0),
            body: body.clamp(0.0, 1.5),
            contact: contact.clamp(0.0, 1.5),
            ring: ring.clamp(0.0, 1.5),
        }
    }

    pub(super) fn generic() -> Self {
        Self::new(
            [185.0, 690.0, 1_850.0],
            [0.075, 0.052, 0.031],
            0.55,
            1.0,
            1.0,
            1.0,
        )
    }
}

/// A one-pole low-pass filter with a coefficient prepared for the output sample rate.
#[derive(Clone, Copy, Debug)]
pub(super) struct OnePole {
    state: f32,
    coefficient: f32,
}

impl OnePole {
    pub(super) fn low_pass(sample_rate: f32, cutoff_hz: f32) -> Self {
        let cutoff = cutoff_hz.clamp(1.0, sample_rate * 0.45);
        Self {
            state: 0.0,
            coefficient: 1.0 - (-TAU * cutoff / sample_rate).exp(),
        }
    }

    #[inline]
    pub(super) fn tick(&mut self, input: f32) -> f32 {
        self.state += self.coefficient * (input - self.state);
        self.state
    }

    /// Returns the filter's current internal state for diagnostics and future DSP composition.
    #[allow(dead_code)]
    #[inline]
    pub(super) fn state(&self) -> f32 {
        self.state
    }
}

/// A stable second-order resonator with a controllable exponential decay time.
#[derive(Clone, Copy, Debug)]
pub(super) struct ModalResonator {
    y1: f32,
    y2: f32,
    feedback: f32,
    radius_squared: f32,
}

impl ModalResonator {
    pub(super) fn new(sample_rate: f32, frequency_hz: f32, decay_seconds: f32) -> Self {
        let frequency = frequency_hz.clamp(1.0, sample_rate * 0.45);
        let radius = (-1.0 / (sample_rate * decay_seconds.max(0.002))).exp();
        Self {
            y1: 0.0,
            y2: 0.0,
            feedback: 2.0 * radius * (TAU * frequency / sample_rate).cos(),
            radius_squared: radius * radius,
        }
    }

    #[inline]
    pub(super) fn excite(&mut self, input: f32) -> f32 {
        let output = input + self.feedback * self.y1 - self.radius_squared * self.y2;
        self.y2 = self.y1;
        self.y1 = if output.is_finite() { output } else { 0.0 };
        self.y1
    }

    #[inline]
    pub(super) fn is_quiet(&self) -> bool {
        self.y1.abs().max(self.y2.abs()) < 0.0005
    }
}

/// A transient detector that follows level but responds more strongly to rising energy.
#[derive(Clone, Copy, Debug)]
pub(super) struct TransientEnvelope {
    level: f32,
    attack: f32,
    release: f32,
}

impl TransientEnvelope {
    pub(super) fn new(sample_rate: f32, attack_seconds: f32, release_seconds: f32) -> Self {
        Self {
            level: 0.0,
            attack: 1.0 - (-1.0 / (sample_rate * attack_seconds.max(0.0001))).exp(),
            release: 1.0 - (-1.0 / (sample_rate * release_seconds.max(0.0001))).exp(),
        }
    }

    /// Returns the current smoothed level and the positive level increase for this sample.
    #[inline]
    pub(super) fn tick(&mut self, input: f32) -> (f32, f32) {
        let magnitude = input.abs().min(1.0);
        let rise = (magnitude - self.level).max(0.0);
        let coefficient = if magnitude > self.level {
            self.attack
        } else {
            self.release
        };
        self.level += coefficient * (magnitude - self.level);
        (self.level, rise)
    }

    pub(super) fn is_quiet(&self) -> bool {
        self.level < 0.001
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resonator_is_finite_and_decays_after_an_impulse() {
        let mut resonator = ModalResonator::new(48_000.0, 740.0, 0.08);
        let mut peak = 0.0_f32;
        let mut tail = 0.0_f32;
        for frame in 0..48_000 {
            let sample = resonator.excite(if frame == 0 { 0.8 } else { 0.0 });
            assert!(sample.is_finite());
            if frame < 1_000 {
                peak = peak.max(sample.abs());
            }
            if frame > 40_000 {
                tail = tail.max(sample.abs());
            }
        }
        assert!(peak > 0.1);
        assert!(tail < peak * 0.001);
        assert!(resonator.is_quiet());
    }

    #[test]
    fn transient_envelope_distinguishes_attack_from_release() {
        let mut envelope = TransientEnvelope::new(48_000.0, 0.002, 0.04);
        let (_, rise) = envelope.tick(0.8);
        assert!(rise > 0.7);
        let (level, next_rise) = envelope.tick(0.8);
        assert!(level > 0.0);
        assert!(next_rise > 0.0 && next_rise < rise);
        for _ in 0..20_000 {
            envelope.tick(0.0);
        }
        assert!(envelope.is_quiet());
    }

    #[test]
    fn one_pole_filter_remains_finite_for_extreme_input() {
        let mut filter = OnePole::low_pass(8_000.0, 99_000.0);
        for _ in 0..1_000 {
            assert!(filter.tick(1.0).is_finite());
        }
        assert!(filter.state() <= 1.0);
    }
}
