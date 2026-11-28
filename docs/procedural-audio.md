# Procedural sound generation

Verdandi provides two deterministic, engine-independent procedural sound sets for effects and environmental sounds. The Retro set preserves the original stylized generators and their seeded output. The parallel Realistic set adds acoustic texture to the existing families, dedicated material-contact synthesis for footsteps and weapon handling, and material-tuned responses for impacts, breakage, object interactions, tools, and mechanisms. Both sets cover the same reusable gameplay concepts: wind, rain, footsteps, impacts, breakage, character actions and vocalizations, weapon/equipment handling, object interactions, tools, and mechanisms.

Generators render interleaved stereo floating-point PCM into caller-owned buffers. Generation is separate from playback: use an `AudioClip` for finite sounds and the bounded stream API for continuous environments. The consuming game decides when a sound occurs and how it relates to game entities; Verdandi provides the reusable sound-generation behavior without depending on that game's event or world model.

## Layered acoustic synthesis

The Realistic family uses shared synthesis primitives rather than adding unrelated oscillators to each generator. Its texture stage separates low-, mid-, and high-frequency noise, tracks transient energy, and excites a small bank of damped resonant modes. Material-aware families tune those modes and their decay from the selected material profile: impacts, weapon contacts, breakage, object interactions, tool actions, and mechanical actions receive distinct body and partial frequencies, friction levels, and ringing behaviour. Secondary contacts prepared by those event generators can excite the same material response, so fragments, repeated tool contacts, and latch/hinge contacts do not all share one generic overlay. These components are internal building blocks and do not change the Retro family or its seeded output.

The synthesis remains deliberately bounded: filters and resonators keep fixed state, are prepared when the generator is constructed, and require no allocation or dynamic dispatch while rendering. This is a bounded acoustic approximation, not a complete physical model: it does not model collision geometry or solve physical material properties. The separate `EnvironmentalAcoustics` processor adds distance attenuation and high-frequency loss, obstruction filtering, and configurable early reflections with a damped feedback tail.


## Applying environmental acoustics

Wrap any procedural generator in `EnvironmentalAcoustics` when the host knows the source's distance, obstruction, and surrounding space. The processor is independent of game entities and can be used with either sound family or a custom `ProceduralGenerator`.

```rust
use verdandi::prelude::{
    EnvironmentalAcoustics, EnvironmentalAcousticsConfig, ImpactConfig, ImpactGenerator,
    ProceduralGenerator,
};

let source = ImpactGenerator::new(
    ImpactConfig {
        sample_rate: 48_000,
        ..ImpactConfig::default()
    },
    0x5eed,
)?;

let mut sound = EnvironmentalAcoustics::new(
    source,
    48_000,
    EnvironmentalAcousticsConfig {
        distance: 0.65,
        occlusion: 0.2,
        room_size: 0.8,
        absorption: 0.35,
        wet: 0.55,
    },
)?;
let mut block = vec![0.0; 512 * 2];
sound.render(&mut block)?;
```

All controls are normalized to `0.0..=1.0`:

- `distance` reduces direct level and progressively removes high-frequency energy.
- `occlusion` adds stronger attenuation and low-pass filtering for obstructed paths.
- `room_size` sets the spacing of three early-reflection taps; zero disables room reflections.
- `absorption` damps high frequencies in the reflection feedback and shortens the tail.
- `wet` controls the reflected signal level independently of the direct path.

Use `set_config` to update a moving source or a changing environment without replacing the generator. Direct and reflection gains transition smoothly; filter cutoffs and reflection timings update immediately, so gradual host-side parameter changes are preferable for continuous movement. `is_finished` includes the environmental tail, allowing finite sources to finish after their reflections have decayed.

The processor preserves stereo and uses a fixed delay buffer prepared with the generator. Rendering performs no allocation or I/O. It is a compact early-reflection model, not convolution reverb, geometric ray tracing, or a substitute for authored impulse responses. The Realistic audition pack includes distant, occluded, small-room, and cavern comparisons.

## Choosing the Retro or Realistic sound set

The original generators remain available unchanged through `verdandi::procedural::retro`. Existing top-level names such as `FootstepGenerator` and `WeaponHandlingGenerator` remain available and retain their original constructors and output; they are the original Retro implementations.

