//! Deterministic procedural sound generation, independent of playback and host APIs.
//!
//! Generators render interleaved stereo f32 PCM into caller-owned buffers. Rendering does not
//! allocate. Run generators on a worker and forward output to AudioClip or StreamWriter.

use crate::rng::{derive_seed, Rng};
use std::f32::consts::TAU;

pub const MIN_PROCEDURAL_SAMPLE_RATE: u32 = 8_000;
pub const MAX_PROCEDURAL_SAMPLE_RATE: u32 = 192_000;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ProceduralError {
    InvalidSampleRate,
    InvalidParameter,
    OutputMustContainStereoFrames,
}

impl std::fmt::Display for ProceduralError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::InvalidSampleRate => write!(f, "sample rate must be between {MIN_PROCEDURAL_SAMPLE_RATE} and {MAX_PROCEDURAL_SAMPLE_RATE} Hz"),
            Self::InvalidParameter => write!(f, "parameter must be finite and in 0.0..=1.0"),
            Self::OutputMustContainStereoFrames => write!(f, "output must contain complete interleaved stereo frames"),
        }
    }
}
impl std::error::Error for ProceduralError {}

/// Stateful source of interleaved stereo PCM. Rendering into caller-owned memory does not allocate.
pub trait ProceduralGenerator {
    /// Render complete stereo frames. The output length must be even.
    fn render(&mut self, output: &mut [f32]) -> Result<(), ProceduralError>;
    /// Finite sources return true after their sound has ended; continuous sources return false.
    fn is_finished(&self) -> bool { false }
}

fn validate_rate(rate: u32) -> Result<(), ProceduralError> {
    if !(MIN_PROCEDURAL_SAMPLE_RATE..=MAX_PROCEDURAL_SAMPLE_RATE).contains(&rate) {
        return Err(ProceduralError::InvalidSampleRate);
    }
    Ok(())
}
fn validate_unit(value: f32) -> Result<(), ProceduralError> {
    if !value.is_finite() || !(0.0..=1.0).contains(&value) {
        return Err(ProceduralError::InvalidParameter);
    }
    Ok(())
}
fn validate_output(output: &[f32]) -> Result<(), ProceduralError> {
    if output.len() % 2 != 0 { return Err(ProceduralError::OutputMustContainStereoFrames); }
    Ok(())
}

#[derive(Clone, Copy)]
struct LowPass { state: f32, coefficient: f32 }
impl LowPass {
    fn new(sample_rate: f32, cutoff: f32) -> Self {
        Self { state: 0.0, coefficient: 1.0 - (-TAU * cutoff / sample_rate).exp() }
    }
    fn tick(&mut self, input: f32) -> f32 {
        self.state += self.coefficient * (input - self.state);
        self.state
    }
}

/// Continuous wind configuration.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct WindConfig {
    pub sample_rate: u32,
    /// Wind strength, 0.0..=1.0.
    pub intensity: f32,
    /// High-frequency content, 0.0..=1.0.
    pub brightness: f32,
}
impl Default for WindConfig {
    fn default() -> Self { Self { sample_rate: 44_100, intensity: 0.55, brightness: 0.35 } }
}

/// Deterministic continuous wind source.
pub struct WindGenerator {
    sample_rate: f32, intensity: f32, rng: Rng, filter: LowPass, phase: f32,
}
impl WindGenerator {
    pub fn new(config: WindConfig, seed: u64) -> Result<Self, ProceduralError> {
        validate_rate(config.sample_rate)?;
        validate_unit(config.intensity)?;
        validate_unit(config.brightness)?;
        Ok(Self {
            sample_rate: config.sample_rate as f32,
            intensity: config.intensity,
            rng: Rng::new(derive_seed(seed, 0x5749_4E44, 0)),
            filter: LowPass::new(config.sample_rate as f32, 180.0 + config.brightness * 2_800.0),
            phase: (derive_seed(seed, 0x5749_4E44, 1) as f32 / u64::MAX as f32) * TAU,
        })
    }
}
impl ProceduralGenerator for WindGenerator {
    fn render(&mut self, output: &mut [f32]) -> Result<(), ProceduralError> {
        validate_output(output)?;
        for frame in output.chunks_exact_mut(2) {
            let noise = self.rng.next_f32() * 2.0 - 1.0;
            let filtered = self.filter.tick(noise);
            self.phase = (self.phase + TAU * 0.07 / self.sample_rate).rem_euclid(TAU);
            let gust = 0.58 + 0.42 * (self.phase.sin() * 0.5 + 0.5);
            let sample = filtered * self.intensity * gust * 0.72;
            frame[0] = sample;
            frame[1] = sample;
        }
        Ok(())
    }
}

