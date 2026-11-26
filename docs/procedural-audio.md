# Procedural sound generation

Verdandi provides deterministic, engine-independent procedural generators for game audio: effects and environmental sounds that complement the music engine. Its scope includes reusable gameplay sound concepts such as footsteps, weapon impacts and clashes, material response, character actions, destruction, and environmental ambience. The built-in generators include wind, rain, generic and surface-specific footsteps, generic impacts, weapon-contact sounds, material-specific breakage, non-vocal character movement effects, and weapon draw/sheathe/reload handling; additional families are tracked in [issue #28](https://github.com/Golden-Bastion/verdandi/issues/28).

Generators render interleaved stereo floating-point PCM into caller-owned buffers. Generation is separate from playback: use an `AudioClip` for finite sounds and the bounded stream API for continuous environments. The consuming game decides when a sound occurs and how it relates to game entities; Verdandi provides the reusable sound-generation behavior without depending on that game's event or world model.

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

The original \`FootstepGenerator::new\` constructor remains material-neutral and retains its existing sound profile. When the host knows the ground material, use \`FootstepGenerator::new_for_surface\` with one of the built-in \`FootstepSurface\` profiles: dirt, grass, gravel, wood, stone, metal, shallow water, or snow. The same intensity and brightness controls apply; brightness adjusts the high-frequency texture. The surface affects body resonance, friction, and the contact transient.

\`\`\`rust
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
\`\`\`

Surface profiles are stylized sound-design starting points, not recordings or physical contact simulations. The host decides which surface applies, and may continue using recorded footsteps for distinctive footwear, movement styles, or highly realistic material detail.


## Generating a weapon impact

Use `WeaponImpactGenerator` for a strike, blade clash, parry, block, or projectile impact. Its material profile shapes the transient and resonant body; force and intensity control the contact's weight and output level. The seed provides repeatable variation.

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

`ImpactMaterial` provides metal, wood, stone, flesh, armor, and a shield profile with a resonant face and brighter rim response. `WeaponImpactKind` distinguishes strikes, blade clashes, parries, blocks, and projectile impacts. These profiles are stylized starting points; tune them against actual playback before treating them as final sound assets.

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
| `WeaponImpactGenerator` | Finite | intensity, force, material (including shield), contact kind (including projectile impact) |
| `BreakageGenerator` | Finite | intensity, force, size, material |
| `CharacterActionGenerator` | Finite | intensity, force, weight, action kind |
| `WeaponHandlingGenerator` | Finite | intensity, weight, speed, material, handling kind |
| `WindGenerator` | Continuous | intensity, brightness |
| `RainGenerator` | Continuous | density, brightness, intensity |

These are initial sound-design building blocks, not a closed catalogue of supported effects. Further families may include vocal pain/exertion and death sounds, blood-splatter effects, bone breaks, and more object-specific destruction sounds. Shared controls should express useful sound-design properties such as material, force, size, resonance, intensity, and deterministic variation where appropriate. Their output may be stylized; do not claim physical accuracy without evidence.


## Application independence

Game-oriented APIs are appropriate in Verdandi. A `SwordClashGenerator` or a reusable material-impact descriptor is not inherently application-specific. An API that looks up a particular game's combat entities, subscribes to its event bus, or encodes its world rules would be. Keep event timing and gameplay meaning in the host; keep reusable sound design and audio rendering in Verdandi.