Use `verdandi::procedural::realistic` for the matching Realistic family names. The realistic footstep generator builds a damped contact body and material-shaped friction; shallow water adds a broadband splash, water displacement, and independently timed droplets. The realistic weapon-handling generator uses action-specific scabbard friction and short fitting contacts. Other Realistic families retain their original event shape under deterministic broadband texture; impacts, breakage, object interactions, tools, and mechanisms also receive material-tuned body and partial modes, friction, and decay.

The Realistic set is still synthesized audio, not recordings or a complete physical simulation. The Realistic audition pack now includes material comparisons for weapon contacts, breakage, object interactions, tools, and mechanisms. The two audition packs are intended to make the difference easy to compare:

```sh
cargo run --release --no-default-features --example procedural_audition
cargo run --release --no-default-features --example procedural_realistic_audition
```

A side-by-side footstep example:

```rust
use verdandi::prelude::{FootstepConfig, FootstepSurface, ProceduralGenerator};
use verdandi::procedural::{realistic, retro};

let config = FootstepConfig::default();
let seed = 0x5eed;

let mut retro_step =
    retro::FootstepGenerator::new_for_surface(config, FootstepSurface::ShallowWater, seed)?;
let mut realistic_step =
    realistic::FootstepGenerator::new_for_surface(config, FootstepSurface::ShallowWater, seed)?;

let mut retro_pcm = vec![0.0; 48_000 * 2 / 2];
let mut realistic_pcm = vec![0.0; 48_000 * 2 / 2];
retro_step.render(&mut retro_pcm)?;
realistic_step.render(&mut realistic_pcm)?;
```

For other families, the realistic variants use the same configuration type and constructor shape as their Retro counterpart. The generic `realistic::AcousticGenerator` adapter is also public for applying the acoustic-texture layer to a custom implementation of `ProceduralGenerator`.

---

## Generating a finite effect

The initial finite generators are `FootstepGenerator` and `ImpactGenerator`. Each emits one sound and then silence. The seed controls its repeatable variation.

```rust
use std::sync::Arc;
use verdandi::prelude::{AudioClip, FootstepConfig, FootstepGenerator, ProceduralGenerator};

fn make_footstep(seed: u64) -> Result<Arc<AudioClip>, Box<dyn std::error::Error>> {
    let config = FootstepConfig::default();
    let mut generator = FootstepGenerator::new(config, seed)?;
    let frames = (config.sample_rate as usize * 22) / 100;
    let mut pcm = vec![0.0; frames * 2];
    generator.render(&mut pcm)?;
    Ok(Arc::new(AudioClip::from_samples(config.sample_rate, 2, pcm)?))
}
```

Submit the resulting clip through the existing effects playback API. Keep the owning `Arc<AudioClip>` in the host's asset catalogue while it may be queued or playing. See [sound effects and ambience](sound-effects.md) for playback and ownership rules.


### Choosing a footstep surface

The original `FootstepGenerator::new` constructor remains material-neutral and retains its existing sound profile. When the host knows the ground material, use `FootstepGenerator::new_for_surface` with one of the built-in `FootstepSurface` profiles: dirt, grass, gravel, wood, stone, metal, shallow water, or snow. The same intensity and brightness controls apply; brightness adjusts the high-frequency texture. The surface affects body resonance, friction, and the contact transient.

```rust
use std::sync::Arc;
use verdandi::prelude::{
    AudioClip, FootstepConfig, FootstepGenerator, FootstepSurface, ProceduralGenerator,
};

fn make_gravel_footstep(seed: u64) -> Result<Arc<AudioClip>, Box<dyn std::error::Error>> {
    let config = FootstepConfig::default();
    let mut generator =
        FootstepGenerator::new_for_surface(config, FootstepSurface::Gravel, seed)?;
    let frames = (config.sample_rate as usize * 22) / 100;
    let mut pcm = vec![0.0; frames * 2];
    generator.render(&mut pcm)?;
    Ok(Arc::new(AudioClip::from_samples(config.sample_rate, 2, pcm)?))
}
```

Surface profiles are stylized sound-design starting points, not recordings or physical contact simulations. The host decides which surface applies, and may continue using recorded footsteps for distinctive footwear, movement styles, or highly realistic material detail.


## Generating a weapon impact

Use `WeaponImpactGenerator` for a strike, blade clash, parry, block, projectile impact, shield bash, armor strike, or projectile deflection. Its material profile shapes the transient and resonant body; force and intensity control the contact's weight and output level. The seed provides repeatable variation.