/// Continuous rain configuration.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct RainConfig {
    pub sample_rate: u32,
    /// Drop density, 0.0..=1.0.
    pub density: f32,
    /// High-frequency content, 0.0..=1.0.
    pub brightness: f32,
    /// Overall strength, 0.0..=1.0.
    pub intensity: f32,
}
impl Default for RainConfig {
    fn default() -> Self { Self { sample_rate: 44_100, density: 0.55, brightness: 0.65, intensity: 0.6 } }
}
const DROP_VOICES: usize = 8;
#[derive(Clone, Copy)]
struct Drop { age: f32, decay: f32, amplitude: f32 }
impl Drop {
    const SILENT: Self = Self { age: 1.0, decay: 1.0, amplitude: 0.0 };
    fn tick(&mut self, age_step: f32) -> f32 {
        if self.age >= 1.0 { return 0.0; }
        let out = self.amplitude * (-self.age * self.decay).exp() * (TAU * (0.08 + self.age * 0.9)).sin();
        self.age += age_step;
        out
    }
}

/// Deterministic continuous rain source with a fixed-size drop voice pool.
pub struct RainGenerator {
    sample_rate: f32, density: f32, brightness: f32, intensity: f32,
    rng: Rng, high_pass_state: f32, drops: [Drop; DROP_VOICES], next_drop: usize,
}
impl RainGenerator {
    pub fn new(config: RainConfig, seed: u64) -> Result<Self, ProceduralError> {
        validate_rate(config.sample_rate)?;
        validate_unit(config.density)?;
        validate_unit(config.brightness)?;
        validate_unit(config.intensity)?;
        Ok(Self {
            sample_rate: config.sample_rate as f32, density: config.density,
            brightness: config.brightness, intensity: config.intensity,
            rng: Rng::new(derive_seed(seed, 0x5241_494E, 0)), high_pass_state: 0.0,
            drops: [Drop::SILENT; DROP_VOICES], next_drop: 0,
        })
    }
}
impl ProceduralGenerator for RainGenerator {
    fn render(&mut self, output: &mut [f32]) -> Result<(), ProceduralError> {
        validate_output(output)?;
        let probability = self.density * (1_800.0 / self.sample_rate);
        let coefficient = 0.025 + self.brightness * 0.65;
        for frame in output.chunks_exact_mut(2) {
            let noise = self.rng.next_f32() * 2.0 - 1.0;
            self.high_pass_state += coefficient * (noise - self.high_pass_state);
            let hiss = (noise - self.high_pass_state) * self.intensity * self.density * 0.10;
            if self.rng.next_f32() < probability {
                let drop = &mut self.drops[self.next_drop];
                drop.age = 0.0;
                drop.decay = 2.5 + self.rng.next_f32() * 4.5;
                drop.amplitude = (0.12 + self.rng.next_f32() * 0.28) * (0.35 + self.brightness * 0.65);
                self.next_drop = (self.next_drop + 1) % DROP_VOICES;
            }
            let age_step = 1.0 / (self.sample_rate * 0.055);\n            let drops: f32 = self.drops.iter_mut().map(|drop| drop.tick(age_step)).sum();
            let sample = (hiss + drops * 0.12) * self.intensity;
            frame[0] = sample.clamp(-1.0, 1.0);
            frame[1] = frame[0];
        }
        Ok(())
    }
}

