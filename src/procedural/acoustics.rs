//! Reusable, allocation-free acoustic synthesis primitives for procedural effects.
//!
//! These components model useful signal behaviours (band-limited noise, transient excitation,
//! and damped resonant modes). They are deliberately small building blocks rather than claims of
//! complete physical simulation.

use std::f32::consts::TAU;

/// Material-shaped response controls for a short contact event.
#[derive(Clone, Copy, Debug)]
pub(super) struct AcousticProfile {
    pub(super) frequencies: [f32; 3],
    pub(super) decays: [f32; 3],
    pub(super) friction: f32,
    pub(super) body: f32,
    pub(super) contact: f32,
    pub(super) ring: f32,
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
        let mut filter = Self {
            state: 0.0,
            coefficient: 0.0,
        };
        filter.set_low_pass(sample_rate, cutoff_hz);
        filter
    }

    /// Change the cutoff without discarding the filter's current signal state.
    pub(super) fn set_low_pass(&mut self, sample_rate: f32, cutoff_hz: f32) {
        let cutoff = cutoff_hz.clamp(1.0, sample_rate * 0.45);
        self.coefficient = 1.0 - (-TAU * cutoff / sample_rate).exp();
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


const REFLECTION_BUFFER_FRAMES: usize = 8_192;
const REFLECTION_GAINS: [f32; 3] = [0.32, 0.22, 0.14];
const FEEDBACK_WEIGHTS: [f32; 3] = [0.50, 0.32, 0.18];

/// Fixed-state environmental filtering, early reflections, and a bounded feedback tail.
pub(super) struct EnvironmentProcessor {
    direct_filters: [OnePole; 2],
    feedback_filters: [OnePole; 2],
    reflection_filters: [OnePole; 2],
    delay_buffers: [[f32; REFLECTION_BUFFER_FRAMES]; 2],
    delays: [[usize; 3]; 2],
    write_index: usize,
    frames_since_enable: usize,
    direct_filter_enabled: bool,
    direct_gain: f32,
    target_direct_gain: f32,
    reflection_gain: f32,
    target_reflection_gain: f32,
    feedback_gain: f32,
    smoothing: f32,
    tail_release: f32,
    tail_level: f32,
}

impl EnvironmentProcessor {
    pub(super) fn new(
        sample_rate: u32,
        distance: f32,
        occlusion: f32,
        room_size: f32,
        absorption: f32,
        wet: f32,
    ) -> Self {
        let rate = sample_rate as f32;
        let mut processor = Self {
            direct_filters: [OnePole::low_pass(rate, 18_000.0); 2],
            feedback_filters: [OnePole::low_pass(rate, 8_000.0); 2],
            reflection_filters: [OnePole::low_pass(rate, 8_000.0); 2],
            delay_buffers: [[0.0; REFLECTION_BUFFER_FRAMES]; 2],
            delays: [[1; 3]; 2],
            write_index: 0,
            frames_since_enable: 0,
            direct_filter_enabled: false,
            direct_gain: 1.0,
            target_direct_gain: 1.0,
            reflection_gain: 0.0,
            target_reflection_gain: 0.0,
            feedback_gain: 0.0,
            smoothing: 1.0 - (-1.0 / (rate * 0.025)).exp(),
            tail_release: (-1.0 / (rate * 0.3)).exp(),
            tail_level: 0.0,
        };
        processor.configure(sample_rate, distance, occlusion, room_size, absorption, wet);
        processor.direct_gain = processor.target_direct_gain;
        processor.reflection_gain = processor.target_reflection_gain;
        processor
    }

    pub(super) fn configure(
        &mut self,
        sample_rate: u32,
        distance: f32,
        occlusion: f32,
        room_size: f32,
        absorption: f32,
        wet: f32,
    ) {
        let was_reflections_disabled =
            self.target_reflection_gain <= 0.000001 && self.reflection_gain <= 0.000001;
        let rate = sample_rate as f32;
        self.direct_filter_enabled = distance > 0.0 || occlusion > 0.0;
        self.target_direct_gain =
            (1.0 / (1.0 + distance * 3.5)) * (1.0 - occlusion * 0.58);
        let direct_cutoff = 18_000.0 / (1.0 + distance * 2.5 + occlusion * 12.0);
        for filter in &mut self.direct_filters {
            filter.set_low_pass(rate, direct_cutoff);
        }

        self.target_reflection_gain = wet * room_size * (1.0 - absorption * 0.5);
        if was_reflections_disabled && self.target_reflection_gain > 0.000001 {
            self.write_index = 0;
            self.frames_since_enable = 0;
            self.tail_level = 0.0;
        }
        self.feedback_gain = 0.54 - absorption * 0.34;
        let feedback_cutoff = 14_000.0 - absorption * 11_500.0;
        for filter in &mut self.feedback_filters {
            filter.set_low_pass(rate, feedback_cutoff);
        }
        for filter in &mut self.reflection_filters {
            filter.set_low_pass(rate, feedback_cutoff);
        }

        let milliseconds = [
            [
                4.0 + room_size * 8.0,
                9.0 + room_size * 12.0,
                16.0 + room_size * 17.0,
            ],
            [
                4.7 + room_size * 8.6,
                10.2 + room_size * 13.4,
                18.1 + room_size * 18.2,
            ],
        ];
        for channel in 0..2 {
            for tap in 0..3 {
                let frames = (rate * milliseconds[channel][tap] * 0.001).round() as usize;
                self.delays[channel][tap] =
                    frames.clamp(1, REFLECTION_BUFFER_FRAMES - 1);
            }
        }
    }

    #[inline]
    fn read_delay(&self, channel: usize, delay: usize) -> f32 {
        if self.frames_since_enable < delay {
            return 0.0;
        }
        self.delay_buffers[channel]
            [(self.write_index + REFLECTION_BUFFER_FRAMES - delay)
                & (REFLECTION_BUFFER_FRAMES - 1)]
    }

    #[inline]
    pub(super) fn process_frame(&mut self, left: f32, right: f32) -> (f32, f32) {
        self.direct_gain +=
            self.smoothing * (self.target_direct_gain - self.direct_gain);
        self.reflection_gain +=
            self.smoothing * (self.target_reflection_gain - self.reflection_gain);

        let input = [left, right];
        let mut output = [0.0; 2];
        let mut sent_level = 0.0_f32;
        let reflections_enabled = self.reflection_gain > 0.000001
            || self.target_reflection_gain > 0.000001;

        for channel in 0..2 {
            let filtered = if self.direct_filter_enabled {
                self.direct_filters[channel].tick(input[channel])
            } else {
                input[channel]
            };
            let direct = filtered * self.direct_gain;

            if reflections_enabled {
                let first = self.read_delay(channel, self.delays[channel][0]);
                let second = self.read_delay(channel, self.delays[channel][1]);
                let third = self.read_delay(channel, self.delays[channel][2]);
                let feedback = (first * FEEDBACK_WEIGHTS[0]
                    + second * FEEDBACK_WEIGHTS[1]
                    + third * FEEDBACK_WEIGHTS[2])
                    * self.feedback_gain;
                let feedback = self.feedback_filters[channel].tick(feedback);
                self.delay_buffers[channel][self.write_index] =
                    (direct + feedback).clamp(-2.0, 2.0);

                let reflected = self.reflection_filters[channel].tick(
                    first * REFLECTION_GAINS[0]
                        + second * REFLECTION_GAINS[1]
                        + third * REFLECTION_GAINS[2],
                ) * self.reflection_gain;
                output[channel] = (direct + reflected).clamp(-1.0, 1.0);
                sent_level = sent_level.max(direct.abs() * self.reflection_gain);
            } else {
                output[channel] = direct.clamp(-1.0, 1.0);
            }
        }

        if reflections_enabled {
            self.write_index =
                (self.write_index + 1) & (REFLECTION_BUFFER_FRAMES - 1);
            self.frames_since_enable = self.frames_since_enable.saturating_add(1);
        }
        self.tail_level = (self.tail_level * self.tail_release).max(sent_level);
        (output[0], output[1])
    }

    pub(super) fn is_quiet(&self) -> bool {
        self.tail_level < 0.0005
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
        let state = filter.state();
        filter.set_low_pass(8_000.0, 240.0);
        assert_eq!(filter.state(), state);
    }

    #[test]
    fn environmental_reflection_delays_fit_the_fixed_buffer_at_maximum_sample_rate() {
        let processor = EnvironmentProcessor::new(192_000, 0.0, 0.0, 1.0, 0.2, 1.0);
        assert!(processor
            .delays
            .iter()
            .flatten()
            .all(|delay| *delay < REFLECTION_BUFFER_FRAMES));
        assert!(processor.delays[1][2] > processor.delays[0][2]);
    }
}
