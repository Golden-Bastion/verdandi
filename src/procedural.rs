//! Deterministic procedural sound generation, independent of playback and host APIs.
//!
//! Generators render interleaved stereo f32 PCM into caller-owned buffers. Rendering does not
//! allocate. Run generators on a worker and forward output to AudioClip or StreamWriter.

use crate::rng::{derive_seed, Rng};
use std::f32::consts::TAU;

/// Lowest supported procedural generator sample rate.
pub const MIN_PROCEDURAL_SAMPLE_RATE: u32 = 8_000;
/// Highest supported procedural generator sample rate.
pub const MAX_PROCEDURAL_SAMPLE_RATE: u32 = 192_000;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ProceduralError {
    /// The requested sample rate is outside the supported range.
    InvalidSampleRate,
    /// A normalized parameter is non-finite or outside 0.0..=1.0.
    InvalidParameter,
    /// The output slice does not contain complete stereo frames.
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
    fn is_finished(&self) -> bool {
        false
    }
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
    if !output.len().is_multiple_of(2) {
        return Err(ProceduralError::OutputMustContainStereoFrames);
    }
    Ok(())
}

#[derive(Clone, Copy)]
struct LowPass {
    state: f32,
    coefficient: f32,
}
impl LowPass {
    fn new(sample_rate: f32, cutoff: f32) -> Self {
        Self {
            state: 0.0,
            coefficient: 1.0 - (-TAU * cutoff / sample_rate).exp(),
        }
    }
    fn tick(&mut self, input: f32) -> f32 {
        self.state += self.coefficient * (input - self.state);
        self.state
    }
}

/// Continuous wind configuration.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct WindConfig {
    /// Output sample rate in Hz.
    pub sample_rate: u32,
    /// Wind strength, 0.0..=1.0.
    pub intensity: f32,
    /// High-frequency content, 0.0..=1.0.
    pub brightness: f32,
}
impl Default for WindConfig {
    fn default() -> Self {
        Self {
            sample_rate: 44_100,
            intensity: 0.55,
            brightness: 0.35,
        }
    }
}