/// Configuration shared by the one-shot footstep generator.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct FootstepConfig {
    pub sample_rate: u32,
    pub intensity: f32,
    /// Surface brightness, 0.0..=1.0.
    pub brightness: f32,
}
impl Default for FootstepConfig {
    fn default() -> Self { Self { sample_rate: 44_100, intensity: 0.75, brightness: 0.45 } }
}

/// Finite deterministic generator for one footstep.
pub struct FootstepGenerator {
    sample_rate: f32, intensity: f32, brightness: f32, position: u32, total: u32,
    rng: Rng, low_pass: f32,
}
impl FootstepGenerator {
    pub fn new(config: FootstepConfig, seed: u64) -> Result<Self, ProceduralError> {
        validate_rate(config.sample_rate)?;
        validate_unit(config.intensity)?;
        validate_unit(config.brightness)?;
        Ok(Self {
            sample_rate: config.sample_rate as f32, intensity: config.intensity,
            brightness: config.brightness, position: 0, total: (config.sample_rate as f32 * 0.22) as u32,
            rng: Rng::new(derive_seed(seed, 0x5354_4550, 0)), low_pass: 0.0,
        })
    }
}
impl ProceduralGenerator for FootstepGenerator {
    fn render(&mut self, output: &mut [f32]) -> Result<(), ProceduralError> {
        validate_output(output)?;
        for frame in output.chunks_exact_mut(2) {
            if self.position >= self.total { frame.fill(0.0); continue; }
            let t = self.position as f32 / self.sample_rate;
            let noise = self.rng.next_f32() * 2.0 - 1.0;
            self.low_pass += (0.06 + self.brightness * 0.72) * (noise - self.low_pass);
            let body = (TAU * (62.0 - 18.0 * (t * 12.0).min(1.0)) * t).sin() * (-t * 23.0).exp();
            let sample = (body * 0.62 + self.low_pass * (-t * 42.0).exp() * 0.38)
                * self.intensity * (1.0 - t / 0.22).max(0.0);
            frame[0] = sample.clamp(-1.0, 1.0);
            frame[1] = frame[0];
            self.position += 1;
        }
        Ok(())
    }
    fn is_finished(&self) -> bool { self.position >= self.total }
}

/// Configuration for a one-shot impact.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ImpactConfig {
    pub sample_rate: u32,
    pub intensity: f32,
    /// Tonal body amount, 0.0..=1.0.
    pub resonance: f32,
}
impl Default for ImpactConfig {
    fn default() -> Self { Self { sample_rate: 44_100, intensity: 0.8, resonance: 0.55 } }
}

/// Finite deterministic generator for one impact.
pub struct ImpactGenerator {
    sample_rate: f32, intensity: f32, resonance: f32, position: u32, total: u32,
    rng: Rng, low_pass: f32, phase: f32,
}
impl ImpactGenerator {
    pub fn new(config: ImpactConfig, seed: u64) -> Result<Self, ProceduralError> {
        validate_rate(config.sample_rate)?;
        validate_unit(config.intensity)?;
        validate_unit(config.resonance)?;
        Ok(Self {
            sample_rate: config.sample_rate as f32, intensity: config.intensity,
            resonance: config.resonance, position: 0, total: (config.sample_rate as f32 * 0.38) as u32,
            rng: Rng::new(derive_seed(seed, 0x494D_5041, 0)), low_pass: 0.0, phase: 0.0,
        })
    }
}
impl ProceduralGenerator for ImpactGenerator {
    fn render(&mut self, output: &mut [f32]) -> Result<(), ProceduralError> {
        validate_output(output)?;
        for frame in output.chunks_exact_mut(2) {
            if self.position >= self.total { frame.fill(0.0); continue; }
            let t = self.position as f32 / self.sample_rate;
            let noise = self.rng.next_f32() * 2.0 - 1.0;
            self.low_pass += 0.16 * (noise - self.low_pass);
            self.phase = (self.phase + (48.0 + self.resonance * 115.0) / self.sample_rate).rem_euclid(1.0);
            let body = (self.phase * TAU).sin() * (-t * (8.0 + (1.0 - self.resonance) * 20.0)).exp();
            let transient = self.low_pass * (-t * 65.0).exp();
            let sample = (body * self.resonance + transient * (1.0 - self.resonance)) * self.intensity;
            frame[0] = sample.clamp(-1.0, 1.0);
            frame[1] = frame[0];
            self.position += 1;
        }
        Ok(())
    }
    fn is_finished(&self) -> bool { self.position >= self.total }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn render<G: ProceduralGenerator>(generator: &mut G, frames: usize) -> Vec<f32> {
        let mut out = vec![0.0; frames * 2];
        generator.render(&mut out).unwrap();
        out
    }
    fn bounded(samples: &[f32]) {
        assert!(samples.iter().all(|s| s.is_finite() && s.abs() <= 1.0));
    }