```rust
use std::sync::Arc;
use verdandi::prelude::{
    AudioClip, ImpactMaterial, ProceduralGenerator, WeaponImpactConfig,
    WeaponImpactGenerator, WeaponImpactKind,
};

fn make_sword_clash(seed: u64) -> Result<Arc<AudioClip>, Box<dyn std::error::Error>> {
    let config = WeaponImpactConfig {
        sample_rate: 48_000,
        intensity: 0.85,
        force: 0.8,
        material: ImpactMaterial::Metal,
        kind: WeaponImpactKind::BladeClash,
    };
    let mut generator = WeaponImpactGenerator::new(config, seed)?;
    let frames = (config.sample_rate as usize * 58) / 100;
    let mut pcm = vec![0.0; frames * 2];
    generator.render(&mut pcm)?;
    Ok(Arc::new(AudioClip::from_samples(config.sample_rate, 2, pcm)?))
}
```

`ImpactMaterial` provides metal, wood, stone, flesh, armor, and a shield profile with a resonant face and brighter rim response. `WeaponImpactKind` distinguishes strikes, blade clashes, parries, blocks, projectile impacts, shield bashes, armor strikes, and projectile deflections. These profiles are stylized starting points; tune them against actual playback before treating them as final sound assets.

A shield block combines the shield material profile with the existing block contact shape:

```rust
use verdandi::prelude::{ImpactMaterial, WeaponImpactConfig, WeaponImpactGenerator, WeaponImpactKind};

let config = WeaponImpactConfig {
    material: ImpactMaterial::Shield,
    kind: WeaponImpactKind::Block,
    force: 0.9,
    ..WeaponImpactConfig::default()
};
let seed = 0x51A7;
let shield_block = WeaponImpactGenerator::new(config, seed)?;
```

To model a projectile hitting a surface, use `WeaponImpactKind::ProjectileImpact` and select the target material. The host remains responsible for choosing the correct material and force from its own gameplay state.

Submit the resulting clip through the existing effects playback API. See [sound effects and ambience](sound-effects.md) for playback and ownership rules.

## Generating material breakage

Use `BreakageGenerator` for a one-shot fracture sound followed by a small, deterministic set of fragment events. `BreakageMaterial` provides wood, glass, stone, ceramic, and metal profiles. Size changes the duration, resonance, and fragment spread; force changes the fracture transient and overall energy; intensity controls output level.

```rust
use std::sync::Arc;
use verdandi::prelude::{
    AudioClip, BreakageConfig, BreakageGenerator, BreakageMaterial, ProceduralGenerator,
};

fn make_glass_break(seed: u64) -> Result<Arc<AudioClip>, Box<dyn std::error::Error>> {
    let config = BreakageConfig {
        sample_rate: 48_000,
        intensity: 0.9,
        force: 0.85,
        size: 0.65,
        material: BreakageMaterial::Glass,
    };
    let mut generator = BreakageGenerator::new(config, seed)?;
    let frames = (config.sample_rate as usize * 9) / 10;
    let mut pcm = vec![0.0; frames * 2];
    generator.render(&mut pcm)?;
    Ok(Arc::new(AudioClip::from_samples(config.sample_rate, 2, pcm)?))
}
```

Submit the resulting clip through the existing effects playback API. The generator prepares its fixed fragment events when it is constructed and does not allocate during rendering. Its profiles are stylized sound-design starting points, not physical fracture simulations; audition and tune them against playback before treating them as final assets.



## Generating non-vocal damage effects

Use `DamageEffectGenerator` for three stylized, non-vocal damage textures:
`DamageEffectKind::BloodSplatter`, `DamageEffectKind::BoneBreak`, and `DamageEffectKind::BluntTrauma`.
`force` changes the event energy, `size` changes the duration, resonance, and spread of secondary contacts, and
`intensity` controls the final level.

```rust
use std::sync::Arc;
use verdandi::prelude::{
    AudioClip, DamageEffectConfig, DamageEffectGenerator, DamageEffectKind, ProceduralGenerator,
};

fn make_bone_break(seed: u64) -> Result<Arc<AudioClip>, Box<dyn std::error::Error>> {
    let config = DamageEffectConfig {
        sample_rate: 48_000,
        intensity: 0.75,
        force: 0.85,
        size: 0.55,
        kind: DamageEffectKind::BoneBreak,
    };
    let mut generator = DamageEffectGenerator::new(config, seed)?;
    let frames = (config.sample_rate as usize * 7) / 10;
    let mut pcm = vec![0.0; frames * 2];
    generator.render(&mut pcm)?;
    Ok(Arc::new(AudioClip::from_samples(config.sample_rate, 2, pcm)?))
}
```

