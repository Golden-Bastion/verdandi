//! Bevy integration (written against Bevy 0.20's `Decodable` API).
//!
//! The music engine runs inside a custom audio source, i.e. on Bevy's audio thread. Game systems
//! steer it through the `MusicController` resource, which only touches atomics.
//!
//! Add `DefaultPlugins` (or at least the asset + audio plugins) *before* `ProceduralMusicPlugin`.

use std::sync::Arc;
use std::time::Duration;

use bevy::audio::{AddAudioSource, ChannelCount, SampleRate, Source};
use bevy::prelude::*;
use bevy::reflect::TypePath;

use crate::composer::Layer;
use crate::engine::{Engine, MusicConfig, MusicShared, MusicStatus, MusicStinger};
use crate::preset::{MusicPace, MusicPreset};
use crate::sound::{
    AudioBus, AudioClip, SoundCategory, SoundCommandError, SoundController, SoundMixer,
    SoundMixerConfig,
};

const CHUNK_FRAMES: usize = 512;

/// The audio asset. Spawn exactly one `AudioPlayer` for it (the plugin does this for you
/// when `autoplay` is true).
#[derive(Asset, TypePath)]
pub struct ProceduralMusic {
    shared: Arc<MusicShared>,
    config: MusicConfig,
    sounds: SoundController,
}

impl ProceduralMusic {
    pub fn new(config: MusicConfig, shared: Arc<MusicShared>) -> Self {
        let sound_config = SoundMixerConfig::new(
            config.sample_rate,
            SoundMixerConfig::default().max_voices,
        );
        let sounds = SoundController::new(sound_config)
            .expect("ProceduralMusic requires a non-zero sample rate");
        Self::with_sound_controller(config, shared, sounds)
    }

    pub fn with_sound_controller(
        config: MusicConfig,
        shared: Arc<MusicShared>,
        sounds: SoundController,
    ) -> Self {
        Self {
            shared,
            config,
            sounds,
        }
    }
}

pub struct MusicDecoder {
    engine: Engine,
    mixer: SoundMixer,
    buf: Vec<f32>,
    pos: usize,
    sample_rate: u32,
}

impl Iterator for MusicDecoder {
    type Item = f32;

    fn next(&mut self) -> Option<f32> {
        if self.pos >= self.buf.len() {
            for frame in self.buf.chunks_exact_mut(2) {
                let music = self.engine.render_frame();
                let sounds = self.mixer.render_frame();
                frame[0] = (music.0 + sounds.0).clamp(-1.0, 1.0);
                frame[1] = (music.1 + sounds.1).clamp(-1.0, 1.0);
            }
            self.pos = 0;
        }
        let sample = self.buf[self.pos];
        self.pos += 1;
        Some(sample) // endless stream
    }
}

impl Source for MusicDecoder {
    fn current_span_len(&self) -> Option<usize> {
        None
    }

    fn channels(&self) -> ChannelCount {
        ChannelCount::new(2).unwrap()
    }

    fn sample_rate(&self) -> SampleRate {
        SampleRate::new(self.sample_rate).unwrap()
    }

    fn total_duration(&self) -> Option<Duration> {
        None
    }
}

impl Decodable for ProceduralMusic {
    type Decoder = MusicDecoder;

    fn decoder(&self) -> Self::Decoder {
        let frames = CHUNK_FRAMES * 2;
        MusicDecoder {
            engine: Engine::new(self.shared.clone(), &self.config),
            mixer: self.sounds.create_mixer(),
            buf: vec![0.0; frames],
            pos: frames, // forces a render on the first sample
            sample_rate: self.config.sample_rate,
        }
    }
}

/// Marker on the entity playing the music. Query its `AudioSink` to pause or change volume.
#[derive(Component)]
pub struct MusicTrack;

/// Game-side handle to the running music.
#[derive(Resource, Clone)]
pub struct AudioController {
    shared: Arc<MusicShared>,
    sounds: SoundController,
}

/// Backwards-compatible name for the unified music and sound controller.
pub type MusicController = AudioController;

impl AudioController {
    pub fn shared(&self) -> &Arc<MusicShared> {
        &self.shared
    }

    /// Pause music transport without advancing musical time.
    pub fn pause(&self) {
        self.shared.pause();
    }

    /// Resume music transport from its paused position.
    pub fn resume(&self) {
        self.shared.resume();
    }

    pub fn is_playing(&self) -> bool {
        self.shared.is_playing()
    }

    /// Set master music gain immediately. Values are clamped to 0.0..=1.0.
    pub fn set_volume(&self, volume: f32) {
        self.shared.set_volume(volume);
    }

    /// The requested master music gain, including an in-progress fade.
    pub fn volume(&self) -> f32 {
        self.shared.volume()
    }

    /// Fade master music gain linearly to the requested value.
    /// A new request replaces any active fade from its current gain.
    pub fn fade_to(&self, volume: f32, duration: Duration) {
        self.shared.fade_to(volume, duration);
    }

    /// Queue a new seed; it takes over at the next loop boundary.
    pub fn set_seed(&self, seed: u64) {
        self.shared.request_seed(seed);
    }

    /// The seed playing now, or the queued one.
    pub fn seed(&self) -> u64 {
        self.shared.target_seed()
    }

    pub fn set_layer_enabled(&self, layer: Layer, on: bool) {
        self.shared.set_layer_enabled(layer, on);
    }

    pub fn layer_enabled(&self, layer: Layer) -> bool {
        self.shared.layer_enabled(layer)
    }

    pub fn toggle_layer(&self, layer: Layer) {
        self.shared
            .set_layer_enabled(layer, !self.shared.layer_enabled(layer));
    }

