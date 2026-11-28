//! More acoustically detailed procedural sound families.
//!
//! The original generators remain available through the retro module. This module provides
//! parallel realistic-style variants. They use deterministic synthesis and fixed-size state;
//! they do not load recordings or claim to be physical simulations.

use super::acoustics::AcousticProfile;
use super::{derive_seed, validate_rate, Rng, TAU};
pub use super::{
    BreakageConfig, BreakageMaterial, CharacterActionConfig, CharacterActionKind,
    CharacterVocalizationConfig, CharacterVocalizationKind, DamageEffectConfig, DamageEffectKind,
    EquipmentMovementConfig, EquipmentMovementKind, FootstepConfig, FootstepSurface, ImpactConfig,
    ImpactMaterial, MechanicalActionConfig, MechanicalActionKind, MechanicalActionMaterial,
    ObjectInteractionConfig, ObjectInteractionKind, ObjectInteractionMaterial, ProceduralError,
    ProceduralGenerator, RainConfig, ToolActionConfig, ToolActionKind, ToolActionMaterial,
    WeaponHandlingConfig, WeaponHandlingKind, WeaponHandlingMaterial, WeaponImpactConfig,
    WeaponImpactKind, WindConfig, MAX_PROCEDURAL_SAMPLE_RATE, MIN_PROCEDURAL_SAMPLE_RATE,
};

/// Deterministic acoustic texture layered over an existing procedural family.
///
/// This adapter adds subtle broadband detail, transient-sensitive texture, and a short natural
/// decay tail. Footsteps and weapon handling have dedicated implementations because those sounds
/// need family-specific physical cues.
pub struct AcousticGenerator<G> {
    inner: G,
    texture: AcousticTexture,
}

impl<G: ProceduralGenerator> AcousticGenerator<G> {
    /// Wrap an existing generator in the realistic acoustic-texture layer.
    ///
    /// The seed controls only the additional texture; it does not alter the wrapped generator's
    /// random stream or output.
    pub fn new(inner: G, sample_rate: u32, seed: u64) -> Result<Self, ProceduralError> {
        Self::with_stream(inner, sample_rate, seed, 0, 0.20)
    }

    fn with_stream(
        inner: G,
        sample_rate: u32,
        seed: u64,
        stream: u64,
        amount: f32,
    ) -> Result<Self, ProceduralError> {
        Self::with_profile(
            inner,
            sample_rate,
            seed,
            stream,
            amount,
            AcousticProfile::generic(),
        )
    }

    fn with_profile(
        inner: G,
        sample_rate: u32,
        seed: u64,
        stream: u64,
        amount: f32,
        profile: AcousticProfile,
    ) -> Result<Self, ProceduralError> {
        validate_rate(sample_rate)?;
        Ok(Self {
            inner,
            texture: AcousticTexture::new(sample_rate, seed, stream, amount, profile),
        })
    }

    /// Recover the wrapped generator.
    pub fn into_inner(self) -> G {
        self.inner
    }
}

impl<G: ProceduralGenerator> ProceduralGenerator for AcousticGenerator<G> {
    fn render(&mut self, output: &mut [f32]) -> Result<(), ProceduralError> {
        self.inner.render(output)?;
        let (frames, _) = output.as_chunks_mut::<2>();
        for frame in frames {
            let mid = (frame[0] + frame[1]) * 0.5;
            let side = (frame[0] - frame[1]) * 0.5;
            let enhanced = self.texture.process(mid);
            frame[0] = (enhanced + side).clamp(-1.0, 1.0);
            frame[1] = (enhanced - side).clamp(-1.0, 1.0);
        }
        Ok(())
    }

    fn is_finished(&self) -> bool {
        self.inner.is_finished() && self.texture.is_quiet()
    }
}

struct AcousticTexture {
    rng: Rng,
    low_noise: super::acoustics::OnePole,
    mid_noise: super::acoustics::OnePole,
    envelope: super::acoustics::TransientEnvelope,
    modes: [super::acoustics::ModalResonator; 3],
    profile: AcousticProfile,
    amount: f32,
}

impl AcousticTexture {
    fn new(
        sample_rate: u32,
        seed: u64,
        stream: u64,
        amount: f32,
        profile: AcousticProfile,
    ) -> Self {
        let rate = sample_rate as f32;
        let mut rng = Rng::new(derive_seed(seed, 0x4143_4F55, stream));
        let tune = |rng: &mut Rng, base: f32, spread: f32| {
            base * (1.0 - spread + rng.next_f32() * spread * 2.0)
        };
        let modes = std::array::from_fn(|index| {
            let spread = 0.035 + index as f32 * 0.025;
            let frequency = tune(&mut rng, profile.frequencies[index], spread);
            super::acoustics::ModalResonator::new(rate, frequency, profile.decays[index])
        });
        Self {
            rng,
            low_noise: super::acoustics::OnePole::low_pass(rate, 360.0),
            mid_noise: super::acoustics::OnePole::low_pass(rate, 2_600.0),
            envelope: super::acoustics::TransientEnvelope::new(rate, 0.003, 0.055),
            modes,
            profile,
            amount,
        }
    }