/// Deterministic continuous wind source.
pub struct WindGenerator {
    sample_rate: f32,
    intensity: f32,
    rng: Rng,
    filter: LowPass,
    phase: f32,
}
impl WindGenerator {
    /// Construct a wind source with an independent deterministic random stream.
    pub fn new(config: WindConfig, seed: u64) -> Result<Self, ProceduralError> {
        validate_rate(config.sample_rate)?;
        validate_unit(config.intensity)?;
        validate_unit(config.brightness)?;
        Ok(Self {
            sample_rate: config.sample_rate as f32,
            intensity: config.intensity,
            rng: Rng::new(derive_seed(seed, 0x5749_4E44, 0)),
            filter: LowPass::new(
                config.sample_rate as f32,
                180.0 + config.brightness * 2_800.0,
            ),
            phase: (derive_seed(seed, 0x5749_4E44, 1) as f32 / u64::MAX as f32) * TAU,
        })
    }
}
impl ProceduralGenerator for WindGenerator {
    fn render(&mut self, output: &mut [f32]) -> Result<(), ProceduralError> {
        validate_output(output)?;
        let (frames, _) = output.as_chunks_mut::<2>();
        for frame in frames {
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
    /// Output sample rate in Hz.
    pub sample_rate: u32,
    /// Drop-event density, 0.0..=1.0.
    pub density: f32,
    /// High-frequency content, 0.0..=1.0.
    pub brightness: f32,
    /// Overall strength, 0.0..=1.0.
    pub intensity: f32,
}
impl Default for RainConfig {
    fn default() -> Self {
        Self {
            sample_rate: 44_100,
            density: 0.55,
            brightness: 0.65,
            intensity: 0.6,
        }
    }
}
const DROP_VOICES: usize = 8;
#[derive(Clone, Copy)]
struct Drop {
    age: f32,
    decay: f32,
    amplitude: f32,
    phase: f32,
    phase_step: f32,
}
impl Drop {
    const SILENT: Self = Self {
        age: 1.0,
        decay: 1.0,
        amplitude: 0.0,
        phase: 0.0,
        phase_step: 0.0,
    };
    fn tick(&mut self, age_step: f32) -> f32 {
        if self.age >= 1.0 {
            return 0.0;
        }
        let out = self.amplitude * (-self.age * self.decay).exp() * (TAU * self.phase).sin();
        self.phase = (self.phase + self.phase_step).rem_euclid(1.0);
        self.age += age_step;
        out
    }
}

/// Deterministic continuous rain source with a fixed-size drop voice pool.
pub struct RainGenerator {
    sample_rate: f32,
    density: f32,
    brightness: f32,
    intensity: f32,
    rng: Rng,
    high_pass_state: f32,
    drops: [Drop; DROP_VOICES],
    next_drop: usize,
}
impl RainGenerator {
    /// Construct a rain source with an independent deterministic random stream.
    pub fn new(config: RainConfig, seed: u64) -> Result<Self, ProceduralError> {
        validate_rate(config.sample_rate)?;
        validate_unit(config.density)?;
        validate_unit(config.brightness)?;
        validate_unit(config.intensity)?;
        Ok(Self {
            sample_rate: config.sample_rate as f32,
            density: config.density,
            brightness: config.brightness,
            intensity: config.intensity,
            rng: Rng::new(derive_seed(seed, 0x5241_494E, 0)),
            high_pass_state: 0.0,
            drops: [Drop::SILENT; DROP_VOICES],
            next_drop: 0,
        })
    }
}
impl ProceduralGenerator for RainGenerator {
    fn render(&mut self, output: &mut [f32]) -> Result<(), ProceduralError> {
        validate_output(output)?;
        let probability = self.density * (120.0 / self.sample_rate);
        let coefficient = 0.025 + self.brightness * 0.65;
        let (frames, _) = output.as_chunks_mut::<2>();
        for frame in frames {
            let noise = self.rng.next_f32() * 2.0 - 1.0;
            self.high_pass_state += coefficient * (noise - self.high_pass_state);
            let hiss = (noise - self.high_pass_state) * self.intensity * self.density * 0.10;
            if self.rng.next_f32() < probability {
                let drop = &mut self.drops[self.next_drop];
                drop.age = 0.0;
                drop.decay = 2.5 + self.rng.next_f32() * 4.5;
                drop.amplitude =
                    (0.12 + self.rng.next_f32() * 0.28) * (0.35 + self.brightness * 0.65);
                drop.phase = 0.0;
                drop.phase_step = (900.0 + self.rng.next_f32() * 3_000.0) / self.sample_rate;
                self.next_drop = (self.next_drop + 1) % DROP_VOICES;
            }
            let age_step = 1.0 / (self.sample_rate * 0.055);
            let drops: f32 = self.drops.iter_mut().map(|drop| drop.tick(age_step)).sum();
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
    /// Output sample rate in Hz.
    pub sample_rate: u32,
    /// Step strength, 0.0..=1.0.
    pub intensity: f32,
    /// Surface brightness, 0.0..=1.0.
    pub brightness: f32,
}
impl Default for FootstepConfig {
    fn default() -> Self {
        Self {
            sample_rate: 44_100,
            intensity: 0.75,
            brightness: 0.45,
        }
    }
}

/// Finite deterministic generator for one footstep.
pub struct FootstepGenerator {
    sample_rate: f32,
    intensity: f32,
    brightness: f32,
    position: u32,
    total: u32,
    rng: Rng,
    low_pass: f32,
}
impl FootstepGenerator {
    /// Construct one deterministic footstep.
    pub fn new(config: FootstepConfig, seed: u64) -> Result<Self, ProceduralError> {
        validate_rate(config.sample_rate)?;
        validate_unit(config.intensity)?;
        validate_unit(config.brightness)?;
        Ok(Self {
            sample_rate: config.sample_rate as f32,
            intensity: config.intensity,
            brightness: config.brightness,
            position: 0,
            total: (config.sample_rate as f32 * 0.22) as u32,
            rng: Rng::new(derive_seed(seed, 0x5354_4550, 0)),
            low_pass: 0.0,
        })
    }
}
impl ProceduralGenerator for FootstepGenerator {
    fn render(&mut self, output: &mut [f32]) -> Result<(), ProceduralError> {
        validate_output(output)?;
        let (frames, _) = output.as_chunks_mut::<2>();
        for frame in frames {
            if self.position >= self.total {
                frame.fill(0.0);
                continue;
            }
            let t = self.position as f32 / self.sample_rate;
            let noise = self.rng.next_f32() * 2.0 - 1.0;
            self.low_pass += (0.06 + self.brightness * 0.72) * (noise - self.low_pass);
            let body = (TAU * (62.0 - 18.0 * (t * 12.0).min(1.0)) * t).sin() * (-t * 23.0).exp();
            let sample = (body * 0.62 + self.low_pass * (-t * 42.0).exp() * 0.38)
                * self.intensity
                * (1.0 - t / 0.22).max(0.0);
            frame[0] = sample.clamp(-1.0, 1.0);
            frame[1] = frame[0];
            self.position += 1;
        }
        Ok(())
    }
    fn is_finished(&self) -> bool {
        self.position >= self.total
    }
}

/// Configuration for a one-shot impact.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ImpactConfig {
    /// Output sample rate in Hz.
    pub sample_rate: u32,
    /// Impact strength, 0.0..=1.0.
    pub intensity: f32,
    /// Tonal body amount, 0.0..=1.0.
    pub resonance: f32,
}
impl Default for ImpactConfig {
    fn default() -> Self {
        Self {
            sample_rate: 44_100,
            intensity: 0.8,
            resonance: 0.55,
        }
    }
}

/// Finite deterministic generator for one impact.
pub struct ImpactGenerator {
    sample_rate: f32,
    intensity: f32,
    resonance: f32,
    position: u32,
    total: u32,
    rng: Rng,
    low_pass: f32,
    phase: f32,
}
impl ImpactGenerator {
    /// Construct one deterministic impact.
    pub fn new(config: ImpactConfig, seed: u64) -> Result<Self, ProceduralError> {
        validate_rate(config.sample_rate)?;
        validate_unit(config.intensity)?;
        validate_unit(config.resonance)?;
        Ok(Self {
            sample_rate: config.sample_rate as f32,
            intensity: config.intensity,
            resonance: config.resonance,
            position: 0,
            total: (config.sample_rate as f32 * 0.38) as u32,
            rng: Rng::new(derive_seed(seed, 0x494D_5041, 0)),
            low_pass: 0.0,
            phase: 0.0,
        })
    }
}
impl ProceduralGenerator for ImpactGenerator {
    fn render(&mut self, output: &mut [f32]) -> Result<(), ProceduralError> {
        validate_output(output)?;
        let (frames, _) = output.as_chunks_mut::<2>();
        for frame in frames {
            if self.position >= self.total {
                frame.fill(0.0);
                continue;
            }
            let t = self.position as f32 / self.sample_rate;
            let noise = self.rng.next_f32() * 2.0 - 1.0;
            self.low_pass += 0.16 * (noise - self.low_pass);
            self.phase =
                (self.phase + (48.0 + self.resonance * 115.0) / self.sample_rate).rem_euclid(1.0);
            let body =
                (self.phase * TAU).sin() * (-t * (8.0 + (1.0 - self.resonance) * 20.0)).exp();
            let transient = self.low_pass * (-t * 65.0).exp();
            let sample =
                (body * self.resonance + transient * (1.0 - self.resonance)) * self.intensity;
            frame[0] = sample.clamp(-1.0, 1.0);
            frame[1] = frame[0];
            self.position += 1;
        }
        Ok(())
    }
    fn is_finished(&self) -> bool {
        self.position >= self.total
    }
}

/// Material profile used to shape a procedural weapon impact.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ImpactMaterial {
    /// Bright, sustained metallic resonance.
    Metal,
    /// Dull, woody knock with a short resonance.
    Wood,
    /// Dense, mid-frequency impact with a firm body.
    Stone,
    /// Soft, low-frequency impact with little ringing.
    Flesh,
    /// Heavy metallic impact with a restrained high-frequency ring.
    Armor,
    /// Resonant shield face with a firm body and a brighter rim response.
    Shield,
}

impl ImpactMaterial {
    fn profile(self) -> (f32, f32, f32, f32, f32) {
        match self {
            Self::Metal => (1_850.0, 1.72, 4.2, 0.92, 260.0),
            Self::Wood => (430.0, 2.08, 13.0, 0.28, 145.0),
            Self::Stone => (760.0, 1.46, 8.0, 0.48, 195.0),
            Self::Flesh => (165.0, 1.24, 22.0, 0.12, 105.0),
            Self::Armor => (1_180.0, 1.83, 5.8, 0.72, 220.0),
            Self::Shield => (940.0, 2.12, 7.4, 0.54, 178.0),
        }
    }
}

/// Character of a weapon contact.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WeaponImpactKind {
    /// A weapon striking a target.
    Strike,
    /// Two blades or metal weapons colliding.
    BladeClash,
    /// A quick, deflecting contact.
    Parry,
    /// A weapon or shield absorbing a blow.
    Block,
    /// A projectile striking a target.
    ProjectileImpact,
}

impl WeaponImpactKind {
    fn profile(self) -> (f32, f32, f32, f32, f32, f32) {
        match self {
            Self::Strike => (0.42, 1.0, 0.78, 0.72, 1.0, 92.0),
            Self::BladeClash => (0.58, 1.25, 1.18, 0.38, 0.68, 118.0),
            Self::Parry => (0.34, 0.95, 1.32, 0.22, 0.82, 125.0),
            Self::Block => (0.46, 0.82, 0.34, 1.15, 1.42, 76.0),
            Self::ProjectileImpact => (0.30, 1.12, 0.66, 0.58, 0.94, 145.0),
        }
    }
}

/// Configuration for one deterministic weapon-contact sound.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct WeaponImpactConfig {
    /// Output sample rate in Hz.
    pub sample_rate: u32,
    /// Overall output level, 0.0..=1.0.
    pub intensity: f32,
    /// Relative contact force, 0.0..=1.0.
    pub force: f32,
    /// Material being struck or contacted, including a shield profile.
    pub material: ImpactMaterial,
    /// Contact character.
    pub kind: WeaponImpactKind,
}

