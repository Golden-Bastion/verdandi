# Getting started

This guide takes you from adding the crate to rendering audio. Choose the integration that matches your application: Bevy or a custom audio host. Both use the same engine-independent music and sound systems.

## Requirements and features

Verdandi targets **Bevy 0.20** for its optional plugin. The crate itself is engine-independent.

| Feature | Default | Purpose |
|---|---:|---|
| **bevy-plugin** | Yes | Adds the Bevy 0.20 audio source, plugin, and unified controller |
| **demo** | No | Adds the Bevy UI/window/input features needed by the interactive example |

For a Bevy application, the default dependency enables the plugin:

~~~toml
[dependencies]
verdandi = "0.9"
~~~

For a custom host that does not use Bevy, disable default features:

~~~toml
[dependencies]
verdandi = { version = "0.9", default-features = false }
~~~

If your application already depends on Bevy, use the Bevy integration guide and keep the Bevy version aligned with Verdandi's Bevy 0.20 target.

## First render without Bevy

The engine renders interleaved stereo f32 samples. Create the shared control state and engine before starting the audio callback. The callback owns the mutable Engine and asks it to fill an even-length output slice.

~~~rust
use std::sync::Arc;
use verdandi::prelude::{Engine, MusicConfig, MusicShared};

fn main() {
    let config = MusicConfig::default();
    let shared = Arc::new(MusicShared::new(&config));
    let mut engine = Engine::new(shared, &config);

    // One hundred and twenty-eight stereo frames (left, right, left, right, ...).
    let mut output = [0.0_f32; 128 * 2];
    engine.render_interleaved(&mut output);

    // Pass output to your host's audio output layer.
}
~~~

This example renders one block only. A real application repeatedly fills the output buffer from its audio callback. The [custom audio hosts guide](custom-audio-hosts.md) shows how to combine music with the sound-effects mixer.

## First render with Bevy

The plugin registers Verdandi's audio source, inserts the controller resource, and starts one music player automatically. Add Bevy's asset/audio plugins first; DefaultPlugins already includes them.

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

Read [Bevy integration](bevy-integration.md) for gameplay controls, stingers, sound effects, bus management, and the MusicTrack marker.

## Run the repository examples

Render a standalone WAV without a Bevy window or audio device:

~~~sh
cargo run --release --no-default-features --example render_wav -- 1234 ethereal 8 ethereal.wav
~~~

The argument order is:

~~~text
render_wav <seed> <preset> <loops> <output.wav> [standard|fast] [layer]
~~~

Presets accept the public names ethereal, warbound, orchestral, and contemplative; older aliases are retained. To render only one supported layer, supply the pace argument first and then the layer name.

Run the interactive Bevy demo with the optional demo feature:

~~~sh
cargo run --release --features demo --example demo -- 1234 ethereal
~~~

The demo supports seed changes, layer toggles, manual/automatic intensity, and pause. On Linux, Bevy's native audio/window dependencies may need to be installed according to the Bevy platform setup instructions.

Audition an external SFZ patch to a WAV file:

~~~sh
cargo run --release --no-default-features --example sfz_audition -- /path/to/instrument.sfz audition.wav
~~~

The [SFZ guide](sfz-instruments.md) explains sample-root handling and the supported subset.

## What to read next

- Use [procedural music](procedural-music.md) to tune a track rather than accepting the defaults.
- Use [sound effects and ambience](sound-effects.md) to add host-provided effects, persistent loops, or streamed environmental audio.
- Use [architecture and real-time contract](architecture.md) before wiring Verdandi into a custom callback or moving audio work across threads.