    fn process(&mut self, input: f32) -> f32 {
        let (level, rise) = self.envelope.tick(input);
        let noise = self.rng.next_f32() * 2.0 - 1.0;
        let low = self.low_noise.tick(noise);
        let mid = self.mid_noise.tick(noise);
        let mid_band = mid - low;
        let high_band = noise - mid;

        // Friction is a sustained, material-shaped noise bed; the transient term adds a sharper
        // contact edge without turning the whole event into broadband hiss.
        let friction = (mid_band * 0.68 + high_band * 0.32)
            * level
            * self.amount
            * (0.12 + self.profile.friction * 0.72)
            * (0.18 + rise * 2.2);
        let body = low * level * self.amount * 0.075 * self.profile.body;

        // Each attack excites material-tuned body and partial modes. This also responds to
        // secondary contacts already present in breakage, tool, and mechanism event profiles.
        let excitation = rise * 0.24 * self.profile.contact * (0.72 + noise * 0.28);
        let resonance = self.modes[0].excite(excitation) * 0.080
            + self.modes[1].excite(excitation) * 0.045
            + self.modes[2].excite(excitation) * 0.025;
        let resonance = resonance * self.profile.ring;

        let dry = input * (1.0 - self.amount * 0.08);
        let sample = dry + friction + body + resonance;
        (sample - sample * sample * sample * 0.10).clamp(-1.0, 1.0)
    }

    fn is_quiet(&self) -> bool {
        self.envelope.is_quiet() && self.modes.iter().all(|mode| mode.is_quiet())
    }
}

macro_rules! acoustic_family {
    ($name:ident, $inner:ident, $config:ident, $stream:expr, $amount:expr) => {
        #[doc = concat!(
                                            "Realistic-style acoustic variant of the original ",
                                            stringify!($inner),
                                            "."
                                        )]
        pub struct $name {
            inner: AcousticGenerator<super::$inner>,
        }

        impl $name {
            /// Construct the realistic-style family with deterministic acoustic texture.
            pub fn new(config: $config, seed: u64) -> Result<Self, ProceduralError> {
                let sample_rate = config.sample_rate;
                let inner = super::$inner::new(config, seed)?;
                let inner =
                    AcousticGenerator::with_stream(inner, sample_rate, seed, $stream, $amount)?;
                Ok(Self { inner })
            }
        }

        impl ProceduralGenerator for $name {
            fn render(&mut self, output: &mut [f32]) -> Result<(), ProceduralError> {
                self.inner.render(output)
            }

            fn is_finished(&self) -> bool {
                self.inner.is_finished()
            }
        }
    };
}

macro_rules! acoustic_family_with_profile {
    ($name:ident, $inner:ident, $config:ident, $stream:expr, $amount:expr, $profile_fn:ident) => {
        #[doc = concat!(
                    "Realistic-style acoustic variant of the original ",
                    stringify!($inner),
                    "."
                )]
        pub struct $name {
            inner: AcousticGenerator<super::$inner>,
        }

        impl $name {
            /// Construct the realistic-style family with a material-shaped acoustic response.
            pub fn new(config: $config, seed: u64) -> Result<Self, ProceduralError> {
                let sample_rate = config.sample_rate;
                let profile = $profile_fn(config);
                let inner = super::$inner::new(config, seed)?;
                let inner = AcousticGenerator::with_profile(
                    inner,
                    sample_rate,
                    seed,
                    $stream,
                    $amount,
                    profile,
                )?;
                Ok(Self { inner })
            }
        }

        impl ProceduralGenerator for $name {
            fn render(&mut self, output: &mut [f32]) -> Result<(), ProceduralError> {
                self.inner.render(output)
            }

            fn is_finished(&self) -> bool {
                self.inner.is_finished()
            }
        }
    };
}

acoustic_family!(WindGenerator, WindGenerator, WindConfig, 1, 0.07);
acoustic_family!(RainGenerator, RainGenerator, RainConfig, 2, 0.08);
acoustic_family_with_profile!(
    ImpactGenerator,
    ImpactGenerator,
    ImpactConfig,
    3,
    0.24,
    impact_acoustic_profile
);
acoustic_family_with_profile!(
    WeaponImpactGenerator,
    WeaponImpactGenerator,
    WeaponImpactConfig,
    4,
    0.20,
    weapon_impact_acoustic_profile
);
acoustic_family_with_profile!(
    BreakageGenerator,
    BreakageGenerator,
    BreakageConfig,
    5,
    0.28,
    breakage_acoustic_profile
);
acoustic_family!(
    CharacterActionGenerator,
    CharacterActionGenerator,
    CharacterActionConfig,
    6,
    0.20
);
acoustic_family!(
    EquipmentMovementGenerator,
    EquipmentMovementGenerator,
    EquipmentMovementConfig,
    7,
    0.30
);
acoustic_family_with_profile!(
    ObjectInteractionGenerator,
    ObjectInteractionGenerator,
    ObjectInteractionConfig,
    8,
    0.22,
    object_interaction_acoustic_profile
);
acoustic_family!(
    DamageEffectGenerator,
    DamageEffectGenerator,
    DamageEffectConfig,
    9,
    0.27
);
acoustic_family_with_profile!(
    ToolActionGenerator,
    ToolActionGenerator,
    ToolActionConfig,
    10,
    0.26,
    tool_action_acoustic_profile
);
acoustic_family!(
    CharacterVocalizationGenerator,
    CharacterVocalizationGenerator,
    CharacterVocalizationConfig,
    11,
    0.13
);
acoustic_family_with_profile!(
    MechanicalActionGenerator,
    MechanicalActionGenerator,
    MechanicalActionConfig,
    12,
    0.22,
    mechanical_action_acoustic_profile
);