impl Default for WeaponImpactConfig {
    fn default() -> Self {
        Self {
            sample_rate: 44_100,
            intensity: 0.8,
            force: 0.65,
            material: ImpactMaterial::Metal,
            kind: WeaponImpactKind::Strike,
        }
    }
}

/// Finite deterministic generator for weapon strikes, clashes, parries, blocks, and projectile impacts.
///
/// The material and contact kind select a stylized resonant profile. The seed adds repeatable
/// variation to the resonant frequencies and phases. This is a sound-design generator, not a
/// physical collision simulation.
pub struct WeaponImpactGenerator {
    sample_rate: f32,
    intensity: f32,
    force: f32,
    position: u32,
    total: u32,
    rng: Rng,
    noise_state: f32,
    noise_coefficient: f32,
    transient_gain: f32,
    ring_gain: f32,
    body_gain: f32,
    ring_decay: f32,
    transient_decay: f32,
    ring_phase_a: f32,
    ring_phase_b: f32,
    body_phase: f32,
    ring_step_a: f32,
    ring_step_b: f32,
    body_step: f32,
}

impl WeaponImpactGenerator {
    /// Construct one deterministic weapon-contact sound.
    pub fn new(config: WeaponImpactConfig, seed: u64) -> Result<Self, ProceduralError> {
        validate_rate(config.sample_rate)?;
        validate_unit(config.intensity)?;
        validate_unit(config.force)?;

        let (base_frequency, overtone_ratio, material_decay, brightness, body_frequency) =
            config.material.profile();
        let (duration, transient_gain, ring_gain, body_gain, ring_decay_scale, transient_decay) =
            config.kind.profile();
        let mut rng = Rng::new(derive_seed(seed, 0x5745_4150, 0));
        let force_pitch = 1.12 - config.force * 0.20;
        let frequency_a = (base_frequency * force_pitch * (0.94 + rng.next_f32() * 0.12))
            .min(config.sample_rate as f32 * 0.40);
        let frequency_b =
            (base_frequency * overtone_ratio * force_pitch * (0.94 + rng.next_f32() * 0.12))
                .min(config.sample_rate as f32 * 0.40);
        let body_frequency =
            (body_frequency * (1.08 - config.force * 0.16)).min(config.sample_rate as f32 * 0.40);
        let ring_phase_a = rng.next_f32();
        let ring_phase_b = rng.next_f32();
        let body_phase = rng.next_f32();

        Ok(Self {
            sample_rate: config.sample_rate as f32,
            intensity: config.intensity,
            force: config.force,
            position: 0,
            total: (config.sample_rate as f32 * duration) as u32,
            rng,
            noise_state: 0.0,
            noise_coefficient: 0.025 + brightness * 0.68,
            transient_gain,
            ring_gain,
            body_gain,
            ring_decay: material_decay * ring_decay_scale,
            transient_decay,
            ring_phase_a,
            ring_phase_b,
            body_phase,
            ring_step_a: frequency_a / config.sample_rate as f32,
            ring_step_b: frequency_b / config.sample_rate as f32,
            body_step: body_frequency / config.sample_rate as f32,
        })
    }
}

impl ProceduralGenerator for WeaponImpactGenerator {
    fn render(&mut self, output: &mut [f32]) -> Result<(), ProceduralError> {
        validate_output(output)?;
        let (frames, _) = output.as_chunks_mut::<2>();
        let force_gain = 0.55 + self.force * 0.65;

        for frame in frames {
            if self.position >= self.total {
                frame.fill(0.0);
                continue;
            }

            let t = self.position as f32 / self.sample_rate;
            let noise = self.rng.next_f32() * 2.0 - 1.0;
            self.noise_state += self.noise_coefficient * (noise - self.noise_state);
            let transient = (noise - self.noise_state)
                * (-t * self.transient_decay).exp()
                * self.transient_gain;
            let ring = (self.ring_phase_a * TAU).sin() * (-t * self.ring_decay).exp()
                + (self.ring_phase_b * TAU).sin() * (-t * self.ring_decay * 1.28).exp() * 0.42;
            let body = (self.body_phase * TAU).sin() * (-t * (12.0 + self.ring_decay)).exp();

            let sample = (transient + ring * self.ring_gain * 0.46 + body * self.body_gain * 0.34)
                * self.intensity
                * force_gain;
            let sample = sample.clamp(-1.0, 1.0);
            frame[0] = sample;
            frame[1] = sample;

            self.ring_phase_a = (self.ring_phase_a + self.ring_step_a).rem_euclid(1.0);
            self.ring_phase_b = (self.ring_phase_b + self.ring_step_b).rem_euclid(1.0);
            self.body_phase = (self.body_phase + self.body_step).rem_euclid(1.0);
            self.position += 1;
        }

        Ok(())
    }

    fn is_finished(&self) -> bool {
        self.position >= self.total
    }
}

const BREAKAGE_FRAGMENTS: usize = 7;

#[derive(Clone, Copy)]
struct BreakageFragment {
    start_frame: u32,
    duration_frames: u32,
    amplitude: f32,
    phase: f32,
    phase_step: f32,
    decay: f32,
}

impl BreakageFragment {
    fn sample(self, position: u32, sample_rate: f32) -> f32 {
        if position < self.start_frame {
            return 0.0;
        }
        let age_frames = position - self.start_frame;
        if age_frames >= self.duration_frames {
            return 0.0;
        }
        let elapsed = age_frames as f32 / sample_rate;
        let phase = (self.phase + age_frames as f32 * self.phase_step).rem_euclid(1.0);
        self.amplitude * (-elapsed * self.decay).exp() * (TAU * phase).sin()
    }
}

/// Material character for a procedural breakage sound.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BreakageMaterial {
    /// Dry cracks and short, low-pitched splinter sounds.
    Wood,
    /// Bright, brittle fracture with many high-frequency shards.
    Glass,
    /// Dense fracture followed by gritty, low-mid frequency debris.
    Stone,
    /// Brittle, bright fracture with short ceramic fragments.
    Ceramic,
    /// Heavy fracture with a lower metallic body and ringing fragments.
    Metal,
}