The splatter profile combines a wet broadband texture with a fixed sequence of small droplet-like
contacts. The bone-break profile uses a sharper fracture transient and shorter secondary cracks.
The blunt-trauma profile emphasizes a low body thump with a short dull crack. These are stylized
effect layers, not anatomical simulations. For nonverbal voice fallback, use the
`CharacterVocalizationGenerator` described below; prefer supplied recordings when a recognizable
character performance is available. Prepare the PCM outside the playback callback and submit it
through the existing effects path.


## Generating synthetic character vocalizations

Use `CharacterVocalizationGenerator` as a deterministic fallback when a suitable host-provided
recording is unavailable. It synthesizes nonverbal `Inhale`, `Exhale`, `Exertion`, `Pain`, and
`DeathGroan` cues from a glottal pulse, three vocal-tract formant resonators, and seeded breath
noise. The base pitch, effort, breathiness, and relative vocal-tract weight shape the result.

```rust
use std::sync::Arc;
use verdandi::prelude::{
    AudioClip, CharacterVocalizationConfig, CharacterVocalizationGenerator,
    CharacterVocalizationKind, ProceduralGenerator,
};

fn make_pain_fallback(seed: u64) -> Result<Arc<AudioClip>, Box<dyn std::error::Error>> {
    let config = CharacterVocalizationConfig {
        sample_rate: 48_000,
        intensity: 0.72,
        pitch_hz: 155.0,
        effort: 0.9,
        breathiness: 0.38,
        weight: 0.55,
        kind: CharacterVocalizationKind::Pain,
    };
    let mut generator = CharacterVocalizationGenerator::new(config, seed)?;
    let frames = (config.sample_rate as usize * 9) / 10;
    let mut pcm = vec![0.0; frames * 2];
    generator.render(&mut pcm)?;
    Ok(Arc::new(AudioClip::from_samples(config.sample_rate, 2, pcm)?))
}
```

The host should select a recording first when one exists and use this generator when it does not.
Verdandi does not own an asset catalogue or decide whether a character has a voice asset. These
synthetic cues are intentionally nonverbal and generic: they are not intelligible speech, voice
cloning, or realistic actor performances. Render the finite clip outside the playback callback
and submit it through the existing effects path.

## Generating character movement sounds

Use `CharacterActionGenerator` for non-vocal character movement effects: jumps, landings, and falls. `force` controls the action's strength, while `weight` shifts the body resonance and secondary impacts toward a heavier or lighter character. The seed provides repeatable variation. This generator does not synthesize speech, breathing, or realistic vocalizations.

```rust
use std::sync::Arc;
use verdandi::prelude::{
    AudioClip, CharacterActionConfig, CharacterActionGenerator, CharacterActionKind,
    ProceduralGenerator,
};

fn make_landing(seed: u64) -> Result<Arc<AudioClip>, Box<dyn std::error::Error>> {
    let config = CharacterActionConfig {
        sample_rate: 48_000,
        intensity: 0.85,
        force: 0.8,
        weight: 0.7,
        kind: CharacterActionKind::Landing,
    };
    let mut generator = CharacterActionGenerator::new(config, seed)?;
    let frames = (config.sample_rate as usize * 4) / 10;
    let mut pcm = vec![0.0; frames * 2];
    generator.render(&mut pcm)?;
    Ok(Arc::new(AudioClip::from_samples(config.sample_rate, 2, pcm)?))
}
```

A jump produces a short rising movement burst, a landing emphasizes a weighted body impact, and a fall uses a longer movement texture with a fixed number of secondary impacts. These are stylized effects rather than physical body simulations. Use host-provided or sampled audio when a specific character identity or convincing vocal performance is required.


## Generating weapon-handling sounds

Use `WeaponHandlingGenerator` for generic draw, sheathe, and reload sounds. The dominant material profile can be metal, wood, or leather. `weight` shifts the resonant body toward heavier or lighter equipment, while `speed` changes the action duration and tightness of its contact sequence. The seed provides repeatable variation.

