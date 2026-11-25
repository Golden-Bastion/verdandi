# Procedural sound generation

Verdandi provides deterministic, engine-independent procedural generators for game audio: effects and environmental sounds that complement the music engine. Its scope includes reusable gameplay sound concepts such as footsteps, weapon impacts and clashes, material response, character actions, destruction, and environmental ambience. The built-in generators include wind, rain, footsteps, generic impacts, and weapon-contact sounds; additional families are tracked in [issue #28](https://github.com/Golden-Bastion/verdandi/issues/28).

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

## Generating a weapon impact

Use `WeaponImpactGenerator` for a strike, blade clash, parry, or block. Its material profile shapes the transient and resonant body; force and intensity control the contact's weight and output level. The seed provides repeatable variation.

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

`ImpactMaterial` currently provides metal, wood, stone, flesh, and armor profiles. `WeaponImpactKind` distinguishes strikes, blade clashes, parries, and blocks. These profiles are stylized starting points; tune them against actual playback before treating them as final sound assets.

Submit the resulting clip through the existing effects playback API. See [sound effects and ambience](sound-effects.md) for playback and ownership rules.

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
| `FootstepGenerator` | Finite | intensity, brightness |
| `ImpactGenerator` | Finite | intensity, resonance |
| `WeaponImpactGenerator` | Finite | intensity, force, material, contact kind |
| `WindGenerator` | Continuous | intensity, brightness |
| `RainGenerator` | Continuous | density, brightness, intensity |

These are initial sound-design building blocks, not a closed catalogue of supported effects. Further families may include shield and armor-specific contacts, character exertion and death sounds, blood-splatter effects, material-specific breakage, and debris. Shared controls should express useful sound-design properties such as material, force, size, resonance, intensity, and deterministic variation where appropriate. Their output may be stylized; do not claim physical accuracy without evidence.


## Application independence

Game-oriented APIs are appropriate in Verdandi. A `SwordClashGenerator` or a reusable material-impact descriptor is not inherently application-specific. An API that looks up a particular game's combat entities, subscribes to its event bus, or encodes its world rules would be. Keep event timing and gameplay meaning in the host; keep reusable sound design and audio rendering in Verdandi.