impl BreakageMaterial {
    fn profile(self) -> (f32, f32, f32, f32, f32, f32, f32, f32) {
        match self {
            Self::Wood => (0.28, 52.0, 190.0, 1.43, 9.0, 320.0, 900.0, 17.0),
            Self::Glass => (0.98, 76.0, 1_520.0, 1.62, 15.0, 1_900.0, 4_300.0, 25.0),
            Self::Stone => (0.52, 42.0, 390.0, 1.27, 11.0, 460.0, 1_450.0, 21.0),
            Self::Ceramic => (0.85, 68.0, 1_120.0, 1.74, 16.0, 1_550.0, 3_700.0, 26.0),
            Self::Metal => (0.64, 31.0, 780.0, 0.72, 6.0, 720.0, 2_100.0, 14.0),
        }
    }
}

/// Configuration for one deterministic material-breakage sound.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct BreakageConfig {
    /// Output sample rate in Hz.
    pub sample_rate: u32,
    /// Overall output level, 0.0..=1.0.
    pub intensity: f32,
    /// Force of the break, 0.0..=1.0.
    pub force: f32,
    /// Approximate object scale, 0.0..=1.0. Larger values produce a longer, heavier break.
    pub size: f32,
    /// Material being fractured.
    pub material: BreakageMaterial,
}

impl Default for BreakageConfig {
    fn default() -> Self {
        Self {
            sample_rate: 44_100,
            intensity: 0.8,
            force: 0.7,
            size: 0.55,
            material: BreakageMaterial::Wood,
        }
    }
}

/// Finite deterministic generator for material-specific breakage and debris.
///
/// The material selects a stylized fracture and fragment profile. Size affects duration,
/// resonance, and fragment spread; force affects the transient and overall impact energy.
/// Fragment events are prepared at construction and rendered from a fixed-size array, so
/// rendering remains deterministic, bounded, and allocation-free. This is a sound-design
/// generator, not a physical fracture simulation.
pub struct BreakageGenerator {
    sample_rate: f32,
    intensity: f32,
    force: f32,
    position: u32,
    total: u32,
    rng: Rng,
    noise_state: f32,
    noise_coefficient: f32,
    crack_gain: f32,
    crack_decay: f32,
    ring_gain: f32,
    ring_decay: f32,
    ring_phase_a: f32,
    ring_phase_b: f32,
    ring_step_a: f32,
    ring_step_b: f32,
    fragments: [BreakageFragment; BREAKAGE_FRAGMENTS],
}

impl BreakageGenerator {
    /// Construct one deterministic breakage sound with an independent random stream.
    pub fn new(config: BreakageConfig, seed: u64) -> Result<Self, ProceduralError> {
        validate_rate(config.sample_rate)?;
        validate_unit(config.intensity)?;
        validate_unit(config.force)?;
        validate_unit(config.size)?;

        let (
            crack_brightness,
            crack_decay,
            body_frequency,
            overtone_ratio,
            ring_decay,
            fragment_frequency,
            fragment_spread,
            fragment_decay,
        ) = config.material.profile();

        let sample_rate = config.sample_rate as f32;
        let total = (sample_rate * (0.30 + config.size * 0.60)) as u32;
        let mut rng = Rng::new(derive_seed(seed, 0x4252_4541, 0));
        let pitch_scale = (1.18 - config.size * 0.34) * (1.12 - config.force * 0.18);
        let frequency_a =
            (body_frequency * pitch_scale * (0.94 + rng.next_f32() * 0.12)).min(sample_rate * 0.40);
        let frequency_b =
            (body_frequency * overtone_ratio * pitch_scale * (0.94 + rng.next_f32() * 0.12))
                .min(sample_rate * 0.40);
        let ring_phase_a = rng.next_f32();
        let ring_phase_b = rng.next_f32();

        let fragment_window = total as f32 * (0.20 + config.size * 0.48);
        let fragment_duration = (sample_rate * (0.04 + config.size * 0.05)) as u32;
        let fragments: [BreakageFragment; BREAKAGE_FRAGMENTS] = std::array::from_fn(|_| {
            let start_frame = (rng.next_f32() * fragment_window) as u32;
            let frequency =
                (fragment_frequency + rng.next_f32() * fragment_spread).min(sample_rate * 0.40);
            let amplitude = (0.035 + rng.next_f32() * 0.085) * (0.45 + config.size * 0.60);
            let phase = rng.next_f32();
            let decay = fragment_decay * (0.85 + rng.next_f32() * 0.30);
            BreakageFragment {
                start_frame,
                duration_frames: fragment_duration,
                amplitude,
                phase,
                phase_step: frequency / sample_rate,
                decay,
            }
        });

        Ok(Self {
            sample_rate,
            intensity: config.intensity,
            force: config.force,
            position: 0,
            total,
            rng,
            noise_state: 0.0,
            noise_coefficient: 0.02 + crack_brightness * 0.72,
            crack_gain: 0.30 + crack_brightness * 0.42,
            crack_decay,
            ring_gain: 0.34 + config.size * 0.12,
            ring_decay: ring_decay * (1.0 - config.size * 0.16),
            ring_phase_a,
            ring_phase_b,
            ring_step_a: frequency_a / sample_rate,
            ring_step_b: frequency_b / sample_rate,
            fragments,
        })
    }
}

impl ProceduralGenerator for BreakageGenerator {
    fn render(&mut self, output: &mut [f32]) -> Result<(), ProceduralError> {
        validate_output(output)?;
        let (frames, _) = output.as_chunks_mut::<2>();
        let force_gain = 0.45 + self.force * 0.80;

        for frame in frames {
            if self.position >= self.total {
                frame.fill(0.0);
                continue;
            }

            let elapsed = self.position as f32 / self.sample_rate;
            let noise = self.rng.next_f32() * 2.0 - 1.0;
            self.noise_state += self.noise_coefficient * (noise - self.noise_state);
            let crack =
                (noise - self.noise_state) * (-elapsed * self.crack_decay).exp() * self.crack_gain;
            let ring = (self.ring_phase_a * TAU).sin() * (-elapsed * self.ring_decay).exp()
                + (self.ring_phase_b * TAU).sin()
                    * (-elapsed * self.ring_decay * 1.42).exp()
                    * 0.34;
            let fragments: f32 = self
                .fragments
                .iter()
                .map(|fragment| fragment.sample(self.position, self.sample_rate))
                .sum();
            let sample =
                (crack + ring * self.ring_gain * 0.42 + fragments) * self.intensity * force_gain;
            let sample = sample.clamp(-1.0, 1.0);
            frame[0] = sample;
            frame[1] = sample;

            self.ring_phase_a = (self.ring_phase_a + self.ring_step_a).rem_euclid(1.0);
            self.ring_phase_b = (self.ring_phase_b + self.ring_step_b).rem_euclid(1.0);
            self.position += 1;
        }

        Ok(())
    }

    fn is_finished(&self) -> bool {
        self.position >= self.total
    }
}


