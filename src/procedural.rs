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

/// Surface profile used by the material-aware footstep generator.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FootstepSurface {
    /// Soft soil with a muted body and dry granular contact.
    Dirt,
    /// Light, damped contact with a soft rustling texture.
    Grass,
    /// Loose stones with a coarse, uneven crunch.
    Gravel,
    /// Hollow, woody contact with a short body resonance.
    Wood,
    /// Firm, dense contact with a compact knock.
    Stone,
    /// Bright, hard contact with a short metallic ring.
    Metal,
    /// Wet contact with a splash-like noise component.
    ShallowWater,
    /// Soft, compressed snow with a muffled crunch.
    Snow,
}

impl FootstepSurface {
    fn profile(self) -> FootstepSurfaceProfile {
        match self {
            Self::Dirt => FootstepSurfaceProfile {
                body_frequency: 76.0,
                body_decay: 22.0,
                body_gain: 0.64,
                noise_coefficient: 0.12,
                texture_gain: 0.18,
                texture_decay: 40.0,
                transient_gain: 0.30,
                transient_decay: 58.0,
            },
            Self::Grass => FootstepSurfaceProfile {
                body_frequency: 94.0,
                body_decay: 24.0,
                body_gain: 0.42,
                noise_coefficient: 0.22,
                texture_gain: 0.24,
                texture_decay: 38.0,
                transient_gain: 0.20,
                transient_decay: 45.0,
            },
            Self::Gravel => FootstepSurfaceProfile {
                body_frequency: 112.0,
                body_decay: 21.0,
                body_gain: 0.52,
                noise_coefficient: 0.48,
                texture_gain: 0.42,
                texture_decay: 32.0,
                transient_gain: 0.46,
                transient_decay: 62.0,
            },
            Self::Wood => FootstepSurfaceProfile {
                body_frequency: 88.0,
                body_decay: 21.0,
                body_gain: 0.68,
                noise_coefficient: 0.10,
                texture_gain: 0.12,
                texture_decay: 44.0,
                transient_gain: 0.26,
                transient_decay: 68.0,
            },
            Self::Stone => FootstepSurfaceProfile {
                body_frequency: 126.0,
                body_decay: 24.0,
                body_gain: 0.62,
                noise_coefficient: 0.16,
                texture_gain: 0.10,
                texture_decay: 52.0,
                transient_gain: 0.43,
                transient_decay: 74.0,
            },
            Self::Metal => FootstepSurfaceProfile {
                body_frequency: 184.0,
                body_decay: 11.0,
                body_gain: 0.40,
                noise_coefficient: 0.08,
                texture_gain: 0.05,
                texture_decay: 44.0,
                transient_gain: 0.54,
                transient_decay: 72.0,
            },
            Self::ShallowWater => FootstepSurfaceProfile {
                body_frequency: 64.0,
                body_decay: 18.0,
                body_gain: 0.48,
                noise_coefficient: 0.62,
                texture_gain: 0.30,
                texture_decay: 27.0,
                transient_gain: 0.65,
                transient_decay: 52.0,
            },
            Self::Snow => FootstepSurfaceProfile {
                body_frequency: 106.0,
                body_decay: 27.0,
                body_gain: 0.46,
                noise_coefficient: 0.40,
                texture_gain: 0.38,
                texture_decay: 30.0,
                transient_gain: 0.30,
                transient_decay: 52.0,
            },
        }
    }

    fn stream_index(self) -> u64 {
        match self {
            Self::Dirt => 0,
            Self::Grass => 1,
            Self::Gravel => 2,
            Self::Wood => 3,
            Self::Stone => 4,
            Self::Metal => 5,
            Self::ShallowWater => 6,
            Self::Snow => 7,
        }
    }
}

#[derive(Clone, Copy)]
struct FootstepSurfaceProfile {
    body_frequency: f32,
    body_decay: f32,
    body_gain: f32,
    noise_coefficient: f32,
    texture_gain: f32,
    texture_decay: f32,
    transient_gain: f32,
    transient_decay: f32,
}

/// Finite deterministic generator for one footstep.
///
/// The regular constructor retains the original generic footstep profile. Use new_for_surface
/// when the host knows which ground material the character is crossing.
pub struct FootstepGenerator {
    sample_rate: f32,
    intensity: f32,
    brightness: f32,
    position: u32,
    total: u32,
    rng: Rng,
    low_pass: f32,
    surface_profile: Option<FootstepSurfaceProfile>,
}
impl FootstepGenerator {
    /// Construct one generic deterministic footstep.
    ///
    /// This constructor retains the original material-neutral sound for compatibility.
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
            surface_profile: None,
        })
    }

    /// Construct one deterministic footstep shaped for a known ground surface.
    ///
    /// The surface selects a stylized mix of body resonance, friction, and contact transient.
    /// Brightness adjusts the high-frequency texture. The seed controls repeatable variation
    /// using a stream independent from the generic footstep constructor.
    pub fn new_for_surface(
        config: FootstepConfig,
        surface: FootstepSurface,
        seed: u64,
    ) -> Result<Self, ProceduralError> {
        let mut generator = Self::new(config, seed)?;
        let mut profile = surface.profile();
        let mut rng = Rng::new(derive_seed(seed, 0x5355_5246, surface.stream_index()));
        profile.body_frequency *= 0.94 + rng.next_f32() * 0.12;
        generator.rng = Rng::new(derive_seed(seed, 0x5354_4550, surface.stream_index() + 1));
        generator.surface_profile = Some(profile);
        Ok(generator)
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
            let sample = if let Some(profile) = self.surface_profile {
                self.low_pass += profile.noise_coefficient
                    * (0.70 + self.brightness * 0.60)
                    * (noise - self.low_pass);
                let progress = self.position as f32 / self.total as f32;
                let body = (TAU * profile.body_frequency * t).sin()
                    * (-t * profile.body_decay).exp()
                    * profile.body_gain;
                let texture = self.low_pass
                    * (-t * profile.texture_decay).exp()
                    * profile.texture_gain
                    * (0.75 + self.brightness * 0.50);
                let transient = (noise - self.low_pass)
                    * (-t * profile.transient_decay).exp()
                    * profile.transient_gain
                    * (0.65 + self.brightness * 0.70);
                (body + texture + transient) * self.intensity * (1.0 - progress).max(0.0)
            } else {
                self.low_pass += (0.06 + self.brightness * 0.72) * (noise - self.low_pass);
                let body =
                    (TAU * (62.0 - 18.0 * (t * 12.0).min(1.0)) * t).sin() * (-t * 23.0).exp();
                (body * 0.62 + self.low_pass * (-t * 42.0).exp() * 0.38)
                    * self.intensity
                    * (1.0 - t / 0.22).max(0.0)
            };
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

                let progress = (index + 1) as f32 / (profile.secondary_impact_count + 1) as f32;
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
            let movement =
                (noise - self.noise_state) * (-elapsed * self.noise_decay).exp() * self.noise_gain;
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
            let sample = (movement + body + secondary) * self.intensity * force_gain;
            let sample = sample.clamp(-1.0, 1.0);
            frame[0] = sample;
            frame[1] = sample;

            self.body_phase = (self.body_phase + body_frequency / self.sample_rate).rem_euclid(1.0);
            self.position += 1;
        }

        Ok(())
    }

    fn is_finished(&self) -> bool {
        self.position >= self.total
    }
}

const WEAPON_HANDLING_EVENTS: usize = 4;

#[derive(Clone, Copy)]
struct WeaponHandlingEvent {
    start_frame: u32,
    duration_frames: u32,
    amplitude: f32,
    phase: f32,
    phase_step: f32,
    decay: f32,
}

impl WeaponHandlingEvent {
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

/// Dominant material character for a weapon-handling sound.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WeaponHandlingMaterial {
    /// Bright metal fittings and mechanical resonance.
    Metal,
    /// Dry wooden contact and a muted body.
    Wood,
    /// Soft friction and subdued fittings, suitable for leather-bound gear.
    Leather,
}

impl WeaponHandlingMaterial {
    fn profile(self) -> (f32, f32, f32, f32) {
        match self {
            Self::Metal => (0.045, 1_360.0, 4.8, 0.48),
            Self::Wood => (0.11, 430.0, 10.5, 0.36),
            Self::Leather => (0.18, 245.0, 14.0, 0.29),
        }
    }
}

/// Weapon interaction represented by a procedural handling sound.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WeaponHandlingKind {
    /// Drawing or unsheathing a weapon, with movement and a release fitting sound.
    Draw,
    /// Returning a weapon to its scabbard or holder, with a closing contact.
    Sheathe,
    /// A short sequence of generic weapon-mechanism handling clicks.
    Reload,
}

impl WeaponHandlingKind {
    fn profile(self) -> (f32, f32, f32, f32, f32, usize, f32) {
        match self {
            Self::Draw => (0.52, 0.62, 0.34, 0.009, 4.6, 2, 0.42),
            Self::Sheathe => (0.48, 0.52, 0.30, 0.011, 5.0, 2, 0.38),
            Self::Reload => (0.72, 0.42, 0.25, 0.007, 4.0, 4, 0.34),
        }
    }

    fn event_positions(self) -> [f32; WEAPON_HANDLING_EVENTS] {
        match self {
            Self::Draw => [0.08, 0.88, 0.0, 0.0],
            Self::Sheathe => [0.14, 0.88, 0.0, 0.0],
            Self::Reload => [0.10, 0.36, 0.66, 0.90],
        }
    }
}

/// Configuration for one deterministic weapon-handling sound.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct WeaponHandlingConfig {
    /// Output sample rate in Hz.
    pub sample_rate: u32,
    /// Overall output level, 0.0..=1.0.
    pub intensity: f32,
    /// Relative weapon heft, 0.0..=1.0. Higher values lower the resonant body and add weight.
    pub weight: f32,
    /// Handling speed, 0.0..=1.0. Higher values shorten the action and tighten its movement.
    pub speed: f32,
    /// Dominant material character of the weapon and its fittings.
    pub material: WeaponHandlingMaterial,
    /// Handling action to render.
    pub kind: WeaponHandlingKind,
}

impl Default for WeaponHandlingConfig {
    fn default() -> Self {
        Self {
            sample_rate: 44_100,
            intensity: 0.8,
            weight: 0.55,
            speed: 0.55,
            material: WeaponHandlingMaterial::Metal,
            kind: WeaponHandlingKind::Draw,
        }
    }
}

/// Finite deterministic generator for weapon drawing, sheathing, and generic reload handling.
///
/// The material selects the friction and resonance character, while weight and speed alter the
/// resonant body and action timing. Small contact events are prepared from the seed at
/// construction and stored in a fixed-size array. Rendering is bounded and allocation-free.
/// These are stylized generic handling effects, not recordings or simulations of a particular
/// weapon mechanism.
pub struct WeaponHandlingGenerator {
    sample_rate: f32,
    intensity: f32,
    position: u32,
    total: u32,
    rng: Rng,
    noise_state: f32,
    noise_coefficient: f32,
    movement_gain: f32,
    body_phase: f32,
    body_start_frequency: f32,
    body_end_frequency: f32,
    body_attack: f32,
    body_decay: f32,
    body_gain: f32,
    events: [WeaponHandlingEvent; WEAPON_HANDLING_EVENTS],
}