fn impact_acoustic_profile(config: ImpactConfig) -> AcousticProfile {
    let body = 48.0 + config.resonance * 115.0;
    AcousticProfile::new(
        [body, body * 2.35, body * 5.4],
        [0.12, 0.075, 0.038],
        0.34 + (1.0 - config.resonance) * 0.32,
        0.65 + config.resonance * 0.55,
        0.9,
        0.65 + config.resonance * 0.55,
    )
}

fn weapon_impact_acoustic_profile(config: WeaponImpactConfig) -> AcousticProfile {
    let (base, overtone, material_decay, brightness, body) = config.material.profile();
    let decay = 1.0 / material_decay;
    AcousticProfile::new(
        [body, base, base * overtone],
        [decay * 1.35, decay, decay * 0.58],
        brightness,
        0.85,
        1.0,
        1.0,
    )
}

fn breakage_acoustic_profile(config: BreakageConfig) -> AcousticProfile {
    let (brightness, _, body, _, ring_decay, fragment, spread, fragment_decay) =
        config.material.profile();
    let decay = 1.0 / ring_decay;
    AcousticProfile::new(
        [body, fragment, fragment + spread],
        [decay * 1.35, decay, 1.0 / fragment_decay],
        brightness,
        0.9,
        1.05,
        1.0,
    )
}

fn object_interaction_acoustic_profile(config: ObjectInteractionConfig) -> AcousticProfile {
    let (frequency, decay_rate, friction, _) = config.material.profile();
    let decay = 1.0 / decay_rate;
    AcousticProfile::new(
        [frequency, frequency * 1.72, frequency * 2.65],
        [decay * 1.35, decay, decay * 0.55],
        friction,
        0.85,
        0.9,
        0.9,
    )
}

fn tool_action_acoustic_profile(config: ToolActionConfig) -> AcousticProfile {
    let (frequency, _, low_noise, high_noise, damping) = config.material.profile();
    let decay = 0.10 / damping;
    AcousticProfile::new(
        [frequency, frequency * 1.82, frequency * 2.75],
        [decay * 1.35, decay, decay * 0.58],
        high_noise,
        0.7 + low_noise * 0.35,
        0.95,
        0.9,
    )
}

fn mechanical_action_acoustic_profile(config: MechanicalActionConfig) -> AcousticProfile {
    let (frequency, _, low_noise, high_noise, damping) = config.material.profile();
    let decay = 0.095 / damping;
    AcousticProfile::new(
        [frequency, frequency * 1.85, frequency * 2.8],
        [decay * 1.35, decay, decay * 0.58],
        high_noise,
        0.7 + low_noise * 0.35,
        0.9,
        0.9,
    )
}

const WATER_DROPLETS: usize = 7;

#[derive(Clone, Copy)]
struct WaterDroplet {
    start: u32,
    duration: u32,
    amplitude: f32,
    frequency: f32,
    phase: f32,
    decay: f32,
}

impl WaterDroplet {
    const SILENT: Self = Self {
        start: u32::MAX,
        duration: 0,
        amplitude: 0.0,
        frequency: 0.0,
        phase: 0.0,
        decay: 1.0,
    };

    fn sample(self, position: u32, sample_rate: f32, noise: f32) -> f32 {
        if position < self.start {
            return 0.0;
        }
        let age = position - self.start;
        if age >= self.duration {
            return 0.0;
        }
        let elapsed = age as f32 / sample_rate;
        let envelope = (-elapsed * self.decay).exp();
        let burst = noise * (-elapsed * 95.0).exp();
        let ripple = (TAU * (self.phase + elapsed * self.frequency)).sin() * 0.16;
        self.amplitude * envelope * (burst * 0.72 + ripple)
    }
}

/// Finite deterministic footstep generator with material-shaped contact and natural texture.
///
/// Shallow water includes a broadband splash, displaced-water body, and independently timed
/// droplets. Other surfaces combine a damped body, filtered friction, and short material modes.
pub struct FootstepGenerator {
    sample_rate: f32,
    intensity: f32,
    brightness: f32,
    surface: FootstepSurface,
    position: u32,
    total: u32,
    rng: Rng,
    low_state: f32,
    body_state: f32,
    mode_phase_a: f32,
    mode_phase_b: f32,
    body_gain: f32,
    texture_gain: f32,
    mode_gain: f32,
    mode_frequency_a: f32,
    mode_frequency_b: f32,
    body_decay: f32,
    droplets: [WaterDroplet; WATER_DROPLETS],
}