const CHARACTER_ACTION_IMPACTS: usize = 3;

#[derive(Clone, Copy)]
struct CharacterActionImpact {
    start_frame: u32,
    duration_frames: u32,
    amplitude: f32,
    phase: f32,
    phase_step: f32,
    decay: f32,
}

impl CharacterActionImpact {
    fn sample(self, position: u32, sample_rate: f32) -> f32 {
        if position < self.start_frame {
            return 0.0;
        }
        let age_frames = position - self.start_frame;
        if age_frames >= self.duration_frames {
            return 0.0;
        }
        let elapsed = age_frames as f32 / sample_rate;
        let phase = (self.phase + age_frames as f32 * self.phase_step).rem_euclid(1.0);
        self.amplitude * (-elapsed * self.decay).exp() * (TAU * phase).sin()
    }
}

/// Character movement event represented by a procedural one-shot.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CharacterActionKind {
    /// A brief, rising movement and effort sound as a character jumps.
    Jump,
    /// A weighted body impact and short follow-up movement when landing.
    Landing,
    /// A longer, uneven sequence of movement and secondary impacts during a fall.
    Fall,
}

#[derive(Clone, Copy)]
struct CharacterActionProfile {
    duration: f32,
    noise_decay: f32,
    noise_gain: f32,
    body_start_frequency: f32,
    body_end_frequency: f32,
    body_attack: f32,
    body_decay: f32,
    body_gain: f32,
    secondary_impact_count: usize,
    secondary_impact_gain: f32,
}

impl CharacterActionKind {
    fn profile(self) -> CharacterActionProfile {
        match self {
            Self::Jump => CharacterActionProfile {
                duration: 0.28,
                noise_decay: 34.0,
                noise_gain: 0.62,
                body_start_frequency: 118.0,
                body_end_frequency: 208.0,
                body_attack: 0.018,
                body_decay: 13.0,
                body_gain: 0.24,
                secondary_impact_count: 0,
                secondary_impact_gain: 0.0,
            },
            Self::Landing => CharacterActionProfile {
                duration: 0.36,
                noise_decay: 68.0,
                noise_gain: 0.78,
                body_start_frequency: 92.0,
                body_end_frequency: 52.0,
                body_attack: 0.002,
                body_decay: 15.0,
                body_gain: 0.82,
                secondary_impact_count: 1,
                secondary_impact_gain: 0.72,
            },
            Self::Fall => CharacterActionProfile {
                duration: 0.78,
                noise_decay: 18.0,
                noise_gain: 0.58,
                body_start_frequency: 76.0,
                body_end_frequency: 40.0,
                body_attack: 0.012,
                body_decay: 5.5,
                body_gain: 0.56,
                secondary_impact_count: 3,
                secondary_impact_gain: 0.58,
            },
        }
    }
}

/// Configuration for one deterministic character movement sound.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct CharacterActionConfig {
    /// Output sample rate in Hz.
    pub sample_rate: u32,
    /// Overall output level, 0.0..=1.0.
    pub intensity: f32,
    /// Relative action force, 0.0..=1.0.
    pub force: f32,
    /// Character body weight/scale, 0.0..=1.0.
    pub weight: f32,
    /// Movement event to render.
    pub kind: CharacterActionKind,
}

impl Default for CharacterActionConfig {
    fn default() -> Self {
        Self {
            sample_rate: 44_100,
            intensity: 0.75,
            force: 0.6,
            weight: 0.55,
            kind: CharacterActionKind::Jump,
        }
    }
}

/// Finite deterministic generator for character jumps, landings, and falls.
///
/// These are stylized non-vocal movement sounds. A jump uses a short rising body tone and
/// movement burst; a landing emphasizes a low body impact; a fall adds a longer movement
/// texture and a fixed number of secondary impacts. The seed controls repeatable variation.
/// This generator does not synthesize speech, breathing, or realistic human vocalizations.
pub struct CharacterActionGenerator {
    sample_rate: f32,
    intensity: f32,
    force: f32,
    position: u32,
    total: u32,
    rng: Rng,
    noise_state: f32,
    noise_coefficient: f32,
    noise_decay: f32,
    noise_gain: f32,
    body_phase: f32,
    body_start_frequency: f32,
    body_end_frequency: f32,
    body_attack: f32,
    body_decay: f32,
    body_gain: f32,
    impacts: [CharacterActionImpact; CHARACTER_ACTION_IMPACTS],
}

impl CharacterActionGenerator {
    /// Construct one deterministic movement sound with an independent random stream.
    pub fn new(config: CharacterActionConfig, seed: u64) -> Result<Self, ProceduralError> {
        validate_rate(config.sample_rate)?;
        validate_unit(config.intensity)?;
        validate_unit(config.force)?;
        validate_unit(config.weight)?;

        let profile = config.kind.profile();
        let sample_rate = config.sample_rate as f32;
        let total = (sample_rate * profile.duration) as u32;
        let mut rng = Rng::new(derive_seed(seed, 0x4348_4152, 0));
        let weight_pitch = 1.24 - config.weight * 0.44;
        let body_start_frequency =
            (profile.body_start_frequency * weight_pitch).min(sample_rate * 0.40);
        let body_end_frequency =
            (profile.body_end_frequency * weight_pitch).min(sample_rate * 0.40);
        let body_phase = rng.next_f32();
        let impact_duration = (sample_rate * (0.035 + config.weight * 0.025)) as u32;
        let impact_frequency_scale = 1.45 - config.weight * 0.40;
        let impacts: [CharacterActionImpact; CHARACTER_ACTION_IMPACTS] =
            std::array::from_fn(|index| {
                if index >= profile.secondary_impact_count {
                    return CharacterActionImpact {
                        start_frame: total,
                        duration_frames: 0,
                        amplitude: 0.0,
                        phase: 0.0,
                        phase_step: 0.0,
                        decay: 1.0,
                    };
                }

                let progress =
                    (index + 1) as f32 / (profile.secondary_impact_count + 1) as f32;
                let jitter = (rng.next_f32() - 0.5) * 0.08;
                let start_frame =
                    (total as f32 * (0.12 + progress * 0.72 + jitter)).max(0.0) as u32;
                let frequency = (profile.body_start_frequency
                    * impact_frequency_scale
                    * (0.8 + rng.next_f32() * 0.7))
                    .min(sample_rate * 0.40);
                let amplitude = profile.secondary_impact_gain
                    * (0.06 + config.force * 0.12)
                    * (0.8 + rng.next_f32() * 0.4)
                    * (0.7 + config.weight * 0.65);
                CharacterActionImpact {
                    start_frame,
                    duration_frames: impact_duration,
                    amplitude,
                    phase: rng.next_f32(),
                    phase_step: frequency / sample_rate,
                    decay: 13.0 + rng.next_f32() * 7.0,
                }
            });

        Ok(Self {
            sample_rate,
            intensity: config.intensity,
            force: config.force,
            position: 0,
            total,
            rng,
            noise_state: 0.0,
            noise_coefficient: 0.035 + config.weight * 0.24,
            noise_decay: profile.noise_decay,
            noise_gain: profile.noise_gain,
            body_phase,
            body_start_frequency,
            body_end_frequency,
            body_attack: profile.body_attack,
            body_decay: profile.body_decay,
            body_gain: profile.body_gain * (0.82 + config.weight * 0.36),
            impacts,
        })
    }
}

