# Bevy integration

Verdandi's optional plugin targets **Bevy 0.20**. It wraps the engine-independent renderer in a Bevy audio source and exposes one resource for controlling music, effects, and ambience.

## Add the plugin

The default Verdandi feature set includes **bevy-plugin**. Bevy's asset and audio plugins must be registered first; **DefaultPlugins** satisfies this requirement.

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

The plugin automatically registers the audio source, inserts the **AudioController** resource, and starts one player with a **MusicTrack** marker. Do not add another player for the same **ProceduralMusic** source. The controller is also available under the backwards-compatible type alias **MusicController**.

## Control music from systems

Use **Res<AudioController>** (or **Res<MusicController>**) in your systems:

~~~rust
use bevy::prelude::Res;
use verdandi::prelude::{
    AudioController, Layer, MusicPace, MusicPreset, MusicStinger,
};

fn player_entered_combat(audio: Res<AudioController>) {
    audio.set_auto_intensity(false);
    audio.set_intensity(1.0);
    audio.set_preset(MusicPreset::HeavyMetal, MusicPace::Fast);
    audio.play_stinger(MusicStinger::CombatStart);
    audio.set_layer_enabled(Layer::RhythmGuitar, false);
}

fn player_entered_safe_area(audio: Res<AudioController>) {
    audio.set_preset(MusicPreset::MythicAmbient, MusicPace::Standard);
    audio.set_auto_intensity(true);
}
~~~

Call these functions from event-driven systems or guarded state transitions, not unconditionally every frame. Runtime preset changes crossfade over two seconds. The normal variant waits for the next bar; the immediate variant begins when the audio callback processes the request.

## Music controls

The controller provides:

- Seed selection with **set_seed**; the new seed takes effect at the next four-bar loop boundary.
- Layer enable/disable/toggle controls and layer state queries.
- Manual or automatic intensity controls.
- Preset and pace changes, including immediate transition requests.
- Synthesized stingers: CombatStart, Victory, BossReveal, and Transition.
- Pause/resume of musical transport without losing the paused position.
- Immediate master gain, mute, and linear master fades.
- **status()** for the active seed, arrangement, musical position, theme, intensity, and layer states.

See [procedural music](procedural-music.md) for the full behaviour of each control and the preset/layer matrix.

## Effects and ambience

The same resource also exposes the sound mixer:

~~~rust
use std::sync::Arc;
use std::time::Duration;
use verdandi::prelude::{AudioBus, AudioClip, AudioController, LoopKey, SoundCategory};

fn play_world_audio(
    audio: &AudioController,
    rain: Arc<AudioClip>,
    footstep: Arc<AudioClip>,
) -> Result<(), verdandi::sound::SoundCommandError> {
    audio.play_loop(
        LoopKey(100),
        rain,
        SoundCategory::Ambience,
        0.55,
        Duration::from_secs(2),
    )?;
    audio.play_one_shot(footstep, SoundCategory::Effects, 0.9)?;
    audio.set_bus_volume(AudioBus::Ambience, 0.7);
    audio.set_bus_muted(AudioBus::Effects, false);
    Ok(())
}
~~~

**AudioClip** is not a Bevy **Asset**. Your application loads and decodes files using its own asset pipeline, then creates and retains **Arc<AudioClip>** values in its own resource or asset catalogue. The example receives those prepared clips as arguments to make that ownership explicit.

For bus-level controls use **set_bus_volume**, **bus_volume**, **set_bus_muted**, and **bus_muted**. Music, effects, and ambience have independent gain and mute state. Playback and streaming details are in [sound effects and ambience](sound-effects.md).

## AudioSink and MusicTrack

The plugin marks its automatically spawned player with **MusicTrack**. Query that entity's Bevy **AudioSink** only when you need Bevy-specific sink controls, such as output-device playback controls. Prefer **AudioController::pause** and **resume** when the desired behaviour is to freeze Verdandi's musical transport: pausing the transport preserves the sequencer position and synthesis state.

~~~rust
use bevy::audio::AudioSink;
use bevy::prelude::*;
use verdandi::prelude::MusicTrack;

fn toggle_host_sink(
    keys: Res<ButtonInput<KeyCode>>,
    sinks: Query<&AudioSink, With<MusicTrack>>,
) {
    if keys.just_pressed(KeyCode::Space) {
        if let Ok(sink) = sinks.single() {
            sink.toggle_playback();
        }
    }
}
~~~

## Advanced plugin setup

**ProceduralMusicPlugin** has a public **autoplay** field. Set it to **false** when the plugin should register its resource and audio source without automatically spawning the player. In that mode, the application is responsible for creating and spawning the **ProceduralMusic** audio asset/player. The application must retain the same **MusicConfig** it passed to the plugin:

~~~rust
use bevy::prelude::*;
use verdandi::prelude::{
    AudioController, MusicConfig, MusicTrack, ProceduralMusic,
    ProceduralMusicPlugin,
};

#[derive(Resource, Clone)]
struct AppMusicConfig(MusicConfig);

fn spawn_custom_music_player(
    mut commands: Commands,
    mut assets: ResMut<Assets<ProceduralMusic>>,
    audio: Res<AudioController>,
    config: Res<AppMusicConfig>,
) {
    let source = assets.add(ProceduralMusic::with_sound_controller(
        config.0.clone(),
        audio.shared().clone(),
        audio.sounds().clone(),
    ));
    commands.spawn((AudioPlayer(source), MusicTrack));
}

fn main() {
    let config = MusicConfig::default();

    App::new()
        .add_plugins(DefaultPlugins)
        .insert_resource(AppMusicConfig(config.clone()))
        .add_plugins(ProceduralMusicPlugin {
            config,
            autoplay: false,
        })
        .add_systems(Startup, spawn_custom_music_player)
        .run();
}
~~~

Most applications should leave autoplay enabled.

The Bevy plugin uses the default sound voice limit of 32. Applications needing a different limit or a non-Bevy device/backend should use the engine-independent **SoundController** and **SoundMixer** APIs described in [custom audio hosts](custom-audio-hosts.md).