impl WeaponHandlingGenerator {
    /// Construct one deterministic weapon-handling sound with an independent random stream.
    pub fn new(config: WeaponHandlingConfig, seed: u64) -> Result<Self, ProceduralError> {
        validate_rate(config.sample_rate)?;
        validate_unit(config.intensity)?;
        validate_unit(config.weight)?;
        validate_unit(config.speed)?;

        let (noise_coefficient, material_frequency, material_decay, material_texture) =
            config.material.profile();
        let (
            base_duration,
            movement_gain,
            body_gain,
            body_attack,
            body_decay,
            event_count,
            event_gain,
        ) = config.kind.profile();

        let sample_rate = config.sample_rate as f32;
        let duration_scale = 1.30 - config.speed * 0.60;
        let total = (sample_rate * base_duration * duration_scale) as u32;
        let mut rng = Rng::new(derive_seed(seed, 0x5748_4E44, 0));
        let weight_pitch = 1.25 - config.weight * 0.48;
        let speed_pitch = 0.94 + config.speed * 0.12;
        let body_start_frequency =
            (material_frequency * weight_pitch * speed_pitch).min(sample_rate * 0.40);
        let body_end_frequency =
            (body_start_frequency * (0.78 + config.speed * 0.16)).min(sample_rate * 0.40);
        let body_phase = rng.next_f32();

        let event_duration = (sample_rate * (0.020 + (1.0 - config.speed) * 0.018)) as u32;
        let event_positions = config.kind.event_positions();
        let events: [WeaponHandlingEvent; WEAPON_HANDLING_EVENTS] = std::array::from_fn(|index| {
            if index >= event_count {
                return WeaponHandlingEvent {
                    start_frame: total,
                    duration_frames: 0,
                    amplitude: 0.0,
                    phase: 0.0,
                    phase_step: 0.0,
                    decay: 1.0,
                };
            }

            let jitter = (rng.next_f32() - 0.5) * 0.025;
            let progress = (event_positions[index] + jitter).clamp(0.025, 0.975);
            let start_frame = (total as f32 * progress) as u32;
            let frequency = (material_frequency * (0.72 + rng.next_f32() * 0.88) * weight_pitch)
                .min(sample_rate * 0.40);
            let amplitude = event_gain
                * material_texture
                * (0.68 + rng.next_f32() * 0.64)
                * (0.82 + config.weight * 0.36);
            WeaponHandlingEvent {
                start_frame,
                duration_frames: event_duration,
                amplitude,
                phase: rng.next_f32(),
                phase_step: frequency / sample_rate,
                decay: material_decay * (0.78 + rng.next_f32() * 0.44),
            }
        });

        Ok(Self {
            sample_rate,
            intensity: config.intensity,
            position: 0,
            total,
            rng,
            noise_state: 0.0,
            noise_coefficient,
            movement_gain: movement_gain * material_texture * (0.78 + config.speed * 0.38),
            body_phase,
            body_start_frequency,
            body_end_frequency,
            body_attack,
            body_decay: body_decay * (1.08 - config.weight * 0.18),
            body_gain: body_gain * (0.78 + config.weight * 0.40),
            events,
        })
    }
}

impl ProceduralGenerator for WeaponHandlingGenerator {
    fn render(&mut self, output: &mut [f32]) -> Result<(), ProceduralError> {
        validate_output(output)?;
        let (frames, _) = output.as_chunks_mut::<2>();

        for frame in frames {
            if self.position >= self.total {
                frame.fill(0.0);
                continue;
            }

            let elapsed = self.position as f32 / self.sample_rate;
            let progress = self.position as f32 / self.total as f32;
            let noise = self.rng.next_f32() * 2.0 - 1.0;
            self.noise_state += self.noise_coefficient * (noise - self.noise_state);
            let movement_envelope = (TAU * progress * 0.5).sin();
            let movement = (noise - self.noise_state) * movement_envelope * self.movement_gain;
            let body_attack = 1.0 - (-elapsed / self.body_attack).exp();
            let body_envelope = body_attack * (-elapsed * self.body_decay).exp();
            let body_frequency = self.body_start_frequency
                + (self.body_end_frequency - self.body_start_frequency) * progress;
            let body = (self.body_phase * TAU).sin() * body_envelope * self.body_gain;
            let contacts: f32 = self
                .events
                .iter()
                .map(|event| event.sample(self.position, self.sample_rate))
                .sum();
            let sample = (movement + body + contacts) * self.intensity;
            let sample = sample.clamp(-1.0, 1.0);
            frame[0] = sample;
            frame[1] = sample;

            self.body_phase = (self.body_phase + body_frequency / self.sample_rate).rem_euclid(1.0);
            self.position += 1;
        }

        Ok(())
    }

    fn is_finished(&self) -> bool {
        self.position >= self.total
    }
}

const EQUIPMENT_MOVEMENT_EVENTS: usize = 5;

#[derive(Clone, Copy)]
struct EquipmentMovementEvent {
    start_frame: u32,
    duration_frames: u32,
    amplitude: f32,
    phase: f32,
    phase_step: f32,
    decay: f32,
}

impl EquipmentMovementEvent {
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

/// Kind of equipment movement represented by a procedural one-shot.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EquipmentMovementKind {
    /// Soft cloth and fabric shifting with little hard contact.
    ClothRustle,
    /// Several restrained contacts from carried pouches, straps, and attached gear.
    GearJostle,
    /// A brighter sequence of contacts from linked or plate armor.
    ArmorRattle,
}

#[derive(Clone, Copy)]
struct EquipmentMovementProfile {
    duration: f32,
    noise_gain: f32,
    noise_coefficient: f32,
    body_start_frequency: f32,
    body_end_frequency: f32,
    body_attack: f32,
    body_decay: f32,
    body_gain: f32,
    event_count: usize,
    event_gain: f32,
}

impl EquipmentMovementKind {
    fn profile(self) -> EquipmentMovementProfile {
        match self {
            Self::ClothRustle => EquipmentMovementProfile {
                duration: 0.42,
                noise_gain: 0.70,
                noise_coefficient: 0.44,
                body_start_frequency: 205.0,
                body_end_frequency: 158.0,
                body_attack: 0.014,
                body_decay: 8.5,
                body_gain: 0.10,
                event_count: 0,
                event_gain: 0.0,
            },
            Self::GearJostle => EquipmentMovementProfile {
                duration: 0.40,
                noise_gain: 0.44,
                noise_coefficient: 0.30,
                body_start_frequency: 154.0,
                body_end_frequency: 118.0,
                body_attack: 0.008,
                body_decay: 9.5,
                body_gain: 0.23,
                event_count: 3,
                event_gain: 0.25,
            },
            Self::ArmorRattle => EquipmentMovementProfile {
                duration: 0.46,
                noise_gain: 0.30,
                noise_coefficient: 0.16,
                body_start_frequency: 238.0,
                body_end_frequency: 188.0,
                body_attack: 0.004,
                body_decay: 7.0,
                body_gain: 0.34,
                event_count: 5,
                event_gain: 0.31,
            },
        }
    }

    fn event_positions(self) -> [f32; EQUIPMENT_MOVEMENT_EVENTS] {
        match self {
            Self::ClothRustle => [0.0; EQUIPMENT_MOVEMENT_EVENTS],
            Self::GearJostle => [0.12, 0.48, 0.86, 0.0, 0.0],
            Self::ArmorRattle => [0.08, 0.27, 0.49, 0.71, 0.91],
        }
    }

    fn stream_index(self) -> u64 {
        match self {
            Self::ClothRustle => 0,
            Self::GearJostle => 1,
            Self::ArmorRattle => 2,
        }
    }
}

/// Configuration for one deterministic equipment-movement sound.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct EquipmentMovementConfig {
    /// Output sample rate in Hz.
    pub sample_rate: u32,
    /// Overall output level, 0.0..=1.0.
    pub intensity: f32,
    /// Relative equipment weight, 0.0..=1.0. Higher values lower its resonant body.
    pub weight: f32,
    /// Movement vigor, 0.0..=1.0. Higher values increase texture and contact energy.
    pub activity: f32,
    /// Movement sound to render.
    pub kind: EquipmentMovementKind,
}

impl Default for EquipmentMovementConfig {
    fn default() -> Self {
        Self {
            sample_rate: 44_100,
            intensity: 0.72,
            weight: 0.55,
            activity: 0.55,
            kind: EquipmentMovementKind::GearJostle,
        }
    }
}

/// Finite deterministic generator for cloth rustle, carried-gear movement, and armor rattle.
///
/// The selected profile changes the balance between friction-like movement and hard contacts.
/// Weight shifts the resonant body; activity controls movement energy and duration. Contact
/// events are prepared at construction in a fixed-capacity array, using a random stream
/// independent from the rendered noise. Output is stylized rather than a physical simulation.
pub struct EquipmentMovementGenerator {
    sample_rate: f32,
    intensity: f32,
    position: u32,
    total: u32,
    rng: Rng,
    noise_state: f32,
    noise_gain: f32,
    noise_coefficient: f32,
    body_phase: f32,
    body_start_frequency: f32,
    body_end_frequency: f32,
    body_attack: f32,
    body_decay: f32,
    body_gain: f32,
    events: [EquipmentMovementEvent; EQUIPMENT_MOVEMENT_EVENTS],
}

impl EquipmentMovementGenerator {
    /// Construct one deterministic equipment-movement sound with an independent random stream.
    pub fn new(config: EquipmentMovementConfig, seed: u64) -> Result<Self, ProceduralError> {
        validate_rate(config.sample_rate)?;
        validate_unit(config.intensity)?;
        validate_unit(config.weight)?;
        validate_unit(config.activity)?;

        let profile = config.kind.profile();
        let sample_rate = config.sample_rate as f32;
        let duration_scale = 1.18 - config.activity * 0.28;
        let total = (sample_rate * profile.duration * duration_scale) as u32;
        let weight_pitch = 1.22 - config.weight * 0.42;
        let body_start_frequency =
            (profile.body_start_frequency * weight_pitch).min(sample_rate * 0.40);
        let body_end_frequency =
            (profile.body_end_frequency * weight_pitch).min(sample_rate * 0.40);

        let stream_index = config.kind.stream_index();
        let noise_rng = Rng::new(derive_seed(seed, 0x4551_4E4F, stream_index));
        let mut phase_rng = Rng::new(derive_seed(seed, 0x4551_5048, stream_index));
        let mut event_rng = Rng::new(derive_seed(seed, 0x4551_4556, stream_index));
        let body_phase = phase_rng.next_f32();

        let event_duration = (sample_rate * (0.024 + config.activity * 0.016)) as u32;
        let event_positions = config.kind.event_positions();
        let events: [EquipmentMovementEvent; EQUIPMENT_MOVEMENT_EVENTS] =
            std::array::from_fn(|index| {
                if index >= profile.event_count {
                    return EquipmentMovementEvent {
                        start_frame: total,
                        duration_frames: 0,
                        amplitude: 0.0,
                        phase: 0.0,
                        phase_step: 0.0,
                        decay: 1.0,
                    };
                }

                let jitter = (event_rng.next_f32() - 0.5) * 0.035;
                let progress = (event_positions[index] + jitter).clamp(0.025, 0.975);
                let start_frame = (total as f32 * progress) as u32;
                let frequency = (profile.body_start_frequency
                    * weight_pitch
                    * (0.72 + event_rng.next_f32() * 0.64))
                    .min(sample_rate * 0.40);
                let amplitude = profile.event_gain
                    * (0.72 + config.activity * 0.56)
                    * (0.78 + config.weight * 0.40)
                    * (0.72 + event_rng.next_f32() * 0.56);
                EquipmentMovementEvent {
                    start_frame,
                    duration_frames: event_duration,
                    amplitude,
                    phase: event_rng.next_f32(),
                    phase_step: frequency / sample_rate,
                    decay: 10.0 + event_rng.next_f32() * 9.0,
                }
            });

        Ok(Self {
            sample_rate,
            intensity: config.intensity,
            position: 0,
            total,
            rng: noise_rng,
            noise_state: 0.0,
            noise_gain: profile.noise_gain * (0.66 + config.activity * 0.68),
            noise_coefficient: profile.noise_coefficient,
            body_phase,
            body_start_frequency,
            body_end_frequency,
            body_attack: profile.body_attack,
            body_decay: profile.body_decay * (1.08 - config.weight * 0.16),
            body_gain: profile.body_gain * (0.78 + config.weight * 0.42),
            events,
        })
    }
}