impl FootstepGenerator {
    /// Construct a realistic-style generic soil footstep.
    pub fn new(config: FootstepConfig, seed: u64) -> Result<Self, ProceduralError> {
        Self::new_for_surface(config, FootstepSurface::Dirt, seed)
    }

    /// Construct a realistic-style footstep for a known ground surface.
    pub fn new_for_surface(
        config: FootstepConfig,
        surface: FootstepSurface,
        seed: u64,
    ) -> Result<Self, ProceduralError> {
        super::validate_rate(config.sample_rate)?;
        super::validate_unit(config.intensity)?;
        super::validate_unit(config.brightness)?;

        let (body_gain, texture_gain, mode_gain, mode_a, mode_b, body_decay) = match surface {
            FootstepSurface::Dirt => (0.34, 0.15, 0.12, 78.0, 163.0, 13.0),
            FootstepSurface::Grass => (0.22, 0.19, 0.055, 92.0, 218.0, 17.0),
            FootstepSurface::Gravel => (0.28, 0.27, 0.07, 118.0, 264.0, 15.0),
            FootstepSurface::Wood => (0.34, 0.075, 0.18, 142.0, 386.0, 12.0),
            FootstepSurface::Stone => (0.32, 0.085, 0.15, 168.0, 492.0, 16.0),
            FootstepSurface::Metal => (0.19, 0.055, 0.22, 318.0, 1_160.0, 21.0),
            FootstepSurface::ShallowWater => (0.28, 0.46, 0.035, 64.0, 174.0, 10.0),
            FootstepSurface::Snow => (0.25, 0.22, 0.035, 104.0, 232.0, 19.0),
        };

        let sample_rate = config.sample_rate as f32;
        let total_seconds = if surface == FootstepSurface::ShallowWater {
            0.42
        } else if surface == FootstepSurface::Gravel || surface == FootstepSurface::Snow {
            0.31
        } else {
            0.27
        };
        let surface_stream = match surface {
            FootstepSurface::Dirt => 0,
            FootstepSurface::Grass => 1,
            FootstepSurface::Gravel => 2,
            FootstepSurface::Wood => 3,
            FootstepSurface::Stone => 4,
            FootstepSurface::Metal => 5,
            FootstepSurface::ShallowWater => 6,
            FootstepSurface::Snow => 7,
        };
        let mut rng = Rng::new(derive_seed(seed, 0x5246_4F4F, surface_stream));
        let mode_phase_a = rng.next_f32();
        let mode_phase_b = rng.next_f32();
        let mode_frequency_a = mode_a * (0.94 + rng.next_f32() * 0.12);
        let mode_frequency_b = mode_b * (0.92 + rng.next_f32() * 0.16);
        let mut droplets = [WaterDroplet::SILENT; WATER_DROPLETS];
        if surface == FootstepSurface::ShallowWater {
            for droplet in &mut droplets {
                let start = (sample_rate * (0.012 + rng.next_f32() * 0.17)) as u32;
                let duration = (sample_rate * (0.035 + rng.next_f32() * 0.055)) as u32;
                *droplet = WaterDroplet {
                    start,
                    duration,
                    amplitude: 0.055 + rng.next_f32() * 0.075,
                    frequency: 420.0 + rng.next_f32() * 1_850.0,
                    phase: rng.next_f32(),
                    decay: 32.0 + rng.next_f32() * 42.0,
                };
            }
        }

        Ok(Self {
            sample_rate,
            intensity: config.intensity,
            brightness: config.brightness,
            surface,
            position: 0,
            total: (sample_rate * total_seconds) as u32,
            rng,
            low_state: 0.0,
            body_state: 0.0,
            mode_phase_a,
            mode_phase_b,
            body_gain,
            texture_gain,
            mode_gain,
            mode_frequency_a,
            mode_frequency_b,
            body_decay,
            droplets,
        })
    }
}