    /// Manual intensity 0..=1 (only used while auto-intensity is off).
    pub fn set_intensity(&self, value: f32) {
        self.shared.set_intensity(value);
    }

    pub fn intensity(&self) -> f32 {
        self.shared.intensity()
    }

    pub fn set_auto_intensity(&self, on: bool) {
        self.shared.set_auto_intensity(on);
    }

    pub fn auto_intensity(&self) -> bool {
        self.shared.auto_intensity()
    }

    /// Queue a preset/pace change for the next bar, then crossfade over two seconds.
    pub fn set_preset(&self, preset: MusicPreset, pace: MusicPace) {
        self.shared.request_preset(preset, pace);
    }

    /// Start the two-second preset crossfade as soon as the audio callback processes it.
    pub fn set_preset_immediate(&self, preset: MusicPreset, pace: MusicPace) {
        self.shared.request_preset_immediate(preset, pace);
    }

    /// Play a short synthesized event cue over the continuing background music.
    pub fn play_stinger(&self, stinger: MusicStinger) {
        self.shared.request_stinger(stinger);
    }

    pub fn status(&self) -> MusicStatus {
        self.shared.status()
    }

    /// Access the engine-independent sound command handle.
    pub fn sounds(&self) -> &SoundController {
        &self.sounds
    }

    /// Queue a one-shot sound effect or environmental cue.
    pub fn play_one_shot(
        &self,
        clip: Arc<AudioClip>,
        category: SoundCategory,
        gain: f32,
    ) -> Result<(), SoundCommandError> {
        self.sounds.play_one_shot(clip, category, gain)
    }

    /// Start or replace a keyed environmental loop.
    pub fn play_loop(
        &self,
        key: crate::sound::LoopKey,
        clip: Arc<AudioClip>,
        category: SoundCategory,
        gain: f32,
        fade_in: Duration,
    ) -> Result<(), SoundCommandError> {
        self.sounds.play_loop(key, clip, category, gain, fade_in)
    }

    pub fn fade_loop(
        &self,
        key: crate::sound::LoopKey,
        gain: f32,
        duration: Duration,
    ) -> Result<(), SoundCommandError> {
        self.sounds.fade_loop(key, gain, duration)
    }

    pub fn stop_loop(
        &self,
        key: crate::sound::LoopKey,
        fade_out: Duration,
    ) -> Result<(), SoundCommandError> {
        self.sounds.stop_loop(key, fade_out)
    }

    /// Set an independent music, effects, or ambience bus gain immediately.
    pub fn set_bus_volume(&self, bus: AudioBus, gain: f32) {
        match bus {
            AudioBus::Music => self.shared.set_volume(gain),
            AudioBus::Effects => self.sounds.set_category_volume(SoundCategory::Effects, gain),
            AudioBus::Ambience => self.sounds.set_category_volume(SoundCategory::Ambience, gain),
        }
    }

    pub fn bus_volume(&self, bus: AudioBus) -> f32 {
        match bus {
            AudioBus::Music => self.shared.volume(),
            AudioBus::Effects => self.sounds.category_volume(SoundCategory::Effects),
            AudioBus::Ambience => self.sounds.category_volume(SoundCategory::Ambience),
        }
    }

    pub fn set_bus_muted(&self, bus: AudioBus, muted: bool) {
        match bus {
            AudioBus::Music => self.shared.set_muted(muted),
            AudioBus::Effects => self.sounds.set_category_muted(SoundCategory::Effects, muted),
            AudioBus::Ambience => self.sounds.set_category_muted(SoundCategory::Ambience, muted),
        }
    }

    pub fn bus_muted(&self, bus: AudioBus) -> bool {
        match bus {
            AudioBus::Music => self.shared.is_muted(),
            AudioBus::Effects => self.sounds.category_muted(SoundCategory::Effects),
            AudioBus::Ambience => self.sounds.category_muted(SoundCategory::Ambience),
        }
    }
}

#[derive(Resource)]
struct MusicSetup {
    shared: Arc<MusicShared>,
    sounds: SoundController,
    config: MusicConfig,
    autoplay: bool,
}

pub struct ProceduralMusicPlugin {
    pub config: MusicConfig,
    /// Spawn the `AudioPlayer` at startup. Set false to spawn `ProceduralMusic` yourself.
    pub autoplay: bool,
}

impl ProceduralMusicPlugin {
    pub fn new(config: MusicConfig) -> Self {
        Self {
            config,
            autoplay: true,
        }
    }
}

impl Default for ProceduralMusicPlugin {
    fn default() -> Self {
        Self::new(MusicConfig::default())
    }
}

impl Plugin for ProceduralMusicPlugin {
    fn build(&self, app: &mut App) {
        let shared = Arc::new(MusicShared::new(&self.config));
        let sound_config = SoundMixerConfig::new(
            self.config.sample_rate,
            SoundMixerConfig::default().max_voices,
        );
        let sounds = SoundController::new(sound_config)
            .expect("ProceduralMusicPlugin requires a non-zero sample rate");
        app.add_audio_source::<ProceduralMusic>()
            .insert_resource(AudioController {
                shared: shared.clone(),
                sounds: sounds.clone(),
            })
            .insert_resource(MusicSetup {
                shared,
                sounds,
                config: self.config.clone(),
                autoplay: self.autoplay,
            })
            .add_systems(Startup, start_music);
    }
}

fn start_music(
    mut commands: Commands,
    mut assets: ResMut<Assets<ProceduralMusic>>,
    setup: Res<MusicSetup>,
) {
    if !setup.autoplay {
        return;
    }
    let handle = assets.add(ProceduralMusic::with_sound_controller(
        setup.config.clone(),
        setup.shared.clone(),
        setup.sounds.clone(),
    ));
    commands.spawn((AudioPlayer(handle), MusicTrack));
}