impl ProceduralGenerator for EquipmentMovementGenerator {
    fn render(&mut self, output: &mut [f32]) -> Result<(), ProceduralError> {
        validate_output(output)?;
        let (frames, _) = output.as_chunks_mut::<2>();

        for frame in frames {
            if self.position >= self.total {
                frame.fill(0.0);
                continue;
            }

            let elapsed = self.position as f32 / self.sample_rate;
            let progress = self.position as f32 / self.total as f32;
            let noise = self.rng.next_f32() * 2.0 - 1.0;
            self.noise_state += self.noise_coefficient * (noise - self.noise_state);
            let high_frequency = noise - self.noise_state;
            let texture = (high_frequency * (1.0 - self.noise_coefficient)
                + self.noise_state * self.noise_coefficient)
                * self.noise_gain
                * (TAU * progress * 0.5).sin();

            let attack = 1.0 - (-elapsed / self.body_attack).exp();
            let body_envelope = attack * (-elapsed * self.body_decay).exp();
            let body_frequency = self.body_start_frequency
                + (self.body_end_frequency - self.body_start_frequency) * progress;
            let body = (self.body_phase * TAU).sin() * body_envelope * self.body_gain;
            let contacts: f32 = self
                .events
                .iter()
                .map(|event| event.sample(self.position, self.sample_rate))
                .sum();

            let sample = (texture + body + contacts) * self.intensity;
            let sample = sample.clamp(-1.0, 1.0);
            frame[0] = sample;
            frame[1] = sample;

            self.body_phase = (self.body_phase + body_frequency / self.sample_rate).rem_euclid(1.0);
            self.position += 1;
        }

        Ok(())
    }

    fn is_finished(&self) -> bool {
        self.position >= self.total
    }
}

const OBJECT_INTERACTION_EVENTS: usize = 4;

#[derive(Clone, Copy)]
struct ObjectInteractionEvent {
    start_frame: u32,
    duration_frames: u32,
    amplitude: f32,
    phase: f32,
    phase_step: f32,
    decay: f32,
}

impl ObjectInteractionEvent {
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

/// Dominant object material used to shape interaction resonance and contact texture.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ObjectInteractionMaterial {
    /// Damped, woody resonance.
    Wood,
    /// Bright, sustained metallic contact.
    Metal,
    /// Dense, short stone response.
    Stone,
    /// Bright, delicate glass response.
    Glass,
    /// Hard, bright ceramic response.
    Ceramic,
}

impl ObjectInteractionMaterial {
    fn profile(self) -> (f32, f32, f32, f32) {
        match self {
            Self::Wood => (240.0, 15.0, 0.74, 0.34),
            Self::Metal => (920.0, 24.0, 0.94, 0.18),
            Self::Stone => (430.0, 18.0, 0.82, 0.25),
            Self::Glass => (1_280.0, 30.0, 0.91, 0.14),
            Self::Ceramic => (1_020.0, 26.0, 0.88, 0.16),
        }
    }
}

/// General object action represented by a procedural one-shot.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ObjectInteractionKind {
    /// Lift or take hold of an object, with restrained contact and movement texture.
    Pickup,
    /// Set an object down in a controlled way.
    Place,
    /// Drop an object, with a short, bounded sequence of suggested bounces.
    Drop,
    /// Move an object or fixture into an open position.
    Open,
    /// Move an object or fixture into a closed position.
    Close,
    /// Operate a small switch, catch, or similar toggle.
    Toggle,
}

#[derive(Clone, Copy)]
struct ObjectInteractionProfile {
    duration: f32,
    noise_gain: f32,
    body_gain: f32,
    body_attack: f32,
    body_decay: f32,
    event_count: usize,
    event_gain: f32,
    event_positions: [f32; OBJECT_INTERACTION_EVENTS],
}

impl ObjectInteractionKind {
    fn profile(self) -> ObjectInteractionProfile {
        match self {
            Self::Pickup => ObjectInteractionProfile {
                duration: 0.30,
                noise_gain: 0.34,
                body_gain: 0.18,
                body_attack: 0.008,
                body_decay: 15.0,
                event_count: 1,
                event_gain: 0.20,
                event_positions: [0.16, 0.0, 0.0, 0.0],
            },
            Self::Place => ObjectInteractionProfile {
                duration: 0.36,
                noise_gain: 0.15,
                body_gain: 0.34,
                body_attack: 0.0015,
                body_decay: 14.0,
                event_count: 1,
                event_gain: 0.42,
                event_positions: [0.76, 0.0, 0.0, 0.0],
            },
            Self::Drop => ObjectInteractionProfile {
                duration: 0.62,
                noise_gain: 0.09,
                body_gain: 0.48,
                body_attack: 0.0008,
                body_decay: 6.5,
                event_count: 4,
                event_gain: 0.45,
                event_positions: [0.035, 0.29, 0.52, 0.72],
            },
            Self::Open => ObjectInteractionProfile {
                duration: 0.72,
                noise_gain: 0.48,
                body_gain: 0.16,
                body_attack: 0.018,
                body_decay: 8.0,
                event_count: 2,
                event_gain: 0.13,
                event_positions: [0.14, 0.88, 0.0, 0.0],
            },
            Self::Close => ObjectInteractionProfile {
                duration: 0.46,
                noise_gain: 0.20,
                body_gain: 0.40,
                body_attack: 0.001,
                body_decay: 12.0,
                event_count: 2,
                event_gain: 0.36,
                event_positions: [0.78, 0.94, 0.0, 0.0],
            },
            Self::Toggle => ObjectInteractionProfile {
                duration: 0.22,
                noise_gain: 0.05,
                body_gain: 0.14,
                body_attack: 0.0005,
                body_decay: 26.0,
                event_count: 1,
                event_gain: 0.48,
                event_positions: [0.46, 0.0, 0.0, 0.0],
            },
        }
    }

    fn stream_index(self) -> u64 {
        match self {
            Self::Pickup => 0,
            Self::Place => 1,
            Self::Drop => 2,
            Self::Open => 3,
            Self::Close => 4,
            Self::Toggle => 5,
        }
    }
}

/// Configuration for one deterministic object-interaction sound.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ObjectInteractionConfig {
    /// Output sample rate in Hz.
    pub sample_rate: u32,
    /// Overall output level, 0.0..=1.0.
    pub intensity: f32,
    /// Relative object weight, 0.0..=1.0. Higher values lower the resonant body.
    pub weight: f32,
    /// Action speed, 0.0..=1.0. Higher values tighten the action and shorten its movement texture.
    pub speed: f32,
    /// Dominant material character of the handled object or fixture.
    pub material: ObjectInteractionMaterial,
    /// Object action to render.
    pub kind: ObjectInteractionKind,
}

impl Default for ObjectInteractionConfig {
    fn default() -> Self {
        Self {
            sample_rate: 44_100,
            intensity: 0.78,
            weight: 0.55,
            speed: 0.55,
            material: ObjectInteractionMaterial::Wood,
            kind: ObjectInteractionKind::Close,
        }
    }
}

/// Finite deterministic generator for common object pickup, placement, drop, and mechanism actions.
///
/// Material shapes the resonance and contact texture. Weight shifts the body toward lower
/// frequencies, while speed changes duration and transient character. Contact events are
/// prepared at construction in a fixed-size array, using seeded streams independent from the
/// rendered noise. These are stylized interaction sounds, not door, lock, or rigid-body
/// simulations; use host-provided recordings for recognizable mechanisms or specific objects.
pub struct ObjectInteractionGenerator {
    sample_rate: f32,
    intensity: f32,
    position: u32,
    total: u32,
    noise_rng: Rng,
    noise_state: f32,
    noise_gain: f32,
    noise_coefficient: f32,
    body_phase: f32,
    body_start_frequency: f32,
    body_end_frequency: f32,
    body_attack: f32,
    body_decay: f32,
    body_gain: f32,
    events: [ObjectInteractionEvent; OBJECT_INTERACTION_EVENTS],
}

impl ObjectInteractionGenerator {
    /// Construct one deterministic object-interaction sound with independent random streams.
    pub fn new(config: ObjectInteractionConfig, seed: u64) -> Result<Self, ProceduralError> {
        validate_rate(config.sample_rate)?;
        validate_unit(config.intensity)?;
        validate_unit(config.weight)?;
        validate_unit(config.speed)?;

        let profile = config.kind.profile();
        let (material_frequency, material_decay, material_texture, noise_coefficient) =
            config.material.profile();
        let sample_rate = config.sample_rate as f32;
        let duration_scale = (1.24 - config.speed * 0.42) * (0.92 + config.weight * 0.14);
        let total = (sample_rate * profile.duration * duration_scale) as u32;
        let weight_pitch = 1.24 - config.weight * 0.42;
        let speed_pitch = 0.94 + config.speed * 0.12;
        let body_start_frequency =
            (material_frequency * weight_pitch * speed_pitch).min(sample_rate * 0.40);
        let body_end_frequency =
            (body_start_frequency * (0.78 + config.speed * 0.16)).min(sample_rate * 0.40);
        let stream_index = config.kind.stream_index();

        let noise_rng = Rng::new(derive_seed(seed, 0x4F49_4E4F, stream_index));
        let mut phase_rng = Rng::new(derive_seed(seed, 0x4F49_5048, stream_index));
        let mut event_rng = Rng::new(derive_seed(seed, 0x4F49_4556, stream_index));
        let body_phase = phase_rng.next_f32();

        let event_duration =
            (sample_rate * (0.020 + config.weight * 0.012 + (1.0 - config.speed) * 0.008)) as u32;
        let events: [ObjectInteractionEvent; OBJECT_INTERACTION_EVENTS] =
            std::array::from_fn(|index| {
                if index >= profile.event_count {
                    return ObjectInteractionEvent {
                        start_frame: total,
                        duration_frames: 0,
                        amplitude: 0.0,
                        phase: 0.0,
                        phase_step: 0.0,
                        decay: 1.0,
                    };
                }

                let jitter = (event_rng.next_f32() - 0.5) * 0.025;
                let progress = (profile.event_positions[index] + jitter).clamp(0.02, 0.97);
                let start_frame = (total as f32 * progress) as u32;
                let frequency =
                    (material_frequency * weight_pitch * (0.68 + event_rng.next_f32() * 0.85))
                        .min(sample_rate * 0.40);
                let amplitude = profile.event_gain
                    * material_texture
                    * (0.72 + config.weight * 0.44)
                    * (0.72 + event_rng.next_f32() * 0.56);
                ObjectInteractionEvent {
                    start_frame,
                    duration_frames: event_duration,
                    amplitude,
                    phase: event_rng.next_f32(),
                    phase_step: frequency / sample_rate,
                    decay: material_decay * (0.78 + event_rng.next_f32() * 0.44),
                }
            });

        Ok(Self {
            sample_rate,
            intensity: config.intensity,
            position: 0,
            total,
            noise_rng,
            noise_state: 0.0,
            noise_gain: profile.noise_gain * material_texture * (0.82 + config.speed * 0.35),
            noise_coefficient,
            body_phase,
            body_start_frequency,
            body_end_frequency,
            body_attack: profile.body_attack,
            body_decay: profile.body_decay * (1.08 - config.weight * 0.18),
            body_gain: profile.body_gain * material_texture * (0.78 + config.weight * 0.44),
            events,
        })
    }
}