impl ProceduralGenerator for CharacterActionGenerator {
    fn render(&mut self, output: &mut [f32]) -> Result<(), ProceduralError> {
        validate_output(output)?;
        let (frames, _) = output.as_chunks_mut::<2>();
        let force_gain = 0.55 + self.force * 0.72;

        for frame in frames {
            if self.position >= self.total {
                frame.fill(0.0);
                continue;
            }

            let elapsed = self.position as f32 / self.sample_rate;
            let progress = self.position as f32 / self.total as f32;
            let noise = self.rng.next_f32() * 2.0 - 1.0;
            self.noise_state += self.noise_coefficient * (noise - self.noise_state);
            let movement = (noise - self.noise_state)
                * (-elapsed * self.noise_decay).exp()
                * self.noise_gain;
            let attack = 1.0 - (-elapsed / self.body_attack).exp();
            let body_envelope = attack * (-elapsed * self.body_decay).exp();
            let body_frequency = self.body_start_frequency
                + (self.body_end_frequency - self.body_start_frequency) * progress;
            let body = (self.body_phase * TAU).sin() * body_envelope * self.body_gain;
            let secondary: f32 = self
                .impacts
                .iter()
                .map(|impact| impact.sample(self.position, self.sample_rate))
                .sum();
            let sample =
                (movement + body + secondary) * self.intensity * force_gain;
            let sample = sample.clamp(-1.0, 1.0);
            frame[0] = sample;
            frame[1] = sample;

            self.body_phase =
                (self.body_phase + body_frequency / self.sample_rate).rem_euclid(1.0);
            self.position += 1;
        }

        Ok(())
    }