impl ProceduralGenerator for FootstepGenerator {
    fn render(&mut self, output: &mut [f32]) -> Result<(), ProceduralError> {
        super::validate_output(output)?;
        let (frames, _) = output.as_chunks_mut::<2>();
        for frame in frames {
            if self.position >= self.total {
                frame.fill(0.0);
                continue;
            }

            let elapsed = self.position as f32 / self.sample_rate;
            let noise = self.rng.next_f32() * 2.0 - 1.0;
            self.low_state += 0.055 * (noise - self.low_state);
            self.body_state += 0.014 * (noise - self.body_state);
            let high = noise - self.low_state;
            let low = self.body_state;
            let attack = 1.0 - (-elapsed * 240.0).exp();
            let body_envelope = attack * (-elapsed * self.body_decay).exp();
            let mode_a =
                (TAU * self.mode_phase_a).sin() * (-elapsed * (self.body_decay * 1.8)).exp();
            let mode_b =
                (TAU * self.mode_phase_b).sin() * (-elapsed * (self.body_decay * 2.7)).exp();

            let sample = if self.surface == FootstepSurface::ShallowWater {
                let splash_envelope = attack * (-elapsed * 18.0).exp();
                let splash = high * splash_envelope * (0.30 + self.brightness * 0.20);
                let displacement = low * splash_envelope * 0.34;
                let droplets: f32 = self
                    .droplets
                    .iter()
                    .map(|drop| drop.sample(self.position, self.sample_rate, high))
                    .sum();
                let footfall = low * body_envelope * 0.32;
                splash + displacement + droplets + footfall
            } else {
                let friction =
                    high * body_envelope * self.texture_gain * (0.65 + self.brightness * 0.70);
                let body = low * body_envelope * self.body_gain;
                let modes = (mode_a + mode_b * 0.55) * body_envelope * self.mode_gain;
                let granular = if matches!(
                    self.surface,
                    FootstepSurface::Gravel | FootstepSurface::Snow
                ) {
                    high * (-elapsed * 24.0).exp() * self.texture_gain * 0.20
                } else {
                    0.0
                };
                body + friction + modes + granular
            };

            frame[0] = (sample * self.intensity).clamp(-1.0, 1.0);
            frame[1] = frame[0];
            self.mode_phase_a =
                (self.mode_phase_a + self.mode_frequency_a / self.sample_rate).rem_euclid(1.0);
            self.mode_phase_b =
                (self.mode_phase_b + self.mode_frequency_b / self.sample_rate).rem_euclid(1.0);
            self.position += 1;
        }
        Ok(())
    }

    fn is_finished(&self) -> bool {
        self.position >= self.total
    }
}

const WEAPON_CONTACTS: usize = 4;

#[derive(Clone, Copy)]
struct WeaponContact {
    start: u32,
    duration: u32,
    amplitude: f32,
    frequency_a: f32,
    frequency_b: f32,
    phase_a: f32,
    phase_b: f32,
    decay: f32,
}

impl WeaponContact {
    const SILENT: Self = Self {
        start: u32::MAX,
        duration: 0,
        amplitude: 0.0,
        frequency_a: 0.0,
        frequency_b: 0.0,
        phase_a: 0.0,
        phase_b: 0.0,
        decay: 1.0,
    };

    fn sample(self, position: u32, sample_rate: f32, noise: f32) -> f32 {
        if position < self.start {
            return 0.0;
        }
        let age = position - self.start;
        if age >= self.duration {
            return 0.0;
        }
        let elapsed = age as f32 / sample_rate;
        let envelope = (-elapsed * self.decay).exp();
        let mode_a = (TAU * (self.phase_a + elapsed * self.frequency_a)).sin() * 0.26;
        let mode_b = (TAU * (self.phase_b + elapsed * self.frequency_b)).sin() * 0.14;
        let transient = noise * (-elapsed * 115.0).exp() * 0.62;
        self.amplitude * envelope * (transient + mode_a + mode_b)
    }
}

/// Finite deterministic draw, sheathe, and reload generator with friction and discrete contacts.
///
/// The realistic version does not use a continuous pitch sweep. Drawing and sheathing use
/// differently timed scabbard friction, while fixed-frequency, quickly damped modes provide
/// small fitting and seating contacts.
pub struct WeaponHandlingGenerator {
    sample_rate: f32,
    intensity: f32,
    weight: f32,
    speed: f32,
    material: WeaponHandlingMaterial,
    kind: WeaponHandlingKind,
    position: u32,
    total: u32,
    rng: Rng,
    low_state: f32,
    mid_state: f32,
    deep_state: f32,
    contacts: [WeaponContact; WEAPON_CONTACTS],
}