impl ProceduralGenerator for ObjectInteractionGenerator {
    fn render(&mut self, output: &mut [f32]) -> Result<(), ProceduralError> {
        validate_output(output)?;
        let (frames, _) = output.as_chunks_mut::<2>();

        for frame in frames {
            if self.position >= self.total {
                frame.fill(0.0);
                continue;
            }

            let elapsed = self.position as f32 / self.sample_rate;
            let progress = self.position as f32 / self.total as f32;
            let noise = self.noise_rng.next_f32() * 2.0 - 1.0;
            self.noise_state += self.noise_coefficient * (noise - self.noise_state);
            let texture =
                (noise - self.noise_state) * self.noise_gain * (TAU * progress * 0.5).sin();

            let attack = 1.0 - (-elapsed / self.body_attack).exp();
            let body_envelope = attack * (-elapsed * self.body_decay).exp();
            let body_frequency = self.body_start_frequency
                + (self.body_end_frequency - self.body_start_frequency) * progress;
            let body = (self.body_phase * TAU).sin() * body_envelope * self.body_gain;
            let contacts: f32 = self
                .events
                .iter()
                .map(|event| event.sample(self.position, self.sample_rate))
                .sum();

            let sample = ((texture + body + contacts) * self.intensity).clamp(-1.0, 1.0);
            frame[0] = sample;
            frame[1] = sample;

            self.body_phase = (self.body_phase + body_frequency / self.sample_rate).rem_euclid(1.0);
            self.position += 1;
        }

        Ok(())
    }

    fn is_finished(&self) -> bool {
        self.position >= self.total
    }
}

const DAMAGE_EFFECT_EVENTS: usize = 8;

#[derive(Clone, Copy)]
struct DamageEffectEvent {
    start_frame: u32,
    duration_frames: u32,
    amplitude: f32,
    phase: f32,
    phase_step: f32,
    decay: f32,
}

impl DamageEffectEvent {
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

/// Stylized non-vocal damage sound represented by a procedural one-shot.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DamageEffectKind {
    /// A wet, broadband splash followed by a small sequence of droplets.
    BloodSplatter,
    /// A sharp dry fracture with several short secondary cracks.
    BoneBreak,
}

#[derive(Clone, Copy)]
struct DamageEffectProfile {
    duration: f32,
    noise_coefficient: f32,
    low_noise_gain: f32,
    high_noise_gain: f32,
    noise_decay: f32,
    body_start_frequency: f32,
    body_end_frequency: f32,
    body_attack: f32,
    body_decay: f32,
    body_gain: f32,
    event_count: usize,
    event_gain: f32,
    event_duration: f32,
    event_decay: f32,
    event_positions: [f32; DAMAGE_EFFECT_EVENTS],
}

impl DamageEffectKind {
    fn profile(self) -> DamageEffectProfile {
        match self {
            Self::BloodSplatter => DamageEffectProfile {
                duration: 0.52,
                noise_coefficient: 0.18,
                low_noise_gain: 0.48,
                high_noise_gain: 0.17,
                noise_decay: 7.5,
                body_start_frequency: 118.0,
                body_end_frequency: 82.0,
                body_attack: 0.004,
                body_decay: 8.0,
                body_gain: 0.14,
                event_count: 7,
                event_gain: 0.12,
                event_duration: 0.034,
                event_decay: 46.0,
                event_positions: [0.05, 0.16, 0.29, 0.43, 0.59, 0.76, 0.91, 0.0],
            },
            Self::BoneBreak => DamageEffectProfile {
                duration: 0.42,
                noise_coefficient: 0.76,
                low_noise_gain: 0.08,
                high_noise_gain: 0.72,
                noise_decay: 18.0,
                body_start_frequency: 760.0,
                body_end_frequency: 310.0,
                body_attack: 0.0006,
                body_decay: 15.0,
                body_gain: 0.38,
                event_count: 6,
                event_gain: 0.17,
                event_duration: 0.022,
                event_decay: 62.0,
                event_positions: [0.02, 0.11, 0.23, 0.39, 0.63, 0.84, 0.0, 0.0],
            },
        }
    }

    fn stream_index(self) -> u64 {
        match self {
            Self::BloodSplatter => 0,
            Self::BoneBreak => 1,
        }
    }
}

/// Configuration for one deterministic non-vocal damage effect.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct DamageEffectConfig {
    /// Output sample rate in Hz.
    pub sample_rate: u32,
    /// Overall output level, 0.0..=1.0.
    pub intensity: f32,
    /// Impact strength, 0.0..=1.0.
    pub force: f32,
    /// Relative scale of the effect, 0.0..=1.0. Larger values broaden the sound and lower resonance.
    pub size: f32,
    /// Damage sound to render.
    pub kind: DamageEffectKind,
}

impl Default for DamageEffectConfig {
    fn default() -> Self {
        Self {
            sample_rate: 44_100,
            intensity: 0.72,
            force: 0.65,
            size: 0.45,
            kind: DamageEffectKind::BloodSplatter,
        }
    }
}

/// Finite deterministic generator for stylized blood-splatter and bone-break effects.
///
/// The two profiles share a bounded one-shot renderer but use different noise, resonance, and
/// contact-event balances. Force controls event energy; size changes duration, resonance, and
/// event spread. Seeded events are prepared at construction in fixed-capacity storage, with
/// independent streams for noise, phase, and secondary contacts. These are non-vocal sound-design
/// textures, not anatomical simulations or substitutes for recorded character performances.
pub struct DamageEffectGenerator {
    sample_rate: f32,
    intensity: f32,
    force_gain: f32,
    position: u32,
    total: u32,
    noise_rng: Rng,
    noise_state: f32,
    noise_coefficient: f32,
    low_noise_gain: f32,
    high_noise_gain: f32,
    noise_decay: f32,
    body_phase: f32,
    body_start_frequency: f32,
    body_end_frequency: f32,
    body_attack: f32,
    body_decay: f32,
    body_gain: f32,
    events: [DamageEffectEvent; DAMAGE_EFFECT_EVENTS],
}

impl DamageEffectGenerator {
    /// Construct one deterministic damage effect with independent random streams.
    pub fn new(config: DamageEffectConfig, seed: u64) -> Result<Self, ProceduralError> {
        validate_rate(config.sample_rate)?;
        validate_unit(config.intensity)?;
        validate_unit(config.force)?;
        validate_unit(config.size)?;

        let profile = config.kind.profile();
        let sample_rate = config.sample_rate as f32;
        let size_scale = 0.68 + config.size * 0.64;
        let total = (sample_rate * profile.duration * size_scale) as u32;
        let pitch_scale = 1.22 - config.size * 0.38;
        let body_start_frequency =
            (profile.body_start_frequency * pitch_scale).min(sample_rate * 0.40);
        let body_end_frequency = (profile.body_end_frequency * pitch_scale).min(sample_rate * 0.40);
        let stream_index = config.kind.stream_index();

        let noise_rng = Rng::new(derive_seed(seed, 0x444D_4E4F, stream_index));
        let mut phase_rng = Rng::new(derive_seed(seed, 0x444D_5048, stream_index));
        let mut event_rng = Rng::new(derive_seed(seed, 0x444D_4556, stream_index));
        let body_phase = phase_rng.next_f32();
        let force_gain = 0.42 + config.force * 0.88;

        let event_window = total as f32 * (0.72 + config.size * 0.22);
        let event_duration =
            (sample_rate * profile.event_duration * (0.84 + config.size * 0.32)) as u32;
        let events: [DamageEffectEvent; DAMAGE_EFFECT_EVENTS] = std::array::from_fn(|index| {
            if index >= profile.event_count {
                return DamageEffectEvent {
                    start_frame: total,
                    duration_frames: 0,
                    amplitude: 0.0,
                    phase: 0.0,
                    phase_step: 0.0,
                    decay: 1.0,
                };
            }

            let jitter = (event_rng.next_f32() - 0.5) * 0.035;
            let progress = (profile.event_positions[index] + jitter).clamp(0.01, 0.98);
            let start_frame = (event_window * progress) as u32;
            let frequency =
                (profile.body_start_frequency * pitch_scale * (0.70 + event_rng.next_f32() * 0.70))
                    .min(sample_rate * 0.40);
            let amplitude = profile.event_gain
                * (0.70 + config.force * 0.58)
                * (0.74 + config.size * 0.48)
                * (0.72 + event_rng.next_f32() * 0.56);
            DamageEffectEvent {
                start_frame,
                duration_frames: event_duration,
                amplitude,
                phase: event_rng.next_f32(),
                phase_step: frequency / sample_rate,
                decay: profile.event_decay * (0.80 + event_rng.next_f32() * 0.40),
            }
        });

        Ok(Self {
            sample_rate,
            intensity: config.intensity,
            force_gain,
            position: 0,
            total,
            noise_rng,
            noise_state: 0.0,
            noise_coefficient: profile.noise_coefficient,
            low_noise_gain: profile.low_noise_gain,
            high_noise_gain: profile.high_noise_gain,
            noise_decay: profile.noise_decay * (1.12 - config.size * 0.20),
            body_phase,
            body_start_frequency,
            body_end_frequency,
            body_attack: profile.body_attack,
            body_decay: profile.body_decay * (1.08 - config.size * 0.16),
            body_gain: profile.body_gain * (0.82 + config.size * 0.40),
            events,
        })
    }
}

