# Verdandi

Verdandi is a reusable Rust game-audio engine. It combines deterministic, seed-driven procedural music and game-oriented sound generation with an engine-independent mixer for host-provided effects and environmental ambience.

Music is generated at runtime by default; no recorded music library is required. Optional SFZ instruments can add recorded timbres using sample libraries supplied by the application. The same seed and configuration reproduce the same musical decisions.

## Capabilities

- Four musical identities: Ethereal, Warbound, Orchestral, and Contemplative.
- Layered four-bar composition, seeded theme selection, automatic or manual intensity, and per-layer controls.
- Runtime seed and preset changes, crossfades, transport pause/resume, gain controls, and synthesized stingers.
- Optional SFZ parsing and sample playback, with procedural fallback for unmatched notes.
- Deterministic procedural wind, rain, surface-shaped footsteps, weapon and projectile impacts with material and shield profiles, material-specific breakage with debris, non-vocal blood-splatter and bone-break effects, character jump/landing/fall sounds, weapon draw/sheathe/reload handling, cloth/gear/armor movement, and object pickup/place/drop/open/close/toggle sounds, tool chopping/digging/scraping/sawing effects, rendered into caller-owned PCM buffers; vocal pain/exertion and death vocalizations, additional combat contacts, and specialized destruction remain planned.
- One-shot effects, persistent keyed loops, fades, independent music/effects/ambience buses, and voice limits.
- Bounded decoded-PCM streaming for long effects and environmental audio.
- An engine-independent core with an optional Bevy 0.20 integration.

Verdandi provides reusable game-audio concepts and generators without depending on a particular game's entities, event system, world simulation, or asset catalogue. The host loads and decodes its own assets. Verdandi handles composition, procedural sound generation, prepared PCM playback, mixing, routing, stream buffering, and real-time playback.

## Quick start

The default feature enables the Bevy 0.20 plugin:

~~~toml
[dependencies]
verdandi = "0.7"
~~~

~~~rust
use bevy::prelude::*;
use verdandi::prelude::{MusicConfig, MusicPreset, ProceduralMusicPlugin};

fn main() {
    App::new()
        .add_plugins(DefaultPlugins)
        .add_plugins(ProceduralMusicPlugin::new(MusicConfig {
            seed: 1234,
            preset: MusicPreset::MythicAmbient,
            ..default()
        }))
        .run();
}
~~~

For a custom audio host without Bevy:

~~~toml
[dependencies]
verdandi = { version = "0.7", default-features = false }
~~~

Start with the [documentation index](docs/README.md). It contains integration guides, full examples, the public API map, and the real-time/ownership contract. The symbol-level reference is available on [docs.rs](https://docs.rs/verdandi/latest/verdandi/).

## Guides

- [Getting started](docs/getting-started.md)
- [Procedural music](docs/procedural-music.md)
- [Procedural sound generation](docs/procedural-audio.md) · [Sound effects and ambience](docs/sound-effects.md)
- [SFZ instruments](docs/sfz-instruments.md)
- [Bevy integration](docs/bevy-integration.md)
- [Custom audio hosts](docs/custom-audio-hosts.md)
- [Architecture and real-time contract](docs/architecture.md)

## Examples

Render a standalone WAV without Bevy or an audio device:

~~~sh
cargo run --release --no-default-features --example render_wav -- 1234 ethereal 8 ethereal.wav
~~~

Run the interactive Bevy demo:

~~~sh
cargo run --release --features demo --example demo -- 1234 warbound fast
~~~

Audition an external SFZ patch:

~~~sh
cargo run --release --no-default-features --example sfz_audition -- /path/to/instrument.sfz audition.wav
~~~

## Current boundaries

- The composition format is fixed to 4/4, with 16 sixteenth-note steps per bar and four bars per loop.
- Procedural instruments are stylized synthesized voices, not physical models or recordings.
- SFZ support is a documented subset; unsupported opcodes are reported and ignored. The built-in sample decoder supports mono/stereo WAV PCM 8/16/24/32-bit and IEEE float 32/64-bit. FLAC and other compressed formats are not supported.
- Verdandi does not bundle sample libraries or non-musical audio recordings.
- Resident sound clips are supplied as decoded mono/stereo PCM. Streamed sound is also supplied as decoded PCM at the mixer output sample rate; the host owns file access, decoding, and any required stream resampling.
