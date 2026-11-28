//! # verdandi
//!
//! A reusable, deterministic game-audio engine. It provides layered procedural music, procedural
//! gameplay sound generation, host-provided PCM playback, and bounded audio streaming.
//!
//! The core is independent of any particular game's entities, event system, world simulation, or
//! asset catalogue. Gameplay code decides when sounds occur; Verdandi provides reusable audio
//! generation and playback behavior. The same seed, configuration, and render sequence reproduce
//! the same generated output.
//!
//! * `rng`, `theory`, `composer`, `synth`, `engine`, `procedural`, `sound`: engine-agnostic core (no Bevy dependency).
//! * `plugin` (feature `bevy-plugin`, on by default): Bevy audio source + unified audio controller.

pub mod composer;
pub mod engine;
pub mod preset;
pub mod procedural;
pub mod rng;
pub mod sfz;
pub mod sound;
pub mod synth;
pub mod theory;

/// Selectable procedural sound families (retro and realistic-style).
pub use procedural::{realistic, retro};

#[cfg(feature = "bevy-plugin")]
pub mod plugin;

pub mod prelude {
    pub use crate::composer::{Layer, LayerAction, LoopPlan, NoteEvent};
    pub use crate::engine::{
        Engine, InstrumentConfig, MusicConfig, MusicShared, MusicStatus, MusicStinger,
    };
    pub use crate::preset::{MusicPace, MusicPreset};
    pub use crate::procedural::{realistic, retro};
    pub use crate::procedural::realistic::{
        EnvironmentalAcoustics, EnvironmentalAcousticsConfig,
    };
    pub use crate::procedural::{
        BreakageConfig, BreakageGenerator, BreakageMaterial, CharacterActionConfig,
        CharacterActionGenerator, CharacterActionKind, CharacterVocalizationConfig,
        CharacterVocalizationGenerator, CharacterVocalizationKind, DamageEffectConfig,
        DamageEffectGenerator, DamageEffectKind, EquipmentMovementConfig,
        EquipmentMovementGenerator, EquipmentMovementKind, FootstepConfig, FootstepGenerator,
        FootstepSurface, ImpactConfig, ImpactGenerator, ImpactMaterial, MechanicalActionConfig,
        MechanicalActionGenerator, MechanicalActionKind, MechanicalActionMaterial,
        ObjectInteractionConfig, ObjectInteractionGenerator, ObjectInteractionKind,
        ObjectInteractionMaterial, ProceduralError, ProceduralGenerator, RainConfig, RainGenerator,
        ToolActionConfig, ToolActionGenerator, ToolActionKind, ToolActionMaterial,
        WeaponHandlingConfig, WeaponHandlingGenerator, WeaponHandlingKind, WeaponHandlingMaterial,
        WeaponImpactConfig, WeaponImpactGenerator, WeaponImpactKind, WindConfig, WindGenerator,
        MAX_PROCEDURAL_SAMPLE_RATE, MIN_PROCEDURAL_SAMPLE_RATE,
    };
    pub use crate::sfz::{
        parse_sfz, SfzDefinition, SfzError, SfzInstrument, SfzLoopMode, SfzRegion, SfzSampler,
    };
    pub use crate::sound::{
        AudioBus, AudioClip, AudioClipError, AudioStream, LoopKey, SoundCategory,
        SoundCommandError, SoundController, SoundMixer, SoundMixerConfig, SoundMixerConfigError,
        SoundMixerCreateError, SoundMixerStatus, StreamBuffer, StreamBufferConfig,
        StreamBufferConfigError, StreamWriteError, StreamWriter, MAX_SOUND_VOICES,
        MAX_STREAM_BUFFER_FRAMES,
    };
    pub use crate::theory::{ScaleKind, Theme, ThemeOverrides};

    #[cfg(feature = "bevy-plugin")]
    pub use crate::plugin::{
        AudioController, MusicController, MusicTrack, ProceduralMusic, ProceduralMusicPlugin,
    };
}