    fn is_finished(&self) -> bool {
        self.position >= self.total
    }
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
        let rain_samples = render(&mut rain_a, 8192);
        assert_eq!(rain_samples, render(&mut rain_b, 8192));
        bounded(&rain_samples);
        let mut rain_whole = RainGenerator::new(RainConfig::default(), 77).unwrap();
        let mut rain_split = RainGenerator::new(RainConfig::default(), 77).unwrap();
        let expected_rain = render(&mut rain_whole, 4096);
        let mut actual_rain = render(&mut rain_split, 1024);
        actual_rain.extend(render(&mut rain_split, 3072));
        assert_eq!(expected_rain, actual_rain);
        assert!(rain_samples.iter().any(|sample| sample.abs() > 0.001));
    }

    #[test]
    fn finite_effects_are_bounded_and_end_in_silence() {
        let mut step = FootstepGenerator::new(FootstepConfig::default(), 7).unwrap();
        let mut impact = ImpactGenerator::new(ImpactConfig::default(), 8).unwrap();
        let a = render(&mut step, 12000);
        let b = render(&mut impact, 20000);
        bounded(&a);
        bounded(&b);
        assert!(step.is_finished() && impact.is_finished());
        assert!(a[19_404..].iter().all(|s| *s == 0.0));
        assert!(b[33_516..].iter().all(|s| *s == 0.0));
        assert!(a.iter().any(|s| s.abs() > 0.001));
        assert!(b.iter().any(|s| s.abs() > 0.001));
    }

    #[test]
    fn rejects_invalid_configuration_and_odd_output() {
        assert!(matches!(
            WindGenerator::new(
                WindConfig {
                    sample_rate: 1000,
                    ..WindConfig::default()
                },
                0
            ),
            Err(ProceduralError::InvalidSampleRate)
        ));
        assert!(matches!(
            RainGenerator::new(
                RainConfig {
                    density: f32::NAN,
                    ..RainConfig::default()
                },
                0
            ),
            Err(ProceduralError::InvalidParameter)
        ));
        let mut wind = WindGenerator::new(WindConfig::default(), 0).unwrap();
        assert_eq!(
            wind.render(&mut [0.0; 3]),
            Err(ProceduralError::OutputMustContainStereoFrames)
        );
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
        let mut quiet = WindGenerator::new(
            WindConfig {
                intensity: 0.2,
                ..config
            },
            12,
        )
        .unwrap();
        let mut loud = WindGenerator::new(
            WindConfig {
                intensity: 0.9,
                ..config
            },
            12,
        )
        .unwrap();
        assert_ne!(render(&mut quiet, 2048), render(&mut loud, 2048));
    }

    #[test]
    fn weapon_impacts_are_deterministic_across_render_blocks() {
        let config = WeaponImpactConfig::default();
        let mut whole = WeaponImpactGenerator::new(config, 0x51A7).unwrap();
        let mut split = WeaponImpactGenerator::new(config, 0x51A7).unwrap();
        let expected = render(&mut whole, 20_000);
        let mut actual = render(&mut split, 7_000);
        actual.extend(render(&mut split, 13_000));
        assert_eq!(expected, actual);
        bounded(&expected);
        assert!(whole.is_finished());
        let end = (config.sample_rate as usize * 42 / 100) * 2;
        assert!(expected[end..].iter().all(|sample| *sample == 0.0));
    }

    #[test]
    fn weapon_impact_materials_and_contact_kinds_change_the_sound() {
        let base = WeaponImpactConfig::default();
        let mut metal = WeaponImpactGenerator::new(base, 99).unwrap();
        let mut wood = WeaponImpactGenerator::new(
            WeaponImpactConfig {
                material: ImpactMaterial::Wood,
                ..base
            },
            99,
        )
        .unwrap();
        let mut stone = WeaponImpactGenerator::new(
            WeaponImpactConfig {
                material: ImpactMaterial::Stone,
                ..base
            },
            99,
        )
        .unwrap();
        let mut flesh = WeaponImpactGenerator::new(
            WeaponImpactConfig {
                material: ImpactMaterial::Flesh,
                ..base
            },
            99,
        )
        .unwrap();
        let mut armor = WeaponImpactGenerator::new(
            WeaponImpactConfig {
                material: ImpactMaterial::Armor,
                ..base
            },
            99,
        )
        .unwrap();
        let mut shield = WeaponImpactGenerator::new(
            WeaponImpactConfig {
                material: ImpactMaterial::Shield,
                ..base
            },
            99,
        )
        .unwrap();
        let metal_pcm = render(&mut metal, 12_000);
        let wood_pcm = render(&mut wood, 12_000);
        let stone_pcm = render(&mut stone, 12_000);
        let flesh_pcm = render(&mut flesh, 12_000);
        let armor_pcm = render(&mut armor, 12_000);
        let shield_pcm = render(&mut shield, 12_000);
        assert_ne!(metal_pcm, wood_pcm);
        assert_ne!(metal_pcm, stone_pcm);
        assert_ne!(metal_pcm, flesh_pcm);
        assert_ne!(metal_pcm, armor_pcm);
        assert_ne!(metal_pcm, shield_pcm);
        assert_ne!(armor_pcm, shield_pcm);

        let mut strike = WeaponImpactGenerator::new(base, 101).unwrap();
        let mut clash = WeaponImpactGenerator::new(
            WeaponImpactConfig {
                kind: WeaponImpactKind::BladeClash,
                ..base
            },
            101,
        )
        .unwrap();
        let mut parry = WeaponImpactGenerator::new(
            WeaponImpactConfig {
                kind: WeaponImpactKind::Parry,
                ..base
            },
            101,
        )
        .unwrap();
        let mut block = WeaponImpactGenerator::new(
            WeaponImpactConfig {
                kind: WeaponImpactKind::Block,
                ..base
            },
            101,
        )
        .unwrap();
        assert_ne!(render(&mut strike, 12_000), render(&mut clash, 12_000));
        assert_ne!(render(&mut strike, 12_000), render(&mut parry, 12_000));
        assert_ne!(render(&mut strike, 12_000), render(&mut block, 12_000));
        let mut projectile = WeaponImpactGenerator::new(
            WeaponImpactConfig {
                kind: WeaponImpactKind::ProjectileImpact,
                ..base
            },
            101,
        )
        .unwrap();
        assert_ne!(render(&mut strike, 12_000), render(&mut projectile, 12_000));
    }

    #[test]
    fn weapon_impact_seed_and_controls_are_repeatable_and_effective() {
        let config = WeaponImpactConfig::default();
        let mut first = WeaponImpactGenerator::new(config, 7).unwrap();
        let mut same = WeaponImpactGenerator::new(config, 7).unwrap();
        let mut other_seed = WeaponImpactGenerator::new(config, 8).unwrap();
        let first_pcm = render(&mut first, 8_000);
        assert_eq!(first_pcm, render(&mut same, 8_000));
        assert_ne!(first_pcm, render(&mut other_seed, 8_000));

        let mut light = WeaponImpactGenerator::new(
            WeaponImpactConfig {
                force: 0.1,
                ..config
            },
            7,
        )
        .unwrap();
        let mut heavy = WeaponImpactGenerator::new(
            WeaponImpactConfig {
                force: 0.95,
                ..config
            },
            7,
        )
        .unwrap();
        assert_ne!(render(&mut light, 8_000), render(&mut heavy, 8_000));

        let mut quiet = WeaponImpactGenerator::new(
            WeaponImpactConfig {
                intensity: 0.2,
                ..config
            },
            7,
        )
        .unwrap();
        let mut loud = WeaponImpactGenerator::new(
            WeaponImpactConfig {
                intensity: 0.9,
                ..config
            },
            7,
        )
        .unwrap();
        assert_ne!(render(&mut quiet, 8_000), render(&mut loud, 8_000));
    }

    #[test]
    fn weapon_impact_rejects_invalid_configuration() {
        assert!(matches!(
            WeaponImpactGenerator::new(
                WeaponImpactConfig {
                    intensity: f32::NAN,
                    ..WeaponImpactConfig::default()
                },
                0
            ),
            Err(ProceduralError::InvalidParameter)
        ));
        assert!(matches!(
            WeaponImpactGenerator::new(
                WeaponImpactConfig {
                    sample_rate: 384_000,
                    ..WeaponImpactConfig::default()
                },
                0
            ),
            Err(ProceduralError::InvalidSampleRate)
        ));
        let mut impact = WeaponImpactGenerator::new(WeaponImpactConfig::default(), 0).unwrap();
        assert_eq!(
            impact.render(&mut [0.0; 3]),
            Err(ProceduralError::OutputMustContainStereoFrames)
        );
    }

    #[test]
    fn breakage_is_deterministic_across_blocks_and_ends_in_silence() {
        let config = BreakageConfig::default();
        let mut whole = BreakageGenerator::new(config, 0xBEEF).unwrap();
        let mut split = BreakageGenerator::new(config, 0xBEEF).unwrap();
        let expected = render(&mut whole, 35_000);
        let mut actual = render(&mut split, 12_345);
        actual.extend(render(&mut split, 22_655));
        assert_eq!(expected, actual);
        bounded(&expected);
        assert!(whole.is_finished());

        let total_frames = (config.sample_rate as f32 * (0.30 + config.size * 0.60)) as usize;
        assert!(expected[total_frames * 2..]
            .iter()
            .all(|sample| *sample == 0.0));
        assert!(expected.iter().any(|sample| sample.abs() > 0.001));
    }

    #[test]
    fn breakage_materials_and_controls_change_the_sound() {
        let base = BreakageConfig::default();
        let mut wood = BreakageGenerator::new(base, 91).unwrap();
        let mut glass = BreakageGenerator::new(
            BreakageConfig {
                material: BreakageMaterial::Glass,
                ..base
            },
            91,
        )
        .unwrap();
        let mut stone = BreakageGenerator::new(
            BreakageConfig {
                material: BreakageMaterial::Stone,
                ..base
            },
            91,
        )
        .unwrap();
        let mut ceramic = BreakageGenerator::new(
            BreakageConfig {
                material: BreakageMaterial::Ceramic,
                ..base
            },
            91,
        )
        .unwrap();
        let mut metal = BreakageGenerator::new(
            BreakageConfig {
                material: BreakageMaterial::Metal,
                ..base
            },
            91,
        )
        .unwrap();
        let wood_pcm = render(&mut wood, 16_000);
        assert_ne!(wood_pcm, render(&mut glass, 16_000));
        assert_ne!(wood_pcm, render(&mut stone, 16_000));
        assert_ne!(wood_pcm, render(&mut ceramic, 16_000));
        assert_ne!(wood_pcm, render(&mut metal, 16_000));

        let mut other_seed = BreakageGenerator::new(base, 92).unwrap();
        assert_ne!(wood_pcm, render(&mut other_seed, 16_000));

        let mut small = BreakageGenerator::new(BreakageConfig { size: 0.15, ..base }, 91).unwrap();
        let mut large = BreakageGenerator::new(BreakageConfig { size: 0.95, ..base }, 91).unwrap();
        assert_ne!(wood_pcm, render(&mut small, 16_000));
        assert_ne!(wood_pcm, render(&mut large, 16_000));

        let mut light = BreakageGenerator::new(BreakageConfig { force: 0.1, ..base }, 91).unwrap();
        let mut heavy = BreakageGenerator::new(
            BreakageConfig {
                force: 0.95,
                ..base
            },
            91,
        )
        .unwrap();
        assert_ne!(wood_pcm, render(&mut light, 16_000));
        assert_ne!(wood_pcm, render(&mut heavy, 16_000));

        let mut quiet = BreakageGenerator::new(
            BreakageConfig {
                intensity: 0.2,
                ..base
            },
            91,
        )
        .unwrap();
        assert_ne!(wood_pcm, render(&mut quiet, 16_000));
    }

    #[test]
    fn breakage_rejects_invalid_configuration_and_odd_output() {
        assert!(matches!(
            BreakageGenerator::new(
                BreakageConfig {
                    sample_rate: 1_000,
                    ..BreakageConfig::default()
                },
                0
            ),
            Err(ProceduralError::InvalidSampleRate)
        ));
        assert!(matches!(
            BreakageGenerator::new(
                BreakageConfig {
                    size: f32::NAN,
                    ..BreakageConfig::default()
                },
                0
            ),
            Err(ProceduralError::InvalidParameter)
        ));
        assert!(matches!(
            BreakageGenerator::new(
                BreakageConfig {
                    intensity: f32::NAN,
                    ..BreakageConfig::default()
                },
                0
            ),
            Err(ProceduralError::InvalidParameter)
        ));
        assert!(matches!(
            BreakageGenerator::new(
                BreakageConfig {
                    force: 1.1,
                    ..BreakageConfig::default()
                },
                0
            ),
            Err(ProceduralError::InvalidParameter)
        ));
        let mut generator = BreakageGenerator::new(BreakageConfig::default(), 0).unwrap();
        assert_eq!(
            generator.render(&mut [0.0; 3]),
            Err(ProceduralError::OutputMustContainStereoFrames)
        );
    }

    #[test]
    fn character_actions_are_deterministic_across_blocks_and_finish_cleanly() {
        let config = CharacterActionConfig {
            kind: CharacterActionKind::Fall,
            ..CharacterActionConfig::default()
        };
        let mut whole = CharacterActionGenerator::new(config, 0xCAFE).unwrap();
        let mut split = CharacterActionGenerator::new(config, 0xCAFE).unwrap();
        assert!(!whole.is_finished());
        let expected = render(&mut whole, 40_000);
        let mut actual = render(&mut split, 17_321);
        actual.extend(render(&mut split, 22_679));
        assert_eq!(expected, actual);
        bounded(&expected);
        assert!(whole.is_finished());

        let total_frames = (config.sample_rate as f32 * 0.78) as usize;
        assert!(expected[total_frames * 2..]
            .iter()
            .all(|sample| *sample == 0.0));
        assert!(expected.iter().any(|sample| sample.abs() > 0.001));
    }

    #[test]
    fn character_action_kinds_and_controls_change_the_sound() {
        let base = CharacterActionConfig::default();
        let mut jump = CharacterActionGenerator::new(base, 17).unwrap();
        let mut landing = CharacterActionGenerator::new(
            CharacterActionConfig {
                kind: CharacterActionKind::Landing,
                ..base
            },
            17,
        )
        .unwrap();
        let mut fall = CharacterActionGenerator::new(
            CharacterActionConfig {
                kind: CharacterActionKind::Fall,
                ..base
            },
            17,
        )
        .unwrap();
        assert!(!jump.is_finished() && !landing.is_finished() && !fall.is_finished());
        let jump_pcm = render(&mut jump, 40_000);
        let landing_pcm = render(&mut landing, 40_000);
        let fall_pcm = render(&mut fall, 40_000);
        bounded(&jump_pcm);
        bounded(&landing_pcm);
        bounded(&fall_pcm);
        assert!(jump.is_finished() && landing.is_finished() && fall.is_finished());
        let jump_end = (base.sample_rate as f32 * 0.28) as usize * 2;
        let landing_end = (base.sample_rate as f32 * 0.36) as usize * 2;
        let fall_end = (base.sample_rate as f32 * 0.78) as usize * 2;
        assert!(jump_pcm[jump_end..].iter().all(|sample| *sample == 0.0));
        assert!(landing_pcm[landing_end..].iter().all(|sample| *sample == 0.0));
        assert!(fall_pcm[fall_end..].iter().all(|sample| *sample == 0.0));
        assert_ne!(jump_pcm, landing_pcm);
        assert_ne!(jump_pcm, fall_pcm);

        let mut other_seed = CharacterActionGenerator::new(base, 18).unwrap();
        assert_ne!(jump_pcm, render(&mut other_seed, 40_000));

        let mut light = CharacterActionGenerator::new(
            CharacterActionConfig {
                force: 0.1,
                ..base
            },
            17,
        )
        .unwrap();
        let mut heavy = CharacterActionGenerator::new(
            CharacterActionConfig {
                force: 0.95,
                ..base
            },
            17,
        )
        .unwrap();
        assert_ne!(jump_pcm, render(&mut light, 40_000));
        assert_ne!(jump_pcm, render(&mut heavy, 40_000));

        let mut small = CharacterActionGenerator::new(
            CharacterActionConfig {
                weight: 0.1,
                ..base
            },
            17,
        )
        .unwrap();
        let mut large = CharacterActionGenerator::new(
            CharacterActionConfig {
                weight: 0.95,
                ..base
            },
            17,
        )
        .unwrap();
        assert_ne!(jump_pcm, render(&mut small, 40_000));
        assert_ne!(jump_pcm, render(&mut large, 40_000));

        let mut quiet = CharacterActionGenerator::new(
            CharacterActionConfig {
                intensity: 0.2,
                ..base
            },
            17,
        )
        .unwrap();
        assert_ne!(jump_pcm, render(&mut quiet, 40_000));
    }

    #[test]
    fn character_actions_reject_invalid_configuration_and_odd_output() {
        assert!(matches!(
            CharacterActionGenerator::new(
                CharacterActionConfig {
                    sample_rate: 1_000,
                    ..CharacterActionConfig::default()
                },
                0
            ),
            Err(ProceduralError::InvalidSampleRate)
        ));
        assert!(matches!(
            CharacterActionGenerator::new(
                CharacterActionConfig {
                    intensity: f32::NAN,
                    ..CharacterActionConfig::default()
                },
                0
            ),
            Err(ProceduralError::InvalidParameter)
        ));
        assert!(matches!(
            CharacterActionGenerator::new(
                CharacterActionConfig {
                    force: 1.1,
                    ..CharacterActionConfig::default()
                },
                0
            ),
            Err(ProceduralError::InvalidParameter)
        ));
        assert!(matches!(
            CharacterActionGenerator::new(
                CharacterActionConfig {
                    weight: -0.1,
                    ..CharacterActionConfig::default()
                },
                0
            ),
            Err(ProceduralError::InvalidParameter)
        ));
        let mut generator =
            CharacterActionGenerator::new(CharacterActionConfig::default(), 0).unwrap();
        assert_eq!(
            generator.render(&mut [0.0; 3]),
            Err(ProceduralError::OutputMustContainStereoFrames)
        );
    }

}