impl WeaponHandlingGenerator {
    /// Construct a realistic-style weapon draw, sheathe, or reload sound.
    pub fn new(config: WeaponHandlingConfig, seed: u64) -> Result<Self, ProceduralError> {
        super::validate_rate(config.sample_rate)?;
        super::validate_unit(config.intensity)?;
        super::validate_unit(config.weight)?;
        super::validate_unit(config.speed)?;

        let sample_rate = config.sample_rate as f32;
        let (base_duration, _, _, _, _, event_count, _) = config.kind.profile();
        let duration = base_duration * (1.3 - config.speed * 0.6);
        let total = (sample_rate * duration) as u32;
        let mut rng = Rng::new(derive_seed(seed, 0x5257_4844, config.kind as u64));
        let (base_frequency_a, base_frequency_b, amplitude, decay) = match config.material {
            WeaponHandlingMaterial::Metal => (1_180.0, 2_240.0, 0.19, 34.0),
            WeaponHandlingMaterial::Wood => (286.0, 540.0, 0.13, 24.0),
            WeaponHandlingMaterial::Leather => (420.0, 760.0, 0.075, 29.0),
        };
        let heft = 1.12 - config.weight * 0.26;
        let positions = match config.kind {
            WeaponHandlingKind::Draw => [0.035, 0.90, 0.0, 0.0],
            WeaponHandlingKind::Sheathe => [0.17, 0.93, 0.0, 0.0],
            WeaponHandlingKind::Reload => [0.09, 0.36, 0.65, 0.91],
        };
        let mut contacts = [WeaponContact::SILENT; WEAPON_CONTACTS];
        for index in 0..event_count {
            let jitter = (rng.next_f32() - 0.5) * 0.035;
            let progress = (positions[index] + jitter).clamp(0.015, 0.98);
            let start = (total as f32 * progress) as u32;
            contacts[index] = WeaponContact {
                start,
                duration: (sample_rate * (0.045 + rng.next_f32() * 0.035)) as u32,
                amplitude: amplitude * (0.72 + rng.next_f32() * 0.50) * (0.9 + config.weight * 0.2),
                frequency_a: base_frequency_a * heft * (0.9 + rng.next_f32() * 0.2),
                frequency_b: base_frequency_b * heft * (0.9 + rng.next_f32() * 0.2),
                phase_a: rng.next_f32(),
                phase_b: rng.next_f32(),
                decay: decay * (0.8 + rng.next_f32() * 0.4),
            };
        }

        Ok(Self {
            sample_rate,
            intensity: config.intensity,
            weight: config.weight,
            speed: config.speed,
            material: config.material,
            kind: config.kind,
            position: 0,
            total,
            rng,
            low_state: 0.0,
            mid_state: 0.0,
            deep_state: 0.0,
            contacts,
        })
    }

    fn friction_envelope(&self, progress: f32) -> f32 {
        match self.kind {
            WeaponHandlingKind::Draw => {
                smoothstep(0.045, 0.20, progress) * (1.0 - smoothstep(0.76, 0.93, progress))
            }
            WeaponHandlingKind::Sheathe => {
                smoothstep(0.13, 0.29, progress) * (1.0 - smoothstep(0.73, 0.89, progress))
            }
            WeaponHandlingKind::Reload => pulse(progress, 0.11, 0.055)
                .max(pulse(progress, 0.39, 0.06))
                .max(pulse(progress, 0.68, 0.055)),
        }
    }
}