```rust
use std::sync::Arc;
use verdandi::prelude::{
    AudioClip, ProceduralGenerator, WeaponHandlingConfig, WeaponHandlingGenerator,
    WeaponHandlingKind, WeaponHandlingMaterial,
};

fn make_weapon_draw(seed: u64) -> Result<Arc<AudioClip>, Box<dyn std::error::Error>> {
    let config = WeaponHandlingConfig {
        sample_rate: 48_000,
        intensity: 0.8,
        weight: 0.7,
        speed: 0.6,
        material: WeaponHandlingMaterial::Metal,
        kind: WeaponHandlingKind::Draw,
    };
    let mut generator = WeaponHandlingGenerator::new(config, seed)?;
    let frames = (config.sample_rate as usize * 7) / 10;
    let mut pcm = vec![0.0; frames * 2];
    generator.render(&mut pcm)?;
    Ok(Arc::new(AudioClip::from_samples(config.sample_rate, 2, pcm)?))
}
```

These profiles are intentionally generic. A consuming game may select a material profile and seed from its own weapon data, or use recorded assets where a particular weapon mechanism needs a distinctive identity. Verdandi does not infer weapon classes or own the host's gameplay state. The generator prepares its small contact sequence at construction and does not allocate while rendering.


## Generating equipment-movement sounds

Use `EquipmentMovementGenerator` for short movement sounds from clothing and carried equipment. `EquipmentMovementKind` offers three broad profiles: `ClothRustle`, `GearJostle`, and `ArmorRattle`. The `weight` control shifts the resonant body, while `activity` controls movement energy and timing. The seed provides repeatable variation.

```rust
use std::sync::Arc;
use verdandi::prelude::{
    AudioClip, EquipmentMovementConfig, EquipmentMovementGenerator, EquipmentMovementKind,
    ProceduralGenerator,
};

fn make_armor_rattle(seed: u64) -> Result<Arc<AudioClip>, Box<dyn std::error::Error>> {
    let config = EquipmentMovementConfig {
        sample_rate: 48_000,
        intensity: 0.75,
        weight: 0.8,
        activity: 0.7,
        kind: EquipmentMovementKind::ArmorRattle,
    };
    let mut generator = EquipmentMovementGenerator::new(config, seed)?;
    let frames = (config.sample_rate as usize * 5) / 10;
    let mut pcm = vec![0.0; frames * 2];
    generator.render(&mut pcm)?;
    Ok(Arc::new(AudioClip::from_samples(config.sample_rate, 2, pcm)?))
}
```

These are stylized movement textures, not simulations of particular clothing, armor construction, or equipment attachment points. The host decides when the sound occurs and which profile best fits its equipment. Use supplied recordings where a specific outfit or gear set needs a recognizable identity.


## Generating object-interaction sounds

Use `ObjectInteractionGenerator` for generic object handling and simple mechanism actions. Its
`ObjectInteractionKind` profiles cover `Pickup`, `Place`, `Drop`, `Open`, `Close`, and
`Toggle`. `ObjectInteractionMaterial` selects a dominant wood, metal, stone, glass, or ceramic
response. `weight` shifts the resonant body, `speed` changes action timing and texture, and
`intensity` controls output level.

```rust
use std::sync::Arc;
use verdandi::prelude::{
    AudioClip, ObjectInteractionConfig, ObjectInteractionGenerator, ObjectInteractionKind,
    ObjectInteractionMaterial, ProceduralGenerator,
};

fn make_container_open(seed: u64) -> Result<Arc<AudioClip>, Box<dyn std::error::Error>> {
    let config = ObjectInteractionConfig {
        sample_rate: 48_000,
        intensity: 0.8,
        weight: 0.65,
        speed: 0.5,
        material: ObjectInteractionMaterial::Wood,
        kind: ObjectInteractionKind::Open,
    };
    let mut generator = ObjectInteractionGenerator::new(config, seed)?;
    let frames = (config.sample_rate as usize * 9) / 10;
    let mut pcm = vec![0.0; frames * 2];
    generator.render(&mut pcm)?;
    Ok(Arc::new(AudioClip::from_samples(config.sample_rate, 2, pcm)?))
}
```

`Drop` uses a small fixed sequence of contacts to suggest a bounded bounce. `Open`, `Close`, and
`Toggle` provide generic movement/contact shapes; they do not model particular hinges, locks,
switch designs, or rigid-body physics. The host chooses the action and material from its own
object state, prepares the clip outside the playback callback, and submits it through the normal
effects path. Use supplied recordings when a door, mechanism, tool, or object needs a recognizable
identity or more detailed moving parts.