impl ProceduralGenerator for DamageEffectGenerator {
    fn render(&mut self, output: &mut [f32]) -> Result<(), ProceduralError> {
        validate_output(output)?;
        let (frames, _) = output.as_chunks_mut::<2>();

        for frame in frames {
            if self.position >= self.total {
                frame.fill(0.0);
                continue;
            }

            let elapsed = self.position as f32 / self.sample_rate;
            let progress = self.position as f32 / self.total as f32;
            let noise = self.noise_rng.next_f32() * 2.0 - 1.0;
            self.noise_state += self.noise_coefficient * (noise - self.noise_state);
            let high_noise = noise - self.noise_state;
            let noise_envelope = (-elapsed * self.noise_decay).exp() * (TAU * progress * 0.5).sin();
            let texture = (self.noise_state * self.low_noise_gain
                + high_noise * self.high_noise_gain)
                * noise_envelope;

            let attack = 1.0 - (-elapsed / self.body_attack).exp();
            let body_envelope = attack * (-elapsed * self.body_decay).exp();
            let body_frequency = self.body_start_frequency
                + (self.body_end_frequency - self.body_start_frequency) * progress;
            let body = (self.body_phase * TAU).sin() * body_envelope * self.body_gain;
            let contacts: f32 = self
                .events
                .iter()
                .map(|event| event.sample(self.position, self.sample_rate))
                .sum();

            let sample =
                ((texture + body + contacts) * self.intensity * self.force_gain).clamp(-1.0, 1.0);
            frame[0] = sample;
            frame[1] = sample;

            self.body_phase = (self.body_phase + body_frequency / self.sample_rate).rem_euclid(1.0);
            self.position += 1;
        }

        Ok(())
    }

    fn is_finished(&self) -> bool {
        self.position >= self.total
    }
}

const TOOL_ACTION_CONTACTS: usize = 4;

#[derive(Clone, Copy)]
struct ToolActionContact {
    start_frame: u32,
    duration_frames: u32,
    amplitude: f32,
    phase: f32,
    phase_step: f32,
    decay: f32,
}

impl ToolActionContact {
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

/// Target material for a procedural tool-action sound.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ToolActionMaterial {
    /// Soft soil and loose earth with a low, muted response.
    Earth,
    /// Fibrous wood with a dry, woody resonance.
    Wood,
    /// Dense stone with a harder, brighter contact.
    Stone,
    /// Metal with a bright, ringing contact.
    Metal,
}

impl ToolActionMaterial {
    fn profile(self) -> (f32, f32, f32, f32, f32) {
        match self {
            Self::Earth => (145.0, 0.14, 0.72, 0.28, 1.35),
            Self::Wood => (265.0, 0.28, 0.34, 0.74, 1.0),
            Self::Stone => (520.0, 0.42, 0.25, 0.92, 0.82),
            Self::Metal => (940.0, 0.22, 0.48, 0.68, 0.62),
        }
    }
}

/// Tool action represented by a finite procedural sound.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ToolActionKind {
    /// A forceful cutting contact followed by a short follow-through.
    Chop,
    /// A gritty tool entry and withdrawal through loose material.
    Dig,
    /// A sustained friction pass across a hard or rough surface.
    Scrape,
    /// Repeated short friction strokes suggesting a hand saw.
    Saw,
}

#[derive(Clone, Copy)]
struct ToolActionProfile {
    duration: f32,
    friction_gain: f32,
    friction_cycles: f32,
    body_gain: f32,
    body_attack: f32,
    body_decay: f32,
    contact_count: usize,
    contact_gain: f32,
    contact_duration: f32,
    contact_decay: f32,
    contact_positions: [f32; TOOL_ACTION_CONTACTS],
}

impl ToolActionKind {
    fn profile(self) -> ToolActionProfile {
        match self {
            Self::Chop => ToolActionProfile {
                duration: 0.48,
                friction_gain: 0.13,
                friction_cycles: 1.0,
                body_gain: 0.30,
                body_attack: 0.0008,
                body_decay: 12.0,
                contact_count: 2,
                contact_gain: 0.42,
                contact_duration: 0.034,
                contact_decay: 30.0,
                contact_positions: [0.035, 0.84, 0.0, 0.0],
            },
            Self::Dig => ToolActionProfile {
                duration: 0.76,
                friction_gain: 0.38,
                friction_cycles: 3.0,
                body_gain: 0.20,
                body_attack: 0.004,
                body_decay: 7.0,
                contact_count: 3,
                contact_gain: 0.20,
                contact_duration: 0.030,
                contact_decay: 26.0,
                contact_positions: [0.16, 0.54, 0.88, 0.0],
            },
            Self::Scrape => ToolActionProfile {
                duration: 0.62,
                friction_gain: 0.68,
                friction_cycles: 1.5,
                body_gain: 0.12,
                body_attack: 0.016,
                body_decay: 5.5,
                contact_count: 1,
                contact_gain: 0.12,
                contact_duration: 0.025,
                contact_decay: 34.0,
                contact_positions: [0.90, 0.0, 0.0, 0.0],
            },
            Self::Saw => ToolActionProfile {
                duration: 0.84,
                friction_gain: 0.72,
                friction_cycles: 9.0,
                body_gain: 0.10,
                body_attack: 0.012,
                body_decay: 4.8,
                contact_count: 2,
                contact_gain: 0.11,
                contact_duration: 0.020,
                contact_decay: 38.0,
                contact_positions: [0.04, 0.94, 0.0, 0.0],
            },
        }
    }

    fn stream_index(self) -> u64 {
        match self {
            Self::Chop => 0,
            Self::Dig => 1,
            Self::Scrape => 2,
            Self::Saw => 3,
        }
    }
}

/// Configuration for one deterministic tool-action sound.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ToolActionConfig {
    /// Output sample rate in Hz.
    pub sample_rate: u32,
    /// Overall output level, 0.0..=1.0.
    pub intensity: f32,
    /// Applied tool force, 0.0..=1.0.
    pub force: f32,
    /// Relative tool weight, 0.0..=1.0. Higher values lower the resonant body.
    pub weight: f32,
    /// Action speed, 0.0..=1.0. Higher values shorten the action and tighten repeated strokes.
    pub speed: f32,
    /// Material contacted by the tool.
    pub material: ToolActionMaterial,
    /// Tool action to render.
    pub kind: ToolActionKind,
}

impl Default for ToolActionConfig {
    fn default() -> Self {
        Self {
            sample_rate: 44_100,
            intensity: 0.78,
            force: 0.7,
            weight: 0.55,
            speed: 0.55,
            material: ToolActionMaterial::Wood,
            kind: ToolActionKind::Chop,
        }
    }
}

/// Finite deterministic generator for chopping, digging, scraping, and sawing sounds.
///
/// Material shapes the resonance and friction texture. Force controls contact energy; weight
/// shifts the resonant body, and speed changes duration and repeated-friction timing. Contacts
/// are prepared at construction in a fixed-size array using a seed stream independent from the
/// rendered noise. These profiles are stylized tool-use textures, not physical tool or material
/// simulations; use host-provided recordings for distinctive tools or machinery.
pub struct ToolActionGenerator {
    sample_rate: f32,
    intensity: f32,
    force_gain: f32,
    position: u32,
    total: u32,
    noise_rng: Rng,
    noise_state: f32,
    noise_coefficient: f32,
    low_noise_gain: f32,
    high_noise_gain: f32,
    friction_gain: f32,
    friction_cycles: f32,
    body_phase: f32,
    body_start_frequency: f32,
    body_end_frequency: f32,
    body_attack: f32,
    body_decay: f32,
    body_gain: f32,
    contacts: [ToolActionContact; TOOL_ACTION_CONTACTS],
}

impl ToolActionGenerator {
    /// Construct one deterministic tool-action sound with independent random streams.
    pub fn new(config: ToolActionConfig, seed: u64) -> Result<Self, ProceduralError> {
        validate_rate(config.sample_rate)?;
        validate_unit(config.intensity)?;
        validate_unit(config.force)?;
        validate_unit(config.weight)?;
        validate_unit(config.speed)?;

        let profile = config.kind.profile();
        let (material_frequency, noise_coefficient, low_noise_gain, high_noise_gain, damping) =
            config.material.profile();
        let sample_rate = config.sample_rate as f32;
        let duration_scale = (1.22 - config.speed * 0.42) * (1.04 - config.weight * 0.08);
        let total = (sample_rate * profile.duration * duration_scale) as u32;
        let weight_pitch = 1.24 - config.weight * 0.42;
        let speed_pitch = 0.92 + config.speed * 0.14;
        let body_start_frequency =
            (material_frequency * weight_pitch * speed_pitch).min(sample_rate * 0.40);
        let body_end_frequency =
            (body_start_frequency * (0.74 + config.speed * 0.18)).min(sample_rate * 0.40);
        let stream_index = config.kind.stream_index();

        let noise_rng = Rng::new(derive_seed(seed, 0x5441_4E4F, stream_index));
        let mut phase_rng = Rng::new(derive_seed(seed, 0x5441_5048, stream_index));
        let mut contact_rng = Rng::new(derive_seed(seed, 0x5441_434F, stream_index));
        let body_phase = phase_rng.next_f32();
        let force_gain = 0.42 + config.force * 0.88;

        let contact_window = total as f32 * (0.78 + config.speed * 0.16);
        let contact_duration =
            (sample_rate * profile.contact_duration * (1.10 - config.speed * 0.20)) as u32;
        let contacts: [ToolActionContact; TOOL_ACTION_CONTACTS] = std::array::from_fn(|index| {
            if index >= profile.contact_count {
                return ToolActionContact {
                    start_frame: total,
                    duration_frames: 0,
                    amplitude: 0.0,
                    phase: 0.0,
                    phase_step: 0.0,
                    decay: 1.0,
                };
            }

            let jitter = (contact_rng.next_f32() - 0.5) * 0.03;
            let progress = (profile.contact_positions[index] + jitter).clamp(0.015, 0.98);
            let start_frame = (contact_window * progress) as u32;
            let frequency =
                (material_frequency * weight_pitch * (0.72 + contact_rng.next_f32() * 0.68))
                    .min(sample_rate * 0.40);
            let amplitude = profile.contact_gain
                * (0.70 + config.force * 0.60)
                * (0.76 + config.weight * 0.42)
                * (0.72 + contact_rng.next_f32() * 0.56);
            ToolActionContact {
                start_frame,
                duration_frames: contact_duration,
                amplitude,
                phase: contact_rng.next_f32(),
                phase_step: frequency / sample_rate,
                decay: profile.contact_decay * damping * (0.80 + contact_rng.next_f32() * 0.40),
            }
        });

        Ok(Self {
            sample_rate,
            intensity: config.intensity,
            force_gain,
            position: 0,
            total,
            noise_rng,
            noise_state: 0.0,
            noise_coefficient,
            low_noise_gain,
            high_noise_gain,
            friction_gain: profile.friction_gain * (0.78 + config.force * 0.46),
            friction_cycles: profile.friction_cycles * (0.72 + config.speed * 0.56),
            body_phase,
            body_start_frequency,
            body_end_frequency,
            body_attack: profile.body_attack,
            body_decay: profile.body_decay * damping,
            body_gain: profile.body_gain * (0.78 + config.weight * 0.42),
            contacts,
        })
    }
}