impl ProceduralGenerator for WeaponHandlingGenerator {
    fn render(&mut self, output: &mut [f32]) -> Result<(), ProceduralError> {
        super::validate_output(output)?;
        let (frames, _) = output.as_chunks_mut::<2>();
        for frame in frames {
            if self.position >= self.total {
                frame.fill(0.0);
                continue;
            }
            let progress = self.position as f32 / self.total as f32;
            let noise = self.rng.next_f32() * 2.0 - 1.0;
            self.low_state += 0.12 * (noise - self.low_state);
            self.mid_state += 0.038 * (self.low_state - self.mid_state);
            self.deep_state += 0.009 * (noise - self.deep_state);
            let high = noise - self.low_state;
            let mid = self.low_state - self.mid_state;
            let friction_envelope = self.friction_envelope(progress);
            let (high_gain, mid_gain, body_gain) = match self.material {
                WeaponHandlingMaterial::Metal => (0.22, 0.085, 0.12),
                WeaponHandlingMaterial::Wood => (0.095, 0.13, 0.20),
                WeaponHandlingMaterial::Leather => (0.055, 0.17, 0.16),
            };
            let friction = (high * high_gain + mid * mid_gain)
                * friction_envelope
                * (0.78 + self.speed * 0.36);
            let body_envelope = 1.0 - smoothstep(0.78, 1.0, progress);
            let body = self.deep_state * body_gain * (0.8 + self.weight * 0.4) * body_envelope;
            let contacts: f32 = self
                .contacts
                .iter()
                .map(|contact| contact.sample(self.position, self.sample_rate, noise))
                .sum();
            let sample =
                (friction + body * (1.0 - friction_envelope * 0.55) + contacts) * self.intensity;
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

fn smoothstep(start: f32, end: f32, value: f32) -> f32 {
    let t = ((value - start) / (end - start)).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

fn pulse(value: f32, center: f32, width: f32) -> f32 {
    let distance = (value - center) / width;
    (-distance * distance * 4.0).exp()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn render<G: ProceduralGenerator + ?Sized>(generator: &mut G, frames: usize) -> Vec<f32> {
        let mut output = vec![0.0; frames * 2];
        generator.render(&mut output).unwrap();
        output
    }

    fn bounded(samples: &[f32]) {
        assert!(samples
            .iter()
            .all(|sample| sample.is_finite() && sample.abs() <= 1.0));
    }

    #[test]
    fn acoustic_layer_is_deterministic_and_keeps_render_block_continuity() {
        let config = ImpactConfig {
            sample_rate: 48_000,
            ..ImpactConfig::default()
        };
        let make = || {
            AcousticGenerator::new(
                super::super::ImpactGenerator::new(config, 77).unwrap(),
                config.sample_rate,
                77,
            )
            .unwrap()
        };
        let mut whole = make();
        let mut split = make();
        let expected = render(&mut whole, 24_000);
        let mut actual = render(&mut split, 333);
        actual.extend(render(&mut split, 7_777));
        actual.extend(render(&mut split, 15_890));
        assert_eq!(expected, actual);
        bounded(&expected);

        let mut retro = super::super::ImpactGenerator::new(config, 77).unwrap();
        assert_ne!(expected, render(&mut retro, 24_000));
    }

    #[test]
    fn material_profiles_change_the_response_without_changing_seeded_block_continuity() {
        let config = ImpactConfig {
            sample_rate: 48_000,
            ..ImpactConfig::default()
        };
        let source = || super::super::ImpactGenerator::new(config, 0xC07A).unwrap();
        let wood = AcousticProfile::new(
            [145.0, 430.0, 900.0],
            [0.12, 0.09, 0.055],
            0.22,
            0.9,
            0.8,
            0.75,
        );
        let metal = AcousticProfile::new(
            [260.0, 1_850.0, 3_180.0],
            [0.22, 0.19, 0.10],
            0.92,
            0.7,
            1.1,
            1.2,
        );
        let make = |profile| {
            AcousticGenerator::with_profile(
                source(),
                config.sample_rate,
                0xC07A,
                0xA11,
                0.24,
                profile,
            )
            .unwrap()
        };
        let mut wood_whole = make(wood);
        let mut wood_split = make(wood);
        let wood_samples = render(&mut wood_whole, 24_000);
        let mut split_samples = render(&mut wood_split, 777);
        split_samples.extend(render(&mut wood_split, 9_223));
        split_samples.extend(render(&mut wood_split, 14_000));
        assert_eq!(wood_samples, split_samples);

        let mut metal_generator = make(metal);
        let metal_samples = render(&mut metal_generator, 24_000);
        bounded(&wood_samples);
        bounded(&metal_samples);
        assert_ne!(wood_samples, metal_samples);
    }

    #[test]
    fn material_resonance_keeps_a_tail_after_the_source_event() {
        let config = ImpactConfig {
            sample_rate: 48_000,
            ..ImpactConfig::default()
        };
        let profile = AcousticProfile::new(
            [120.0, 510.0, 1_420.0],
            [0.22, 0.18, 0.12],
            0.5,
            1.0,
            1.0,
            1.0,
        );
        let inner = super::super::ImpactGenerator::new(config, 0x7A11).unwrap();
        let mut generator = AcousticGenerator::with_profile(
            inner,
            config.sample_rate,
            0x7A11,
            0xA12,
            0.24,
            profile,
        )
        .unwrap();
        let tail = render(&mut generator, 24_000);
        assert!(tail[36_480..].iter().any(|sample| sample.abs() > 0.00001));
        assert!(!generator.is_finished());
        render(&mut generator, 96_000);
        assert!(generator.is_finished());
    }

    #[test]
    fn realistic_footsteps_are_seeded_distinct_and_water_has_a_splash_tail() {
        let config = FootstepConfig {
            sample_rate: 48_000,
            ..FootstepConfig::default()
        };
        let mut water_a =
            FootstepGenerator::new_for_surface(config, FootstepSurface::ShallowWater, 91).unwrap();
        let mut water_b =
            FootstepGenerator::new_for_surface(config, FootstepSurface::ShallowWater, 91).unwrap();
        let mut dirt =
            FootstepGenerator::new_for_surface(config, FootstepSurface::Dirt, 91).unwrap();
        let water = render(&mut water_a, 24_000);
        assert_eq!(water, render(&mut water_b, 24_000));
        let soil = render(&mut dirt, 24_000);
        bounded(&water);
        bounded(&soil);
        assert_ne!(water, soil);
        assert!(water[7_000..16_000]
            .iter()
            .any(|sample| sample.abs() > 0.0005));
    }

    #[test]
    fn realistic_weapon_handling_separates_draw_and_sheathe() {
        let config = WeaponHandlingConfig {
            sample_rate: 48_000,
            ..WeaponHandlingConfig::default()
        };
        let mut draw = WeaponHandlingGenerator::new(
            WeaponHandlingConfig {
                kind: WeaponHandlingKind::Draw,
                ..config
            },
            0xD12A,
        )
        .unwrap();
        let mut sheathe = WeaponHandlingGenerator::new(
            WeaponHandlingConfig {
                kind: WeaponHandlingKind::Sheathe,
                ..config
            },
            0xD12A,
        )
        .unwrap();
        let draw_samples = render(&mut draw, 40_000);
        let sheathe_samples = render(&mut sheathe, 40_000);
        bounded(&draw_samples);
        bounded(&sheathe_samples);
        assert_ne!(draw_samples, sheathe_samples);
        assert!(draw.is_finished() && sheathe.is_finished());
    }

    #[test]
    fn realistic_footsteps_and_weapon_handling_are_block_deterministic() {
        let footstep_config = FootstepConfig {
            sample_rate: 48_000,
            ..FootstepConfig::default()
        };
        let make_footstep = || {
            FootstepGenerator::new_for_surface(
                footstep_config,
                FootstepSurface::ShallowWater,
                0x5A1A,
            )
            .unwrap()
        };
        let mut footstep_whole = make_footstep();
        let mut footstep_split = make_footstep();
        let expected_footstep = render(&mut footstep_whole, 24_000);
        let mut actual_footstep = render(&mut footstep_split, 1_337);
        actual_footstep.extend(render(&mut footstep_split, 8_191));
        actual_footstep.extend(render(&mut footstep_split, 14_472));
        assert_eq!(expected_footstep, actual_footstep);

        let handling_config = WeaponHandlingConfig {
            sample_rate: 48_000,
            ..WeaponHandlingConfig::default()
        };
        let make_handling = || {
            WeaponHandlingGenerator::new(
                WeaponHandlingConfig {
                    kind: WeaponHandlingKind::Draw,
                    ..handling_config
                },
                0x5A1A,
            )
            .unwrap()
        };
        let mut handling_whole = make_handling();
        let mut handling_split = make_handling();
        let expected_handling = render(&mut handling_whole, 26_000);
        let mut actual_handling = render(&mut handling_split, 1_337);
        actual_handling.extend(render(&mut handling_split, 8_191));
        actual_handling.extend(render(&mut handling_split, 16_472));
        assert_eq!(expected_handling, actual_handling);
        bounded(&expected_footstep);
        bounded(&expected_handling);
        assert!(footstep_whole.is_finished());
        assert!(handling_whole.is_finished());
    }

    #[test]
    fn realistic_footsteps_and_weapon_handling_reject_invalid_configs() {
        assert!(matches!(
            FootstepGenerator::new_for_surface(
                FootstepConfig {
                    sample_rate: 1_000,
                    ..FootstepConfig::default()
                },
                FootstepSurface::ShallowWater,
                0,
            ),
            Err(ProceduralError::InvalidSampleRate)
        ));
        assert!(matches!(
            WeaponHandlingGenerator::new(
                WeaponHandlingConfig {
                    speed: f32::NAN,
                    ..WeaponHandlingConfig::default()
                },
                0,
            ),
            Err(ProceduralError::InvalidParameter)
        ));
    }

    #[test]
    fn realistic_family_adapters_construct_and_render() {
        let mut wind = WindGenerator::new(
            WindConfig {
                sample_rate: 48_000,
                ..WindConfig::default()
            },
            1,
        )
        .unwrap();
        let mut rain = RainGenerator::new(
            RainConfig {
                sample_rate: 48_000,
                ..RainConfig::default()
            },
            2,
        )
        .unwrap();
        let mut impact = ImpactGenerator::new(
            ImpactConfig {
                sample_rate: 48_000,
                ..ImpactConfig::default()
            },
            3,
        )
        .unwrap();
        let mut weapon_impact = WeaponImpactGenerator::new(
            WeaponImpactConfig {
                sample_rate: 48_000,
                ..WeaponImpactConfig::default()
            },
            4,
        )
        .unwrap();
        let mut breakage = BreakageGenerator::new(
            BreakageConfig {
                sample_rate: 48_000,
                ..BreakageConfig::default()
            },
            5,
        )
        .unwrap();
        let mut action = CharacterActionGenerator::new(
            CharacterActionConfig {
                sample_rate: 48_000,
                ..CharacterActionConfig::default()
            },
            6,
        )
        .unwrap();
        let mut equipment = EquipmentMovementGenerator::new(
            EquipmentMovementConfig {
                sample_rate: 48_000,
                ..EquipmentMovementConfig::default()
            },
            7,
        )
        .unwrap();
        let mut object = ObjectInteractionGenerator::new(
            ObjectInteractionConfig {
                sample_rate: 48_000,
                ..ObjectInteractionConfig::default()
            },
            8,
        )
        .unwrap();
        let mut damage = DamageEffectGenerator::new(
            DamageEffectConfig {
                sample_rate: 48_000,
                ..DamageEffectConfig::default()
            },
            9,
        )
        .unwrap();
        let mut tool = ToolActionGenerator::new(
            ToolActionConfig {
                sample_rate: 48_000,
                ..ToolActionConfig::default()
            },
            10,
        )
        .unwrap();
        let mut voice = CharacterVocalizationGenerator::new(
            CharacterVocalizationConfig {
                sample_rate: 48_000,
                ..CharacterVocalizationConfig::default()
            },
            11,
        )
        .unwrap();
        let mut mechanism = MechanicalActionGenerator::new(
            MechanicalActionConfig {
                sample_rate: 48_000,
                ..MechanicalActionConfig::default()
            },
            12,
        )
        .unwrap();

        let mut generators: [&mut dyn ProceduralGenerator; 12] = [
            &mut wind,
            &mut rain,
            &mut impact,
            &mut weapon_impact,
            &mut breakage,
            &mut action,
            &mut equipment,
            &mut object,
            &mut damage,
            &mut tool,
            &mut voice,
            &mut mechanism,
        ];
        for generator in &mut generators {
            let output = render(*generator, 24_000);
            bounded(&output);
            assert!(output.iter().any(|sample| sample.abs() > 0.0001));
        }
    }
}