    #[test]
    fn continuous_generators_are_deterministic_across_blocks() {
        let mut whole = WindGenerator::new(WindConfig::default(), 42).unwrap();
        let mut split = WindGenerator::new(WindConfig::default(), 42).unwrap();
        let expected = render(&mut whole, 4096);
        let mut actual = render(&mut split, 1024);
        actual.extend(render(&mut split, 3072));
        assert_eq!(expected, actual);
        let mut rain_a = RainGenerator::new(RainConfig::default(), 91).unwrap();
        let mut rain_b = RainGenerator::new(RainConfig::default(), 91).unwrap();
        assert_eq!(render(&mut rain_a, 2048), render(&mut rain_b, 2048));
    }

    #[test]
    fn finite_effects_are_bounded_and_end_in_silence() {
        let mut step = FootstepGenerator::new(FootstepConfig::default(), 7).unwrap();
        let mut impact = ImpactGenerator::new(ImpactConfig::default(), 8).unwrap();
        let a = render(&mut step, 12000);
        let b = render(&mut impact, 20000);
        bounded(&a); bounded(&b);
        assert!(step.is_finished() && impact.is_finished());
        assert!(a[19_404..].iter().all(|s| *s == 0.0));
        assert!(b[33_516..].iter().all(|s| *s == 0.0));
        assert!(a.iter().any(|s| s.abs() > 0.001));
        assert!(b.iter().any(|s| s.abs() > 0.001));
    }

    #[test]
    fn rejects_invalid_configuration_and_odd_output() {
        assert!(matches!(WindGenerator::new(WindConfig { sample_rate: 1000, ..WindConfig::default() }, 0), Err(ProceduralError::InvalidSampleRate)));
        assert!(matches!(RainGenerator::new(RainConfig { density: f32::NAN, ..RainConfig::default() }, 0), Err(ProceduralError::InvalidParameter)));
        let mut wind = WindGenerator::new(WindConfig::default(), 0).unwrap();
        assert_eq!(wind.render(&mut [0.0; 3]), Err(ProceduralError::OutputMustContainStereoFrames));
    }

    #[test]
    fn seeds_and_parameters_affect_only_their_generator() {
        let config = WindConfig::default();
        let mut first = WindGenerator::new(config, 1).unwrap();
        let expected = render(&mut first, 512);
        let mut other = WindGenerator::new(config, 2).unwrap();
        let _ = render(&mut other, 1024);
        let mut repeated = WindGenerator::new(config, 1).unwrap();
        assert_eq!(expected, render(&mut repeated, 512));
        let mut quiet = WindGenerator::new(WindConfig { intensity: 0.2, ..config }, 12).unwrap();
        let mut loud = WindGenerator::new(WindConfig { intensity: 0.9, ..config }, 12).unwrap();
        assert_ne!(render(&mut quiet, 2048), render(&mut loud, 2048));
    }
}
