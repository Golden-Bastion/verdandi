//! Backwards-compatible retro/stylized procedural sound family set.
//!
//! These are the original generators and constructors. Their algorithms, defaults, seeded output,
//! and public names remain unchanged. New integrations can select this namespace explicitly.

pub use super::{
    BreakageConfig, BreakageGenerator, BreakageMaterial, CharacterActionConfig,
    CharacterActionGenerator, CharacterActionKind, CharacterVocalizationConfig,
    CharacterVocalizationGenerator, CharacterVocalizationKind, DamageEffectConfig,
    DamageEffectGenerator, DamageEffectKind, EquipmentMovementConfig, EquipmentMovementGenerator,
    EquipmentMovementKind, FootstepConfig, FootstepGenerator, FootstepSurface, ImpactConfig,
    ImpactGenerator, ImpactMaterial, MechanicalActionConfig, MechanicalActionGenerator,
    MechanicalActionKind, MechanicalActionMaterial, ObjectInteractionConfig,
    ObjectInteractionGenerator, ObjectInteractionKind, ObjectInteractionMaterial, ProceduralError,
    ProceduralGenerator, RainConfig, RainGenerator, ToolActionConfig, ToolActionGenerator,
    ToolActionKind, ToolActionMaterial, WeaponHandlingConfig, WeaponHandlingGenerator,
    WeaponHandlingKind, WeaponHandlingMaterial, WeaponImpactConfig, WeaponImpactGenerator,
    WeaponImpactKind, WindConfig, WindGenerator, MAX_PROCEDURAL_SAMPLE_RATE,
    MIN_PROCEDURAL_SAMPLE_RATE,
};