impl ProceduralGenerator for ToolActionGenerator {
    fn render(&mut self, output: &mut [f32]) -> Result<(), ProceduralError> {
        validate_output(output)?;
        let (frames, _) = output.as_chunks_mut::<2>();

        for frame in frames {
            if self.position >= self.total {
                frame.fill(0.0);
                continue;
            }

            let elapsed = self.position as f32 / self.sample_rate;
            let progress = self.position as f32 / self.total as f32;
            let noise = self.noise_rng.next_f32() * 2.0 - 1.0;
            self.noise_state += self.noise_coefficient * (noise - self.noise_state);
            let high_noise = noise - self.noise_state;
            let action_envelope = (TAU * progress * 0.5).sin();
            let stroke_phase = progress * self.friction_cycles;
            let stroke_envelope = 0.48 + 0.52 * (TAU * stroke_phase).sin().abs();
            let friction = (self.noise_state * self.low_noise_gain
                + high_noise * self.high_noise_gain)
                * self.friction_gain
                * action_envelope
                * stroke_envelope;

            let attack = 1.0 - (-elapsed / self.body_attack).exp();
            let body_envelope = attack * (-elapsed * self.body_decay).exp();
            let body_frequency = self.body_start_frequency
                + (self.body_end_frequency - self.body_start_frequency) * progress;
            let body = (self.body_phase * TAU).sin() * body_envelope * self.body_gain;
            let contacts: f32 = self
                .contacts
                .iter()
                .map(|contact| contact.sample(self.position, self.sample_rate))
                .sum();

            let sample =
                ((friction + body + contacts) * self.intensity * self.force_gain).clamp(-1.0, 1.0);
            frame[0] = sample;
            frame[1] = sample;

            self.body_phase = (self.body_phase + body_frequency / self.sample_rate).rem_euclid(1.0);
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
    fn object_interactions_are_deterministic_across_blocks_and_finish_cleanly() {
        let config = ObjectInteractionConfig {
            kind: ObjectInteractionKind::Drop,
            ..ObjectInteractionConfig::default()
        };
        let mut whole = ObjectInteractionGenerator::new(config, 0x0B1E).unwrap();
        let mut split = ObjectInteractionGenerator::new(config, 0x0B1E).unwrap();
        assert!(!whole.is_finished());
        let expected = render(&mut whole, 48_000);
        let mut actual = render(&mut split, 257);
        actual.extend(render(&mut split, 7_991));
        actual.extend(render(&mut split, 39_752));
        assert_eq!(expected, actual);
        bounded(&expected);
        assert!(expected.iter().any(|sample| sample.abs() > 0.001));
        assert!(whole.is_finished());
        assert!(split.is_finished());
        let silence = render(&mut split, 256);
        assert!(silence.iter().all(|sample| *sample == 0.0));
    }

    #[test]
    fn object_interaction_kinds_and_materials_have_distinct_bounded_profiles() {
        let base = ObjectInteractionConfig::default();
        let kinds = [
            ObjectInteractionKind::Pickup,
            ObjectInteractionKind::Place,
            ObjectInteractionKind::Drop,
            ObjectInteractionKind::Open,
            ObjectInteractionKind::Close,
            ObjectInteractionKind::Toggle,
        ];
        let kind_outputs: Vec<Vec<f32>> = kinds
            .into_iter()
            .map(|kind| {
                let mut generator =
                    ObjectInteractionGenerator::new(ObjectInteractionConfig { kind, ..base }, 91)
                        .unwrap();
                let samples = render(&mut generator, 48_000);
                bounded(&samples);
                samples
            })
            .collect();
        for output in kind_outputs.iter().skip(1) {
            assert_ne!(&kind_outputs[0], output);
        }

        let materials = [
            ObjectInteractionMaterial::Wood,
            ObjectInteractionMaterial::Metal,
            ObjectInteractionMaterial::Stone,
            ObjectInteractionMaterial::Glass,
            ObjectInteractionMaterial::Ceramic,
        ];
        let material_outputs: Vec<Vec<f32>> = materials
            .into_iter()
            .map(|material| {
                let mut generator = ObjectInteractionGenerator::new(
                    ObjectInteractionConfig {
                        kind: ObjectInteractionKind::Drop,
                        material,
                        ..base
                    },
                    92,
                )
                .unwrap();
                let samples = render(&mut generator, 48_000);
                bounded(&samples);
                samples
            })
            .collect();
        for output in material_outputs.iter().skip(1) {
            assert_ne!(&material_outputs[0], output);
        }
    }

    #[test]
    fn object_interaction_seed_and_controls_are_effective() {
        let config = ObjectInteractionConfig::default();
        let mut first = ObjectInteractionGenerator::new(config, 7).unwrap();
        let mut same = ObjectInteractionGenerator::new(config, 7).unwrap();
        let mut other_seed = ObjectInteractionGenerator::new(config, 8).unwrap();
        let first_pcm = render(&mut first, 24_000);
        assert_eq!(first_pcm, render(&mut same, 24_000));
        assert_ne!(first_pcm, render(&mut other_seed, 24_000));

        let mut light = ObjectInteractionGenerator::new(
            ObjectInteractionConfig {
                weight: 0.1,
                ..config
            },
            7,
        )
        .unwrap();
        let mut heavy = ObjectInteractionGenerator::new(
            ObjectInteractionConfig {
                weight: 0.95,
                ..config
            },
            7,
        )
        .unwrap();
        assert_ne!(first_pcm, render(&mut light, 24_000));
        assert_ne!(first_pcm, render(&mut heavy, 24_000));

        let mut slow = ObjectInteractionGenerator::new(
            ObjectInteractionConfig {
                speed: 0.1,
                ..config
            },
            7,
        )
        .unwrap();
        let mut fast = ObjectInteractionGenerator::new(
            ObjectInteractionConfig {
                speed: 0.95,
                ..config
            },
            7,
        )
        .unwrap();
        assert_ne!(render(&mut slow, 24_000), render(&mut fast, 24_000));

        let mut quiet = ObjectInteractionGenerator::new(
            ObjectInteractionConfig {
                intensity: 0.2,
                ..config
            },
            7,
        )
        .unwrap();
        let mut loud = ObjectInteractionGenerator::new(
            ObjectInteractionConfig {
                intensity: 0.9,
                ..config
            },
            7,
        )
        .unwrap();
        assert_ne!(render(&mut quiet, 24_000), render(&mut loud, 24_000));
    }

    #[test]
    fn object_interaction_rejects_invalid_configuration_and_odd_output() {
        assert!(matches!(
            ObjectInteractionGenerator::new(
                ObjectInteractionConfig {
                    sample_rate: 384_000,
                    ..ObjectInteractionConfig::default()
                },
                0
            ),
            Err(ProceduralError::InvalidSampleRate)
        ));
        assert!(matches!(
            ObjectInteractionGenerator::new(
                ObjectInteractionConfig {
                    intensity: f32::NAN,
                    ..ObjectInteractionConfig::default()
                },
                0
            ),
            Err(ProceduralError::InvalidParameter)
        ));
        assert!(matches!(
            ObjectInteractionGenerator::new(
                ObjectInteractionConfig {
                    weight: 1.1,
                    ..ObjectInteractionConfig::default()
                },
                0
            ),
            Err(ProceduralError::InvalidParameter)
        ));
        assert!(matches!(
            ObjectInteractionGenerator::new(
                ObjectInteractionConfig {
                    speed: f32::INFINITY,
                    ..ObjectInteractionConfig::default()
                },
                0
            ),
            Err(ProceduralError::InvalidParameter)
        ));
        let mut generator =
            ObjectInteractionGenerator::new(ObjectInteractionConfig::default(), 0).unwrap();
        assert_eq!(
            generator.render(&mut [0.0; 3]),
            Err(ProceduralError::OutputMustContainStereoFrames)
        );
    }

    #[test]
    fn damage_effects_are_deterministic_across_blocks_and_finish_cleanly() {
        for kind in [DamageEffectKind::BloodSplatter, DamageEffectKind::BoneBreak] {
            let config = DamageEffectConfig {
                kind,
                ..DamageEffectConfig::default()
            };
            let mut whole = DamageEffectGenerator::new(config, 0xDA6E).unwrap();
            let mut split = DamageEffectGenerator::new(config, 0xDA6E).unwrap();
            let expected = render(&mut whole, 48_000);
            let mut actual = render(&mut split, 311);
            actual.extend(render(&mut split, 8_192));
            actual.extend(render(&mut split, 39_497));
            assert_eq!(expected, actual);
            bounded(&expected);
            assert!(expected.iter().any(|sample| sample.abs() > 0.001));
            assert!(whole.is_finished());
            assert!(split.is_finished());
            let silence = render(&mut split, 128);
            assert!(silence.iter().all(|sample| *sample == 0.0));
        }
    }

    #[test]
    fn damage_effect_kinds_have_distinct_bounded_profiles() {
        let base = DamageEffectConfig::default();
        let mut splatter = DamageEffectGenerator::new(
            DamageEffectConfig {
                kind: DamageEffectKind::BloodSplatter,
                ..base
            },
            61,
        )
        .unwrap();
        let mut fracture = DamageEffectGenerator::new(
            DamageEffectConfig {
                kind: DamageEffectKind::BoneBreak,
                ..base
            },
            61,
        )
        .unwrap();
        let splatter_pcm = render(&mut splatter, 32_000);
        let fracture_pcm = render(&mut fracture, 32_000);
        assert_ne!(splatter_pcm, fracture_pcm);
        bounded(&splatter_pcm);
        bounded(&fracture_pcm);
    }

    #[test]
    fn damage_effect_seed_and_controls_are_effective() {
        let config = DamageEffectConfig::default();
        let mut first = DamageEffectGenerator::new(config, 17).unwrap();
        let mut same = DamageEffectGenerator::new(config, 17).unwrap();
        let mut other_seed = DamageEffectGenerator::new(config, 18).unwrap();
        let first_pcm = render(&mut first, 24_000);
        assert_eq!(first_pcm, render(&mut same, 24_000));
        assert_ne!(first_pcm, render(&mut other_seed, 24_000));

        let mut light = DamageEffectGenerator::new(
            DamageEffectConfig {
                force: 0.1,
                ..config
            },
            17,
        )
        .unwrap();
        let mut heavy = DamageEffectGenerator::new(
            DamageEffectConfig {
                force: 0.95,
                ..config
            },
            17,
        )
        .unwrap();
        assert_ne!(first_pcm, render(&mut light, 24_000));
        assert_ne!(first_pcm, render(&mut heavy, 24_000));

        let mut small = DamageEffectGenerator::new(
            DamageEffectConfig {
                size: 0.1,
                ..config
            },
            17,
        )
        .unwrap();
        let mut large = DamageEffectGenerator::new(
            DamageEffectConfig {
                size: 0.95,
                ..config
            },
            17,
        )
        .unwrap();
        assert_ne!(render(&mut small, 24_000), render(&mut large, 24_000));

        let mut quiet = DamageEffectGenerator::new(
            DamageEffectConfig {
                intensity: 0.2,
                ..config
            },
            17,
        )
        .unwrap();
        let mut loud = DamageEffectGenerator::new(
            DamageEffectConfig {
                intensity: 0.9,
                ..config
            },
            17,
        )
        .unwrap();
        assert_ne!(render(&mut quiet, 24_000), render(&mut loud, 24_000));
    }

    #[test]
    fn damage_effect_rejects_invalid_configuration_and_odd_output() {
        assert!(matches!(
            DamageEffectGenerator::new(
                DamageEffectConfig {
                    sample_rate: 1_000,
                    ..DamageEffectConfig::default()
                },
                0
            ),
            Err(ProceduralError::InvalidSampleRate)
        ));
        assert!(matches!(
            DamageEffectGenerator::new(
                DamageEffectConfig {
                    intensity: f32::NAN,
                    ..DamageEffectConfig::default()
                },
                0
            ),
            Err(ProceduralError::InvalidParameter)
        ));
        assert!(matches!(
            DamageEffectGenerator::new(
                DamageEffectConfig {
                    force: 1.1,
                    ..DamageEffectConfig::default()
                },
                0
            ),
            Err(ProceduralError::InvalidParameter)
        ));
        assert!(matches!(
            DamageEffectGenerator::new(
                DamageEffectConfig {
                    size: f32::INFINITY,
                    ..DamageEffectConfig::default()
                },
                0
            ),
            Err(ProceduralError::InvalidParameter)
        ));
        let mut generator = DamageEffectGenerator::new(DamageEffectConfig::default(), 0).unwrap();
        assert_eq!(
            generator.render(&mut [0.0; 3]),
            Err(ProceduralError::OutputMustContainStereoFrames)
        );
    }

    #[test]
    fn tool_actions_are_deterministic_across_blocks_and_finish_cleanly() {
        let config = ToolActionConfig {
            kind: ToolActionKind::Saw,
            ..ToolActionConfig::default()
        };
        let mut whole = ToolActionGenerator::new(config, 0x7001).unwrap();
        let mut split = ToolActionGenerator::new(config, 0x7001).unwrap();
        let expected = render(&mut whole, 48_000);
        let mut actual = render(&mut split, 337);
        actual.extend(render(&mut split, 8_192));
        actual.extend(render(&mut split, 39_471));
        assert_eq!(expected, actual);
        bounded(&expected);
        assert!(expected.iter().any(|sample| sample.abs() > 0.001));
        assert!(whole.is_finished());
        assert!(split.is_finished());
        let silence = render(&mut split, 128);
        assert!(silence.iter().all(|sample| *sample == 0.0));
    }

    #[test]
    fn tool_action_kinds_and_materials_have_distinct_bounded_profiles() {
        let base = ToolActionConfig::default();
        let kinds = [
            ToolActionKind::Chop,
            ToolActionKind::Dig,
            ToolActionKind::Scrape,
            ToolActionKind::Saw,
        ];
        let kind_outputs: Vec<Vec<f32>> = kinds
            .into_iter()
            .map(|kind| {
                let mut generator =
                    ToolActionGenerator::new(ToolActionConfig { kind, ..base }, 111).unwrap();
                let samples = render(&mut generator, 48_000);
                bounded(&samples);
                samples
            })
            .collect();
        for output in kind_outputs.iter().skip(1) {
            assert_ne!(&kind_outputs[0], output);
        }

        let materials = [
            ToolActionMaterial::Earth,
            ToolActionMaterial::Wood,
            ToolActionMaterial::Stone,
            ToolActionMaterial::Metal,
        ];
        let material_outputs: Vec<Vec<f32>> = materials
            .into_iter()
            .map(|material| {
                let mut generator = ToolActionGenerator::new(
                    ToolActionConfig {
                        kind: ToolActionKind::Chop,
                        material,
                        ..base
                    },
                    112,
                )
                .unwrap();
                let samples = render(&mut generator, 48_000);
                bounded(&samples);
                samples
            })
            .collect();
        for output in material_outputs.iter().skip(1) {
            assert_ne!(&material_outputs[0], output);
        }
    }

    #[test]
    fn tool_action_seed_and_controls_are_effective() {
        let config = ToolActionConfig::default();
        let mut first = ToolActionGenerator::new(config, 23).unwrap();
        let mut same = ToolActionGenerator::new(config, 23).unwrap();
        let mut other_seed = ToolActionGenerator::new(config, 24).unwrap();
        let first_pcm = render(&mut first, 24_000);
        assert_eq!(first_pcm, render(&mut same, 24_000));
        assert_ne!(first_pcm, render(&mut other_seed, 24_000));

        let mut low_force = ToolActionGenerator::new(
            ToolActionConfig {
                force: 0.1,
                ..config
            },
            23,
        )
        .unwrap();
        let mut high_force = ToolActionGenerator::new(
            ToolActionConfig {
                force: 0.95,
                ..config
            },
            23,
        )
        .unwrap();
        assert_ne!(first_pcm, render(&mut low_force, 24_000));
        assert_ne!(first_pcm, render(&mut high_force, 24_000));

        let mut low_weight = ToolActionGenerator::new(
            ToolActionConfig {
                weight: 0.1,
                ..config
            },
            23,
        )
        .unwrap();
        let mut high_weight = ToolActionGenerator::new(
            ToolActionConfig {
                weight: 0.95,
                ..config
            },
            23,
        )
        .unwrap();
        assert_ne!(first_pcm, render(&mut low_weight, 24_000));
        assert_ne!(first_pcm, render(&mut high_weight, 24_000));

        let mut slow = ToolActionGenerator::new(
            ToolActionConfig {
                speed: 0.1,
                ..config
            },
            23,
        )
        .unwrap();
        let mut fast = ToolActionGenerator::new(
            ToolActionConfig {
                speed: 0.95,
                ..config
            },
            23,
        )
        .unwrap();
        assert_ne!(render(&mut slow, 24_000), render(&mut fast, 24_000));

        let mut quiet = ToolActionGenerator::new(
            ToolActionConfig {
                intensity: 0.2,
                ..config
            },
            23,
        )
        .unwrap();
        let mut loud = ToolActionGenerator::new(
            ToolActionConfig {
                intensity: 0.9,
                ..config
            },
            23,
        )
        .unwrap();
        assert_ne!(render(&mut quiet, 24_000), render(&mut loud, 24_000));
    }

    #[test]
    fn tool_action_rejects_invalid_configuration_and_odd_output() {
        assert!(matches!(
            ToolActionGenerator::new(
                ToolActionConfig {
                    sample_rate: 1_000,
                    ..ToolActionConfig::default()
                },
                0
            ),
            Err(ProceduralError::InvalidSampleRate)
        ));
        assert!(matches!(
            ToolActionGenerator::new(
                ToolActionConfig {
                    intensity: f32::NAN,
                    ..ToolActionConfig::default()
                },
                0
            ),
            Err(ProceduralError::InvalidParameter)
        ));
        assert!(matches!(
            ToolActionGenerator::new(
                ToolActionConfig {
                    force: 1.1,
                    ..ToolActionConfig::default()
                },
                0
            ),
            Err(ProceduralError::InvalidParameter)
        ));
        assert!(matches!(
            ToolActionGenerator::new(
                ToolActionConfig {
                    weight: f32::INFINITY,
                    ..ToolActionConfig::default()
                },
                0
            ),
            Err(ProceduralError::InvalidParameter)
        ));
        assert!(matches!(
            ToolActionGenerator::new(
                ToolActionConfig {
                    speed: -0.1,
                    ..ToolActionConfig::default()
                },
                0
            ),
            Err(ProceduralError::InvalidParameter)
        ));
        let mut generator = ToolActionGenerator::new(ToolActionConfig::default(), 0).unwrap();
        assert_eq!(
            generator.render(&mut [0.0; 3]),
            Err(ProceduralError::OutputMustContainStereoFrames)
        );
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
    fn equipment_movement_is_deterministic_across_blocks_and_finishes_cleanly() {
        let config = EquipmentMovementConfig::default();
        let mut whole = EquipmentMovementGenerator::new(config, 0xE011).unwrap();
        let mut split = EquipmentMovementGenerator::new(config, 0xE011).unwrap();
        assert!(!whole.is_finished());
        let expected = render(&mut whole, 30_000);
        let mut actual = render(&mut split, 12_345);
        actual.extend(render(&mut split, 17_655));
        assert_eq!(expected, actual);
        bounded(&expected);
        assert!(whole.is_finished());

        let total_frames =
            (config.sample_rate as f32 * 0.40 * (1.18 - config.activity * 0.28)) as usize;
        assert!(expected[total_frames * 2..]
            .iter()
            .all(|sample| *sample == 0.0));
        assert!(expected.iter().any(|sample| sample.abs() > 0.001));
    }

    #[test]
    fn equipment_movement_kinds_have_distinct_bounded_profiles() {
        let base = EquipmentMovementConfig::default();
        let kinds = [
            EquipmentMovementKind::ClothRustle,
            EquipmentMovementKind::GearJostle,
            EquipmentMovementKind::ArmorRattle,
        ];
        let mut renders = Vec::new();

        for kind in kinds {
            let mut generator =
                EquipmentMovementGenerator::new(EquipmentMovementConfig { kind, ..base }, 31)
                    .unwrap();
            let samples = render(&mut generator, 30_000);
            bounded(&samples);
            assert!(generator.is_finished());
            assert!(samples.iter().any(|sample| sample.abs() > 0.001));
            renders.push(samples);
        }

        assert_ne!(renders[0], renders[1]);
        assert_ne!(renders[0], renders[2]);
        assert_ne!(renders[1], renders[2]);
    }

    #[test]
    fn equipment_movement_seed_weight_activity_and_intensity_are_effective() {
        let base = EquipmentMovementConfig::default();
        let mut first = EquipmentMovementGenerator::new(base, 9).unwrap();
        let expected = render(&mut first, 24_000);
        let mut same = EquipmentMovementGenerator::new(base, 9).unwrap();
        let mut other_seed = EquipmentMovementGenerator::new(base, 10).unwrap();
        assert_eq!(expected, render(&mut same, 24_000));
        assert_ne!(expected, render(&mut other_seed, 24_000));

        let mut light = EquipmentMovementGenerator::new(
            EquipmentMovementConfig {
                weight: 0.1,
                ..base
            },
            9,
        )
        .unwrap();
        let mut heavy = EquipmentMovementGenerator::new(
            EquipmentMovementConfig {
                weight: 0.95,
                ..base
            },
            9,
        )
        .unwrap();
        assert_ne!(expected, render(&mut light, 24_000));
        assert_ne!(expected, render(&mut heavy, 24_000));

        let mut calm = EquipmentMovementGenerator::new(
            EquipmentMovementConfig {
                activity: 0.1,
                ..base
            },
            9,
        )
        .unwrap();
        let mut vigorous = EquipmentMovementGenerator::new(
            EquipmentMovementConfig {
                activity: 0.95,
                ..base
            },
            9,
        )
        .unwrap();
        assert_ne!(expected, render(&mut calm, 24_000));
        assert_ne!(expected, render(&mut vigorous, 24_000));

        let mut quiet = EquipmentMovementGenerator::new(
            EquipmentMovementConfig {
                intensity: 0.2,
                ..base
            },
            9,
        )
        .unwrap();
        assert_ne!(expected, render(&mut quiet, 24_000));
    }

    #[test]
    fn equipment_movement_rejects_invalid_configuration_and_odd_output() {
        assert!(matches!(
            EquipmentMovementGenerator::new(
                EquipmentMovementConfig {
                    sample_rate: 1_000,
                    ..EquipmentMovementConfig::default()
                },
                0
            ),
            Err(ProceduralError::InvalidSampleRate)
        ));
        assert!(matches!(
            EquipmentMovementGenerator::new(
                EquipmentMovementConfig {
                    activity: f32::NAN,
                    ..EquipmentMovementConfig::default()
                },
                0
            ),
            Err(ProceduralError::InvalidParameter)
        ));
        assert!(matches!(
            EquipmentMovementGenerator::new(
                EquipmentMovementConfig {
                    weight: 1.1,
                    ..EquipmentMovementConfig::default()
                },
                0
            ),
            Err(ProceduralError::InvalidParameter)
        ));

        let mut generator =
            EquipmentMovementGenerator::new(EquipmentMovementConfig::default(), 0).unwrap();
        assert_eq!(
            generator.render(&mut [0.0; 3]),
            Err(ProceduralError::OutputMustContainStereoFrames)
        );
    }

    #[test]
    fn surface_footsteps_are_deterministic_across_blocks_and_end_in_silence() {
        let config = FootstepConfig::default();
        let mut whole =
            FootstepGenerator::new_for_surface(config, FootstepSurface::Gravel, 0xF007).unwrap();
        let mut split =
            FootstepGenerator::new_for_surface(config, FootstepSurface::Gravel, 0xF007).unwrap();
        let expected = render(&mut whole, 12_000);
        let mut actual = render(&mut split, 4_321);
        actual.extend(render(&mut split, 7_679));
        assert_eq!(expected, actual);
        bounded(&expected);
        assert!(whole.is_finished());
        let total_frames = (config.sample_rate as usize * 22) / 100;
        assert!(expected[total_frames * 2..]
            .iter()
            .all(|sample| *sample == 0.0));
        assert!(expected.iter().any(|sample| sample.abs() > 0.001));
    }

    #[test]
    fn footstep_surfaces_have_distinct_bounded_profiles() {
        let config = FootstepConfig::default();
        let surfaces = [
            FootstepSurface::Dirt,
            FootstepSurface::Grass,
            FootstepSurface::Gravel,
            FootstepSurface::Wood,
            FootstepSurface::Stone,
            FootstepSurface::Metal,
            FootstepSurface::ShallowWater,
            FootstepSurface::Snow,
        ];
        let mut renders = Vec::new();
        for surface in surfaces {
            let mut generator =
                FootstepGenerator::new_for_surface(config, surface, 0xF007).unwrap();
            let samples = render(&mut generator, 12_000);
            bounded(&samples);
            assert!(generator.is_finished());
            assert!(samples.iter().any(|sample| sample.abs() > 0.001));
            renders.push(samples);
        }

        for (index, samples) in renders.iter().enumerate() {
            assert!(
                renders[..index].iter().all(|previous| previous != samples),
                "surface profile {index} duplicated an earlier sound"
            );
        }
    }

    #[test]
    fn surface_footstep_controls_and_seed_are_effective() {
        let base = FootstepConfig::default();
        let mut first =
            FootstepGenerator::new_for_surface(base, FootstepSurface::Stone, 14).unwrap();
        let expected = render(&mut first, 12_000);
        let mut same =
            FootstepGenerator::new_for_surface(base, FootstepSurface::Stone, 14).unwrap();
        let mut other_seed =
            FootstepGenerator::new_for_surface(base, FootstepSurface::Stone, 15).unwrap();
        assert_eq!(expected, render(&mut same, 12_000));
        assert_ne!(expected, render(&mut other_seed, 12_000));

        let mut quiet = FootstepGenerator::new_for_surface(
            FootstepConfig {
                intensity: 0.2,
                ..base
            },
            FootstepSurface::Stone,
            14,
        )
        .unwrap();
        let mut bright = FootstepGenerator::new_for_surface(
            FootstepConfig {
                brightness: 0.95,
                ..base
            },
            FootstepSurface::Stone,
            14,
        )
        .unwrap();
        assert_ne!(expected, render(&mut quiet, 12_000));
        assert_ne!(expected, render(&mut bright, 12_000));
    }

    #[test]
    fn surface_footsteps_reject_invalid_configuration_and_odd_output() {
        assert!(matches!(
            FootstepGenerator::new_for_surface(
                FootstepConfig {
                    sample_rate: 1_000,
                    ..FootstepConfig::default()
                },
                FootstepSurface::Dirt,
                0
            ),
            Err(ProceduralError::InvalidSampleRate)
        ));
        assert!(matches!(
            FootstepGenerator::new_for_surface(
                FootstepConfig {
                    brightness: f32::NAN,
                    ..FootstepConfig::default()
                },
                FootstepSurface::Grass,
                0
            ),
            Err(ProceduralError::InvalidParameter)
        ));
        let mut generator = FootstepGenerator::new_for_surface(
            FootstepConfig::default(),
            FootstepSurface::Metal,
            0,
        )
        .unwrap();
        assert_eq!(
            generator.render(&mut [0.0; 3]),
            Err(ProceduralError::OutputMustContainStereoFrames)
        );
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
        assert!(landing_pcm[landing_end..]
            .iter()
            .all(|sample| *sample == 0.0));
        assert!(fall_pcm[fall_end..].iter().all(|sample| *sample == 0.0));
        assert_ne!(jump_pcm, landing_pcm);
        assert_ne!(jump_pcm, fall_pcm);

        let mut other_seed = CharacterActionGenerator::new(base, 18).unwrap();
        assert_ne!(jump_pcm, render(&mut other_seed, 40_000));

        let mut light =
            CharacterActionGenerator::new(CharacterActionConfig { force: 0.1, ..base }, 17)
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

    #[test]
    fn weapon_handling_is_deterministic_across_blocks_and_finishes_cleanly() {
        let config = WeaponHandlingConfig {
            kind: WeaponHandlingKind::Reload,
            ..WeaponHandlingConfig::default()
        };
        let mut whole = WeaponHandlingGenerator::new(config, 0x5745_4150).unwrap();
        let mut split = WeaponHandlingGenerator::new(config, 0x5745_4150).unwrap();
        assert!(!whole.is_finished());
        let expected = render(&mut whole, 50_000);
        let mut actual = render(&mut split, 12_345);
        actual.extend(render(&mut split, 37_655));
        assert_eq!(expected, actual);
        bounded(&expected);
        assert!(whole.is_finished());

        let total_frames =
            (config.sample_rate as f32 * 0.72 * (1.30 - config.speed * 0.60)) as usize;
        assert!(expected[total_frames * 2..]
            .iter()
            .all(|sample| *sample == 0.0));
        assert!(expected.iter().any(|sample| sample.abs() > 0.001));
    }

    #[test]
    fn weapon_handling_kinds_and_materials_change_the_sound() {
        let base = WeaponHandlingConfig::default();
        let mut draw = WeaponHandlingGenerator::new(base, 41).unwrap();
        let mut sheathe = WeaponHandlingGenerator::new(
            WeaponHandlingConfig {
                kind: WeaponHandlingKind::Sheathe,
                ..base
            },
            41,
        )
        .unwrap();
        let mut reload = WeaponHandlingGenerator::new(
            WeaponHandlingConfig {
                kind: WeaponHandlingKind::Reload,
                ..base
            },
            41,
        )
        .unwrap();
        let draw_pcm = render(&mut draw, 40_000);
        let sheathe_pcm = render(&mut sheathe, 40_000);
        let reload_pcm = render(&mut reload, 40_000);
        bounded(&draw_pcm);
        bounded(&sheathe_pcm);
        bounded(&reload_pcm);
        assert!(draw.is_finished() && sheathe.is_finished() && reload.is_finished());
        assert_ne!(draw_pcm, sheathe_pcm);
        assert_ne!(draw_pcm, reload_pcm);

        let mut wood = WeaponHandlingGenerator::new(
            WeaponHandlingConfig {
                material: WeaponHandlingMaterial::Wood,
                ..base
            },
            41,
        )
        .unwrap();
        let mut leather = WeaponHandlingGenerator::new(
            WeaponHandlingConfig {
                material: WeaponHandlingMaterial::Leather,
                ..base
            },
            41,
        )
        .unwrap();
        let wood_pcm = render(&mut wood, 40_000);
        let leather_pcm = render(&mut leather, 40_000);
        bounded(&wood_pcm);
        bounded(&leather_pcm);
        assert!(wood.is_finished() && leather.is_finished());
        assert_ne!(draw_pcm, wood_pcm);
        assert_ne!(draw_pcm, leather_pcm);
    }

    #[test]
    fn weapon_handling_seed_and_controls_are_effective() {
        let base = WeaponHandlingConfig::default();
        let mut first = WeaponHandlingGenerator::new(base, 5).unwrap();
        let first_pcm = render(&mut first, 32_000);
        let mut same = WeaponHandlingGenerator::new(base, 5).unwrap();
        let mut other_seed = WeaponHandlingGenerator::new(base, 6).unwrap();
        assert_eq!(first_pcm, render(&mut same, 32_000));
        assert_ne!(first_pcm, render(&mut other_seed, 32_000));

        let mut lighter = WeaponHandlingGenerator::new(
            WeaponHandlingConfig {
                weight: 0.1,
                ..base
            },
            5,
        )
        .unwrap();
        let mut heavier = WeaponHandlingGenerator::new(
            WeaponHandlingConfig {
                weight: 0.95,
                ..base
            },
            5,
        )
        .unwrap();
        assert_ne!(first_pcm, render(&mut lighter, 32_000));
        assert_ne!(first_pcm, render(&mut heavier, 32_000));

        let mut slower =
            WeaponHandlingGenerator::new(WeaponHandlingConfig { speed: 0.1, ..base }, 5).unwrap();
        let mut faster = WeaponHandlingGenerator::new(
            WeaponHandlingConfig {
                speed: 0.95,
                ..base
            },
            5,
        )
        .unwrap();
        assert_ne!(first_pcm, render(&mut slower, 32_000));
        assert_ne!(first_pcm, render(&mut faster, 32_000));

        let mut quiet = WeaponHandlingGenerator::new(
            WeaponHandlingConfig {
                intensity: 0.2,
                ..base
            },
            5,
        )
        .unwrap();
        assert_ne!(first_pcm, render(&mut quiet, 32_000));
    }

    #[test]
    fn weapon_handling_rejects_invalid_configuration_and_odd_output() {
        assert!(matches!(
            WeaponHandlingGenerator::new(
                WeaponHandlingConfig {
                    speed: f32::NAN,
                    ..WeaponHandlingConfig::default()
                },
                0
            ),
            Err(ProceduralError::InvalidParameter)
        ));
        assert!(matches!(
            WeaponHandlingGenerator::new(
                WeaponHandlingConfig {
                    weight: 1.1,
                    ..WeaponHandlingConfig::default()
                },
                0
            ),
            Err(ProceduralError::InvalidParameter)
        ));
        assert!(matches!(
            WeaponHandlingGenerator::new(
                WeaponHandlingConfig {
                    intensity: f32::NAN,
                    ..WeaponHandlingConfig::default()
                },
                0
            ),
            Err(ProceduralError::InvalidParameter)
        ));
        assert!(matches!(
            WeaponHandlingGenerator::new(
                WeaponHandlingConfig {
                    sample_rate: 1_000,
                    ..WeaponHandlingConfig::default()
                },
                0
            ),
            Err(ProceduralError::InvalidSampleRate)
        ));
        let mut generator =
            WeaponHandlingGenerator::new(WeaponHandlingConfig::default(), 0).unwrap();
        assert_eq!(
            generator.render(&mut [0.0; 3]),
            Err(ProceduralError::OutputMustContainStereoFrames)
        );
    }
}