## Generating tool-action sounds

Use `ToolActionGenerator` for generic chopping, digging, scraping, and sawing sounds.
`ToolActionMaterial` selects the target response for earth, wood, stone, or metal.
`force` controls contact energy, `weight` shifts the resonant body, and `speed` changes
the action duration and repeated-friction timing.

```rust
use std::sync::Arc;
use verdandi::prelude::{
    AudioClip, ProceduralGenerator, ToolActionConfig, ToolActionGenerator, ToolActionKind,
    ToolActionMaterial,
};

fn make_wood_chop(seed: u64) -> Result<Arc<AudioClip>, Box<dyn std::error::Error>> {
    let config = ToolActionConfig {
        sample_rate: 48_000,
        intensity: 0.8,
        force: 0.85,
        weight: 0.7,
        speed: 0.6,
        material: ToolActionMaterial::Wood,
        kind: ToolActionKind::Chop,
    };
    let mut generator = ToolActionGenerator::new(config, seed)?;
    let frames = (config.sample_rate as usize * 7) / 10;
    let mut pcm = vec![0.0; frames * 2];
    generator.render(&mut pcm)?;
    Ok(Arc::new(AudioClip::from_samples(config.sample_rate, 2, pcm)?))
}
```

`Chop` emphasizes a strong contact and short follow-through; `Dig` combines gritty movement with
several contacts; `Scrape` sustains friction across the action; and `Saw` uses repeated friction
strokes. The material profiles are stylized sound-design starting points, not physical tool or
surface simulations. Use supplied recordings where a specific tool, engine, or machine needs a
recognizable identity. Generate clips outside the playback callback and submit them through the
existing effects path.


## Generating hinge and mechanism sounds

Use `MechanicalActionGenerator` for generic hinge, latch, lock, and crank cues. The supported
actions are `HingeOpen`, `HingeClose`, `LatchEngage`, `LatchRelease`, `LockTurn`, and
`Crank`. `MechanicalActionMaterial` selects a wood, metal, or stone response. Force controls
contact energy, weight shifts the resonant body, and speed changes duration and contact timing.

```rust
use std::sync::Arc;
use verdandi::prelude::{
    AudioClip, MechanicalActionConfig, MechanicalActionGenerator, MechanicalActionKind,
    MechanicalActionMaterial, ProceduralGenerator,
};

fn make_latch_click(seed: u64) -> Result<Arc<AudioClip>, Box<dyn std::error::Error>> {
    let config = MechanicalActionConfig {
        sample_rate: 48_000,
        intensity: 0.8,
        force: 0.65,
        weight: 0.4,
        speed: 0.7,
        material: MechanicalActionMaterial::Metal,
        kind: MechanicalActionKind::LatchEngage,
    };
    let mut generator = MechanicalActionGenerator::new(config, seed)?;
    let frames = (config.sample_rate as usize * 4) / 10;
    let mut pcm = vec![0.0; frames * 2];
    generator.render(&mut pcm)?;
    Ok(Arc::new(AudioClip::from_samples(config.sample_rate, 2, pcm)?))
}
```

These profiles provide generic movement, friction, and contact layers; they do not simulate a real
hinge, lock, or machine. Use a host recording when a particular door, latch, or mechanism needs
recognizable hardware detail.

## Building a procedural audition pack

The `procedural_audition` and `procedural_realistic_audition` examples render the complete Retro and Realistic family sets to standalone WAV files so their profiles can be compared outside a game or audio device. They cover surface footsteps, generic impacts, all weapon-contact kinds, breakage materials, damage effects, nonverbal vocalizations, character movement, weapon handling, equipment movement, object interactions, tool actions, mechanical actions, wind, and rain.

Run it without enabling Bevy:

```sh
cargo run --release --no-default-features --example procedural_audition
```

The Retro default output directory is `target/retro-audition-pack`; the Realistic pack uses `target/realistic-audition-pack`. Both accept a custom directory and base seed:

```sh
cargo run --release --no-default-features --example procedural_audition -- ./my-retro-pack 1234
cargo run --release --no-default-features --example procedural_realistic_audition -- ./my-realistic-pack 1234
```

