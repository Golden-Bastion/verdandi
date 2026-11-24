# Procedural music

Verdandi creates a four-bar arrangement from a seed, a musical theme, and a preset. Each preset defines both composition behaviour and synthesis character. It does not load recorded audio unless you explicitly assign SFZ instruments.

## Presets and pace

The public enum variants retain their original Rust names; their display names are shown in the first column.

| Display name | Rust variant | Character |
|---|---|---|
| Ethereal | **MusicPreset::MythicAmbient** | Spacious, string-led exploration music |
| Warbound | **MusicPreset::HeavyMetal** | Driving riffs, aggressive guitars, and percussion |
| Orchestral | **MusicPreset::LegendaryMetal** | Heavy guitar music with orchestral and choir-like layers |
| Contemplative | **MusicPreset::ClassicalAmbient** | Restrained, reflective, string-led music |

**MusicPace::Standard** is the default. **MusicPace::Fast** increases the tempo/rhythmic density of Warbound and Orchestral. Ethereal and Contemplative retain their slower tempo ranges regardless of pace.

## Configure a track

**MusicConfig** contains the initial state of a music engine. This example pins the theme while allowing the preset to generate its own arrangement:

~~~rust
use verdandi::prelude::{
    MusicConfig, MusicPace, MusicPreset, ScaleKind, ThemeOverrides,
};

let config = MusicConfig {
    seed: 1234,
    preset: MusicPreset::MythicAmbient,
    pace: MusicPace::Standard,
    theme: ThemeOverrides {
        tonic_pitch_class: Some(2), // D
        scale: Some(ScaleKind::Dorian),
        bpm: Some(84.0),
        progression: Some([0, 4, 5, 3]),
    },
    auto_intensity: false,
    intensity: 0.7,
    ..MusicConfig::default()
};
~~~

Theme overrides are optional and independent:

- **tonic_pitch_class:** pitch class from 0 (C) through 11 (B).
- **scale:** one of Minor, Dorian, Major, Mixolydian, or Phrygian.
- **bpm:** requested tempo; the generated theme clamps it to 40–220 BPM.
- **progression:** four scale-degree roots, one per bar.

When an override is absent, Verdandi derives that part of the theme from the seed and preset. The same seed, preset, pace, and overrides reproduce the same musical choices.

## Layers

The composer has eight independently controllable layers:

| Layer | Role |
|---|---|
| **Drums** | Percussion |
| **Bass** | Low-end foundation |
| **Pad** | Sustained harmonic bed |
| **Arp** | Arpeggiated motion |
| **Lead** | Main melodic voice |
| **RhythmGuitar** | Rhythmic guitar layer |
| **Strings** | Bowed-string-like layer |
| **Choir** | Choir-like layer |

Not every preset uses every layer:

| Preset | Layers used |
|---|---|
| Ethereal | Bass, Pad, Arp, Lead, Strings |
| Warbound | Drums, Bass, Lead, RhythmGuitar |
| Orchestral | All eight |
| Contemplative | Bass, Pad, Arp, Lead, Strings |

A layer must both be enabled and used by the selected preset to contribute. Its intensity threshold must also be reached. The thresholds are: Pad 0.00; Bass and Strings 0.20; RhythmGuitar 0.30; Drums and Choir 0.35; Arp 0.55; Lead 0.75.

## Intensity

Automatic intensity is enabled by default and evolves across **auto_intensity_period_loops** (8 loops by default). In automatic mode, the configured **intensity** value is not the active control value.

To take direct control, disable automatic intensity and set a value from 0.0 to 1.0:

~~~rust
audio.set_auto_intensity(false);
audio.set_intensity(0.85);
audio.set_layer_enabled(Layer::Lead, false);
~~~

The example above uses the Bevy **AudioController**; the same operations are available on **MusicShared** in a custom host. Values are clamped to the supported range.

Use **MusicStatus** to inspect the active seed, preset, pace, loop index, step, BPM, tonic, scale, effective intensity, per-layer action, enabled state, and audible state. **status.key_name()** converts the tonic to a note name.

## Runtime controls

With Bevy, obtain the **AudioController** resource in a system. The same resource is also exported under the backwards-compatible alias **MusicController**.

~~~rust
use bevy::prelude::Res;
use std::time::Duration;
use verdandi::prelude::{
    AudioController, Layer, MusicPace, MusicPreset, MusicStinger,
};

fn enter_combat(audio: Res<AudioController>) {
    audio.set_auto_intensity(false);
    audio.set_intensity(1.0);
    audio.set_preset(MusicPreset::HeavyMetal, MusicPace::Fast);
    audio.play_stinger(MusicStinger::CombatStart);
    audio.fade_to(0.8, Duration::from_secs(1));
    audio.set_layer_enabled(Layer::RhythmGuitar, false);
}
~~~

The control methods behave as follows:

- **set_seed(seed):** queues a new seed for the next four-bar loop boundary.
- **set_preset(preset, pace):** queues a style change for the next bar, then crossfades over two seconds.
- **set_preset_immediate(preset, pace):** starts that two-second crossfade as soon as the audio callback processes the request.
- **play_stinger(kind):** plays a short synthesized cue over the continuing music. The available cues are CombatStart, Victory, BossReveal, and Transition. A newer cue replaces a cue that is still playing.
- **pause()** and **resume():** pause and resume musical transport without advancing the position while paused.
- **set_volume(gain):** immediately sets master music gain.
- **fade_to(gain, duration):** linearly fades master music gain. A new fade request replaces the active fade from its current gain.

Music gain covers the music engine's output, including stingers and preset crossfades. Muting the Music bus is separate from pausing transport: mute produces silence while musical time continues; pause freezes musical time.

## Determinism notes

Seeded composition is a core contract. Verdandi derives separate random streams for theme selection and per-layer/per-loop choices so that unrelated layer activity does not casually perturb another layer's random sequence. A layer may repeat, mutate, or regenerate its pattern at a four-bar loop boundary. Seed changes are also applied at a loop boundary.

Determinism applies to musical decisions for the same configuration. It does not make different host decoders, output devices, sample libraries, or user-side processing equivalent.