The example uses 48 kHz stereo, signed 16-bit PCM WAV for every file. Finite generators are rendered until they report completion; wind and rain are each rendered for five seconds using one persistent generator instance. Each file gets a deterministic seed derived from the supplied base seed and its position in the export sequence. The generated README lists each file, profile, and seed.

Generator default control values are retained, with the selected profile or representative material changed where needed to create comparisons. Files are not independently normalized, so relative output levels remain part of the audition. PCM conversion only bounds samples to the representable WAV range. The pack is a listening aid, not a declaration that any generated profile is final or acoustically approved.

## Generating continuous ambience

The initial continuous generators are `WindGenerator` and `RainGenerator`. Their state persists between calls, so render successive blocks using the same generator instance.

```rust
use verdandi::prelude::{ProceduralGenerator, RainConfig, RainGenerator};

let mut rain = RainGenerator::new(RainConfig::default(), 0x5eed)?;
let mut block = vec![0.0; 4_096 * 2];

// Call repeatedly on a host worker. Reuse the buffer between blocks.
rain.render(&mut block)?;
```

To play the generated blocks, create a `StreamBuffer` at the same sample rate, then write each rendered block using its `StreamWriter`. The producer must handle partial writes and backpressure without blocking the audio callback. Keep the writer and stream handle alive according to the stream lifecycle described in [sound effects and ambience](sound-effects.md).

## Rendering contract

- Buffers contain interleaved stereo frames: left sample followed by right sample. Their length must be even.
- Generators write into caller-owned memory and do not allocate during `render`.
- Continuous generators preserve state across arbitrary block boundaries.
- Finite generators return `is_finished() == true` after their sound has ended and write silence for subsequent frames.
- Supported sample rates are 8,000 through 192,000 Hz.
- Normalized configuration values must be finite and within 0.0..=1.0.
- Identical seeds, configurations, sample rates, and render sequences produce identical output.
- The initial generators produce centered mono-compatible content duplicated to stereo. Spatial placement and more advanced stereo fields can be layered on by the host or added to future generators.

Generation is not intended to run in the playback callback. Although rendering itself is allocation-free, generate on a worker thread and pass the completed blocks to `AudioClip` or `StreamWriter`. Do not perform asset loading, decoding, waiting, or stream backpressure handling on the audio callback.

## Available generators

| Generator | Type | Main controls |
|---|---|---|
| `FootstepGenerator` | Finite | intensity, brightness, optional surface profile |
| `ImpactGenerator` | Finite | intensity, resonance |
| `WeaponImpactGenerator` | Finite | intensity, force, material, contact kind (including specialized shield/armor/projectile contacts) |
| `BreakageGenerator` | Finite | intensity, force, size, material |
| `DamageEffectGenerator` | Finite | intensity, force, size, damage effect kind |
| `CharacterVocalizationGenerator` | Finite | intensity, pitch, effort, breathiness, vocal-tract weight, vocalization kind |
| `CharacterActionGenerator` | Finite | intensity, force, weight, action kind |
| `WeaponHandlingGenerator` | Finite | intensity, weight, speed, material, handling kind |
| `EquipmentMovementGenerator` | Finite | intensity, weight, activity, movement kind |
| `ObjectInteractionGenerator` | Finite | intensity, weight, speed, material, object action |
| `ToolActionGenerator` | Finite | intensity, force, weight, speed, target material, tool action |
| `MechanicalActionGenerator` | Finite | intensity, force, weight, speed, mechanism material, action kind |
| `WindGenerator` | Continuous | intensity, brightness |
| `RainGenerator` | Continuous | density, brightness, intensity |

Each family is available through both `procedural::retro` and `procedural::realistic`. These are sound-design building blocks, not a closed catalogue of supported effects. The first expansion now covers nonverbal voice fallback, specialized combat contacts, generic tool actions, and common hinge/lock mechanisms. Further specialized assets or event profiles should be added where real integration needs justify them. Shared controls should express useful sound-design properties such as material, force, size, resonance, intensity, and deterministic variation where appropriate. The Retro set is intentionally stylized. The Realistic set adds material-shaped cues and acoustic texture, but does not claim physically accurate simulation.


## Application independence

Game-oriented APIs are appropriate in Verdandi. A `SwordClashGenerator` or a reusable material-impact descriptor is not inherently application-specific. An API that looks up a particular game's combat entities, subscribes to its event bus, or encodes its world rules would be. Keep event timing and gameplay meaning in the host; keep reusable sound design and audio rendering in Verdandi.
