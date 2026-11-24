//! Engine-independent playback for host-provided, pre-decoded sound assets.
//!
//! The host owns loading, decoding, and the lifetime of each clip. The mixer only reads immutable
//! PCM data and performs bounded mixing/resampling; it never opens files or decodes audio.

use std::cell::UnsafeCell;
use std::error::Error;
use std::fmt;
use std::mem::MaybeUninit;
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::Duration;

const COMMAND_QUEUE_CAPACITY: usize = 64;
const MAX_COMMANDS_PER_FRAME: usize = COMMAND_QUEUE_CAPACITY;
pub const MAX_SOUND_VOICES: usize = 128;

/// The independent gain/mute buses exposed by the unified audio controller.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AudioBus {
    Music,
    Effects,
    Ambience,
}

/// A non-musical routing category handled by SoundMixer.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum SoundCategory {
    #[default]
    Effects,
    Ambience,
}

impl SoundCategory {
    fn index(self) -> usize {
        match self {
            Self::Effects => 0,
            Self::Ambience => 1,
        }
    }
}

/// Stable application-defined identity for a persistent looping sound.
///
/// Starting another loop with the same key replaces the existing loop at the next render frame.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct LoopKey(pub u64);

/// Immutable interleaved mono or stereo PCM prepared by the host.
///
/// Keep an owning `Arc<AudioClip>` in the host's asset catalogue while a clip may be queued or
/// playing. This ensures playback completion or replacement on the render thread cannot release
/// the final sample-buffer allocation there.
#[derive(Clone, Debug, PartialEq)]
pub struct AudioClip {
    sample_rate: u32,
    channels: u16,
    samples: Arc<[f32]>,
}

impl AudioClip {
    /// Construct a clip from host-decoded PCM samples.
    ///
    /// Samples are interleaved when channels is 2. Only mono and stereo are supported.
    pub fn from_samples(
        sample_rate: u32,
        channels: u16,
        samples: Vec<f32>,
    ) -> Result<Self, AudioClipError> {
        Self::from_shared_samples(sample_rate, channels, Arc::from(samples))
    }

    /// Construct a clip from shared host-owned PCM without copying the sample buffer.
    pub fn from_shared_samples(
        sample_rate: u32,
        channels: u16,
        samples: Arc<[f32]>,
    ) -> Result<Self, AudioClipError> {
        if sample_rate == 0 {
            return Err(AudioClipError::InvalidSampleRate);
        }
        if !matches!(channels, 1 | 2) {
            return Err(AudioClipError::UnsupportedChannelCount(channels));
        }
        if samples.is_empty() {
            return Err(AudioClipError::Empty);
        }
        if samples.len() % usize::from(channels) != 0 {
            return Err(AudioClipError::IncompleteFrame {
                sample_count: samples.len(),
                channels,
            });
        }
        if let Some(index) = samples.iter().position(|sample| !sample.is_finite()) {
            return Err(AudioClipError::NonFiniteSample { index });
        }

        Ok(Self {
            sample_rate,
            channels,
            samples,
        })
    }

    pub fn sample_rate(&self) -> u32 {
        self.sample_rate
    }

    pub fn channels(&self) -> u16 {
        self.channels
    }

    pub fn frame_count(&self) -> usize {
        self.samples.len() / usize::from(self.channels)
    }

    pub fn duration(&self) -> Duration {
        Duration::from_secs_f64(self.frame_count() as f64 / f64::from(self.sample_rate))
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum AudioClipError {
    InvalidSampleRate,
    UnsupportedChannelCount(u16),
    Empty,
    IncompleteFrame { sample_count: usize, channels: u16 },
    NonFiniteSample { index: usize },
}

impl fmt::Display for AudioClipError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidSampleRate => write!(f, "sample rate must be greater than zero"),
            Self::UnsupportedChannelCount(channels) => {
                write!(f, "only mono and stereo clips are supported, got {channels} channels")
            }
            Self::Empty => write!(f, "audio clip must contain at least one frame"),
            Self::IncompleteFrame {
                sample_count,
                channels,
            } => write!(
                f,
                "{sample_count} samples do not form complete {channels}-channel frames"
            ),
            Self::NonFiniteSample { index } => {
                write!(f, "sample {index} is not finite")
            }
        }
    }
}

impl Error for AudioClipError {}

/// Configuration for a bounded, engine-independent sound mixer.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SoundMixerConfig {
    pub sample_rate: u32,
    pub max_voices: usize,
}

impl SoundMixerConfig {
    pub const fn new(sample_rate: u32, max_voices: usize) -> Self {
        Self {
            sample_rate,
            max_voices,
        }
    }
}

impl Default for SoundMixerConfig {
    fn default() -> Self {
        Self {
            sample_rate: 44_100,
            max_voices: 32,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SoundMixerConfigError {
    InvalidSampleRate,
    InvalidVoiceLimit { requested: usize, maximum: usize },
}

impl fmt::Display for SoundMixerConfigError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidSampleRate => write!(f, "mixer sample rate must be greater than zero"),
            Self::InvalidVoiceLimit { requested, maximum } => write!(
                f,
                "voice limit must be in 1..={maximum}, got {requested}"
            ),
        }
    }
}

impl Error for SoundMixerConfigError {}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SoundMixerCreateError {
    AlreadyCreated,
}

impl fmt::Display for SoundMixerCreateError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::AlreadyCreated => write!(f, "a sound controller already has an active mixer"),
        }
    }
}

impl Error for SoundMixerCreateError {}

/// A non-blocking failure to submit a playback command.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SoundCommandError {
    /// The bounded command queue is full. The caller may retry later.
    QueueFull,
    /// Another producer is submitting a command. No thread waits; the caller may retry later.
    ProducerBusy,
}

impl fmt::Display for SoundCommandError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::QueueFull => write!(f, "sound command queue is full"),
            Self::ProducerBusy => write!(f, "another thread is submitting a sound command"),
        }
    }
}

impl Error for SoundCommandError {}

/// Snapshot of the mixer's bounded voice usage and rejected-play count.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SoundMixerStatus {
    pub active_voices: usize,
    pub max_voices: usize,
    pub rejected_voices: u64,
}

/// Control-side handle for submitting sound playback commands.
///
/// Create exactly one active SoundMixer for each controller.
///
/// Commands are placed in a bounded queue. Concurrent producers are serialized with a
/// non-blocking atomic gate: a contending caller receives ProducerBusy instead of making the
/// audio thread wait or spin.
#[derive(Clone)]
pub struct SoundController {
    shared: Arc<SoundShared>,
}

impl SoundController {
    pub fn new(config: SoundMixerConfig) -> Result<Self, SoundMixerConfigError> {
        if config.sample_rate == 0 {
            return Err(SoundMixerConfigError::InvalidSampleRate);
        }
        if config.max_voices == 0 || config.max_voices > MAX_SOUND_VOICES {
            return Err(SoundMixerConfigError::InvalidVoiceLimit {
                requested: config.max_voices,
                maximum: MAX_SOUND_VOICES,
            });
        }

        Ok(Self {
            shared: Arc::new(SoundShared::new(config)),
        })
    }

    /// Create the single audio-thread-owned mixer that consumes this controller's commands.
    ///
    /// Construct the mixer before starting the host audio callback so its voice storage is
    /// allocated ahead of rendering. A second active mixer would violate the queue's
    /// single-consumer contract and is rejected by try_create_mixer.
    pub fn create_mixer(&self) -> SoundMixer {
        self.try_create_mixer()
            .expect("a SoundController may only have one active SoundMixer")
    }

    pub fn try_create_mixer(&self) -> Result<SoundMixer, SoundMixerCreateError> {
        self.shared
            .consumer_active
            .compare_exchange(false, true, Ordering::Acquire, Ordering::Relaxed)
            .map_err(|_| SoundMixerCreateError::AlreadyCreated)?;
        Ok(SoundMixer::new(self.shared.clone()))
    }

    pub fn sample_rate(&self) -> u32 {
        self.shared.config.sample_rate
    }

    pub fn max_voices(&self) -> usize {
        self.shared.config.max_voices
    }

    /// Queue a one-shot clip. If every voice is occupied when the command is consumed, the
    /// newest one-shot is rejected and existing voices continue unchanged.
    pub fn play_one_shot(
        &self,
        clip: Arc<AudioClip>,
        category: SoundCategory,
        gain: f32,
    ) -> Result<(), SoundCommandError> {
        self.submit(AudioCommand::PlayOneShot {
            clip,
            category,
            gain: sanitize_gain(gain),
        })
    }

    /// Start a persistent loop, replacing any active loop with the same key.
    ///
    /// The replacement begins at the next render frame. A non-zero fade_in starts the new
    /// loop at silence and linearly reaches gain over the requested duration.
    pub fn play_loop(
        &self,
        key: LoopKey,
        clip: Arc<AudioClip>,
        category: SoundCategory,
        gain: f32,
        fade_in: Duration,
    ) -> Result<(), SoundCommandError> {
        self.submit(AudioCommand::PlayLoop {
            key,
            clip,
            category,
            gain: sanitize_gain(gain),
            fade_in_frames: duration_frames(fade_in, self.sample_rate()),
        })
    }

    /// Fade a keyed loop to a new per-voice gain. A later fade replaces the current fade from
    /// the gain reached at the time the new command is consumed.
    pub fn fade_loop(
        &self,
        key: LoopKey,
        gain: f32,
        duration: Duration,
    ) -> Result<(), SoundCommandError> {
        self.submit(AudioCommand::FadeLoop {
            key,
            gain: sanitize_gain(gain),
            duration_frames: duration_frames(duration, self.sample_rate()),
        })
    }

    /// Stop a keyed loop immediately or after a linear fade to silence.
    pub fn stop_loop(
        &self,
        key: LoopKey,
        fade_out: Duration,
    ) -> Result<(), SoundCommandError> {
        self.submit(AudioCommand::StopLoop {
            key,
            duration_frames: duration_frames(fade_out, self.sample_rate()),
        })
    }

    pub fn set_category_volume(&self, category: SoundCategory, gain: f32) {
        self.shared.category_gain[category.index()]
            .store(sanitize_gain(gain).to_bits(), Ordering::Release);
    }

    pub fn category_volume(&self, category: SoundCategory) -> f32 {
        f32::from_bits(self.shared.category_gain[category.index()].load(Ordering::Acquire))
    }

    pub fn set_category_muted(&self, category: SoundCategory, muted: bool) {
        self.shared.category_muted[category.index()].store(muted, Ordering::Release);
    }

    pub fn category_muted(&self, category: SoundCategory) -> bool {
        self.shared.category_muted[category.index()].load(Ordering::Acquire)
    }

    pub fn status(&self) -> SoundMixerStatus {
        SoundMixerStatus {
            active_voices: self.shared.active_voices.load(Ordering::Acquire),
            max_voices: self.shared.config.max_voices,
            rejected_voices: self.shared.rejected_voices.load(Ordering::Relaxed),
        }
    }

    fn submit(&self, command: AudioCommand) -> Result<(), SoundCommandError> {
        if self
            .shared
            .producer_active
            .compare_exchange(false, true, Ordering::Acquire, Ordering::Relaxed)
            .is_err()
        {
            return Err(SoundCommandError::ProducerBusy);
        }
        let _guard = ProducerGuard(&self.shared.producer_active);
        self.shared
            .commands
            .push(command)
            .map_err(|_| SoundCommandError::QueueFull)
    }
}

struct ProducerGuard<'a>(&'a AtomicBool);

impl Drop for ProducerGuard<'_> {
    fn drop(&mut self) {
        self.0.store(false, Ordering::Release);
    }
}

struct SoundShared {
    config: SoundMixerConfig,
    commands: SpscQueue<AudioCommand, COMMAND_QUEUE_CAPACITY>,
    producer_active: AtomicBool,
    consumer_active: AtomicBool,
    category_gain: [AtomicU32; 2],
    category_muted: [AtomicBool; 2],
    active_voices: AtomicUsize,
    rejected_voices: AtomicU64,
}

impl SoundShared {
    fn new(config: SoundMixerConfig) -> Self {
        Self {
            config,
            commands: SpscQueue::new(),
            producer_active: AtomicBool::new(false),
            consumer_active: AtomicBool::new(false),
            category_gain: std::array::from_fn(|_| AtomicU32::new(1.0f32.to_bits())),
            category_muted: std::array::from_fn(|_| AtomicBool::new(false)),
            active_voices: AtomicUsize::new(0),
            rejected_voices: AtomicU64::new(0),
        }
    }
}

enum AudioCommand {
    PlayOneShot {
        clip: Arc<AudioClip>,
        category: SoundCategory,
        gain: f32,
    },
    PlayLoop {
        key: LoopKey,
        clip: Arc<AudioClip>,
        category: SoundCategory,
        gain: f32,
        fade_in_frames: u64,
    },
    FadeLoop {
        key: LoopKey,
        gain: f32,
        duration_frames: u64,
    },
    StopLoop {
        key: LoopKey,
        duration_frames: u64,
    },
}

/// A fixed-capacity single-producer/single-consumer ring. The producer gate above ensures that
/// multiple control threads never access the producer side simultaneously.
struct SpscQueue<T, const N: usize> {
    slots: [UnsafeCell<MaybeUninit<T>>; N],
    write: AtomicUsize,
    read: AtomicUsize,
}

// SAFETY: each slot is written by the single producer and read by the single consumer, with
// ownership transferred through release/acquire publication of the monotonically increasing
// indices. T is never accessed concurrently.
unsafe impl<T: Send, const N: usize> Send for SpscQueue<T, N> {}
// SAFETY: the producer gate enforces one writer; the queue indices synchronize writer and reader.
unsafe impl<T: Send, const N: usize> Sync for SpscQueue<T, N> {}

impl<T, const N: usize> SpscQueue<T, N> {
    fn new() -> Self {
        assert!(N > 0);
        Self {
            slots: std::array::from_fn(|_| UnsafeCell::new(MaybeUninit::uninit())),
            write: AtomicUsize::new(0),
            read: AtomicUsize::new(0),
        }
    }

    fn push(&self, value: T) -> Result<(), T> {
        let write = self.write.load(Ordering::Relaxed);
        let read = self.read.load(Ordering::Acquire);
        if write.wrapping_sub(read) >= N {
            return Err(value);
        }

        // SAFETY: the acquire read above confirms this slot has been consumed. The producer gate
        // ensures no second producer can write this slot concurrently.
        unsafe {
            (*self.slots[write % N].get()).write(value);
        }
        self.write.store(write.wrapping_add(1), Ordering::Release);
        Ok(())
    }

    fn pop(&self) -> Option<T> {
        let read = self.read.load(Ordering::Relaxed);
        let write = self.write.load(Ordering::Acquire);
        if read == write {
            return None;
        }

        // SAFETY: the acquire read above observes a fully initialized slot. The consumer is the
        // only reader and advances read only after taking ownership of its value.
        let value = unsafe { (*self.slots[read % N].get()).assume_init_read() };
        self.read.store(read.wrapping_add(1), Ordering::Release);
        Some(value)
    }
}

impl<T, const N: usize> Drop for SpscQueue<T, N> {
    fn drop(&mut self) {
        while self.pop().is_some() {}
    }
}

/// Audio-thread-owned, fixed-voice mixer.
///
/// Construct this before entering the host's render callback. The render path processes at most
/// 64 queued commands per frame, reads immutable PCM, resamples linearly, mixes active voices,
/// and performs no file access or decoding.
pub struct SoundMixer {
    shared: Arc<SoundShared>,
    voices: Vec<Voice>,
    sample_rate: u32,
    _consumer_guard: ConsumerGuard,
}

struct ConsumerGuard(Arc<SoundShared>);

impl Drop for ConsumerGuard {
    fn drop(&mut self) {
        self.0.active_voices.store(0, Ordering::Release);
        self.0.consumer_active.store(false, Ordering::Release);
    }
}

impl SoundMixer {
    fn new(shared: Arc<SoundShared>) -> Self {
        let voices = (0..shared.config.max_voices)
            .map(|_| Voice::default())
            .collect();
        Self {
            sample_rate: shared.config.sample_rate,
            _consumer_guard: ConsumerGuard(shared.clone()),
            shared,
            voices,
        }
    }

    /// Render one stereo frame. The host is responsible for mixing this with music and applying
    /// any final master limiter/clipping policy.
    pub fn render_frame(&mut self) -> (f32, f32) {
        self.process_commands();

        let gains = [
            f32::from_bits(self.shared.category_gain[0].load(Ordering::Acquire)),
            f32::from_bits(self.shared.category_gain[1].load(Ordering::Acquire)),
        ];
        let muted = [
            self.shared.category_muted[0].load(Ordering::Acquire),
            self.shared.category_muted[1].load(Ordering::Acquire),
        ];

        let mut left = 0.0f64;
        let mut right = 0.0f64;
        let mut active = 0;
        for voice in &mut self.voices {
            if voice.clip.is_none() {
                continue;
            }
            let category_index = voice.category.index();
            let bus_gain = if muted[category_index] {
                0.0
            } else {
                gains[category_index]
            };
            let (voice_left, voice_right) = voice.render(self.sample_rate, bus_gain);
            left += f64::from(voice_left);
            right += f64::from(voice_right);
            if voice.clip.is_some() {
                active += 1;
            }
        }

        self.shared.active_voices.store(active, Ordering::Release);
        let maximum = f64::from(f32::MAX);
        (
            left.clamp(-maximum, maximum) as f32,
            right.clamp(-maximum, maximum) as f32,
        )
    }

    /// Fill an interleaved stereo output slice.
    pub fn render_interleaved(&mut self, out: &mut [f32]) {
        for frame in out.chunks_exact_mut(2) {
            let (left, right) = self.render_frame();
            frame[0] = left;
            frame[1] = right;
        }
    }

    fn process_commands(&mut self) {
        for _ in 0..MAX_COMMANDS_PER_FRAME {
            let Some(command) = self.shared.commands.pop() else {
                break;
            };
            self.apply_command(command);
        }
    }

    fn apply_command(&mut self, command: AudioCommand) {
        match command {
            AudioCommand::PlayOneShot {
                clip,
                category,
                gain,
            } => {
                if let Some(voice) = self.voices.iter_mut().find(|voice| voice.clip.is_none()) {
                    voice.start(clip, None, false, category, gain, 0);
                } else {
                    self.shared.rejected_voices.fetch_add(1, Ordering::Relaxed);
                }
            }
            AudioCommand::PlayLoop {
                key,
                clip,
                category,
                gain,
                fade_in_frames,
            } => {
                let existing = self.voices.iter_mut().find(|voice| {
                    voice.clip.is_some() && voice.looping && voice.loop_key == Some(key)
                });
                if let Some(voice) = existing {
                    voice.start(clip, Some(key), true, category, gain, fade_in_frames);
                } else if let Some(voice) =
                    self.voices.iter_mut().find(|voice| voice.clip.is_none())
                {
                    voice.start(clip, Some(key), true, category, gain, fade_in_frames);
                } else {
                    self.shared.rejected_voices.fetch_add(1, Ordering::Relaxed);
                }
            }
            AudioCommand::FadeLoop {
                key,
                gain,
                duration_frames,
            } => {
                if let Some(voice) = self.voices.iter_mut().find(|voice| {
                    voice.clip.is_some() && voice.looping && voice.loop_key == Some(key)
                }) {
                    voice.fade_to(gain, duration_frames, false);
                }
            }
            AudioCommand::StopLoop {
                key,
                duration_frames,
            } => {
                if let Some(voice) = self.voices.iter_mut().find(|voice| {
                    voice.clip.is_some() && voice.looping && voice.loop_key == Some(key)
                }) {
                    if duration_frames == 0 {
                        voice.clear();
                    } else {
                        voice.fade_to(0.0, duration_frames, true);
                    }
                }
            }
        }
    }
}

#[derive(Default)]
struct Voice {
    clip: Option<Arc<AudioClip>>,
    category: SoundCategory,
    loop_key: Option<LoopKey>,
    looping: bool,
    position: f64,
    gain: f32,
    fade_start_gain: f32,
    fade_target_gain: f32,
    fade_elapsed_frames: u64,
    fade_total_frames: u64,
    stop_after_fade: bool,
}

impl Voice {
    fn start(
        &mut self,
        clip: Arc<AudioClip>,
        loop_key: Option<LoopKey>,
        looping: bool,
        category: SoundCategory,
        gain: f32,
        fade_in_frames: u64,
    ) {
        self.clip = Some(clip);
        self.category = category;
        self.loop_key = loop_key;
        self.looping = looping;
        self.position = 0.0;
        self.gain = if fade_in_frames == 0 { gain } else { 0.0 };
        self.fade_start_gain = self.gain;
        self.fade_target_gain = self.gain;
        self.fade_elapsed_frames = 0;
        self.fade_total_frames = 0;
        self.stop_after_fade = false;
        if fade_in_frames > 0 {
            self.fade_to(gain, fade_in_frames, false);
        }
    }

    fn fade_to(&mut self, target: f32, duration_frames: u64, stop_after: bool) {
        self.fade_start_gain = self.gain;
        self.fade_target_gain = target;
        self.fade_elapsed_frames = 0;
        self.fade_total_frames = duration_frames;
        self.stop_after_fade = stop_after;
        if duration_frames == 0 {
            self.gain = target;
            if stop_after {
                self.clear();
            }
        }
    }

    fn render(&mut self, output_rate: u32, bus_gain: f32) -> (f32, f32) {
        let Some(clip) = self.clip.as_ref() else {
            return (0.0, 0.0);
        };
        let frame_count = clip.frame_count();
        if self.position >= frame_count as f64 {
            if self.looping {
                self.position %= frame_count as f64;
            } else {
                self.clear();
                return (0.0, 0.0);
            }
        }

        let current = self.position.floor() as usize;
        let next = if current + 1 < frame_count {
            current + 1
        } else if self.looping {
            0
        } else {
            current
        };
        let fraction = (self.position - current as f64) as f32;
        let channels = usize::from(clip.channels);
        let samples = &clip.samples;

        let current_left = samples[current * channels];
        let next_left = samples[next * channels];
        let left = current_left + (next_left - current_left) * fraction;

        let right = if channels == 1 {
            left
        } else {
            let current_right = samples[current * channels + 1];
            let next_right = samples[next * channels + 1];
            current_right + (next_right - current_right) * fraction
        };

        let output = (left * self.gain * bus_gain, right * self.gain * bus_gain);
        self.position += f64::from(clip.sample_rate) / f64::from(output_rate);
        self.advance_fade();
        if !self.looping && self.position >= frame_count as f64 {
            self.clear();
        }
        output
    }

    fn advance_fade(&mut self) {
        if self.fade_total_frames == 0 {
            return;
        }
        self.fade_elapsed_frames = self.fade_elapsed_frames.saturating_add(1);
        let progress = (self.fade_elapsed_frames as f32 / self.fade_total_frames as f32)
            .clamp(0.0, 1.0);
        self.gain =
            self.fade_start_gain + (self.fade_target_gain - self.fade_start_gain) * progress;
        if self.fade_elapsed_frames >= self.fade_total_frames {
            self.gain = self.fade_target_gain;
            self.fade_total_frames = 0;
            if self.stop_after_fade {
                self.clear();
            }
        }
    }

    fn clear(&mut self) {
        self.clip = None;
        self.loop_key = None;
        self.looping = false;
        self.position = 0.0;
        self.gain = 0.0;
        self.fade_start_gain = 0.0;
        self.fade_target_gain = 0.0;
        self.fade_elapsed_frames = 0;
        self.fade_total_frames = 0;
        self.stop_after_fade = false;
    }
}


fn sanitize_gain(gain: f32) -> f32 {
    if gain.is_nan() {
        0.0
    } else {
        gain.clamp(0.0, 1.0)
    }
}

fn duration_frames(duration: Duration, sample_rate: u32) -> u64 {
    let frames = duration
        .as_nanos()
        .saturating_mul(u128::from(sample_rate))
        / 1_000_000_000;
    frames.min(u128::from(u64::MAX)) as u64
}

#[cfg(test)]
mod tests {
    use super::*;

    fn clip(sample_rate: u32, channels: u16, samples: Vec<f32>) -> Arc<AudioClip> {
        Arc::new(AudioClip::from_samples(sample_rate, channels, samples).unwrap())
    }

    fn system(sample_rate: u32, max_voices: usize) -> (SoundController, SoundMixer) {
        let controller = SoundController::new(SoundMixerConfig::new(sample_rate, max_voices))
            .unwrap();
        let mixer = controller.create_mixer();
        (controller, mixer)
    }

    #[test]
    fn clip_validation_rejects_invalid_pcm() {
        assert_eq!(
            AudioClip::from_samples(0, 1, vec![0.0]),
            Err(AudioClipError::InvalidSampleRate)
        );
        assert_eq!(
            AudioClip::from_samples(44_100, 3, vec![0.0, 0.0, 0.0]),
            Err(AudioClipError::UnsupportedChannelCount(3))
        );
        assert_eq!(
            AudioClip::from_samples(44_100, 2, vec![0.0]),
            Err(AudioClipError::IncompleteFrame {
                sample_count: 1,
                channels: 2,
            })
        );
        assert_eq!(
            AudioClip::from_samples(44_100, 1, vec![f32::NAN]),
            Err(AudioClipError::NonFiniteSample { index: 0 })
        );
    }

    #[test]
    fn one_shot_plays_mono_pcm_and_ends_at_source_duration() {
        let (controller, mut mixer) = system(4, 2);
        let sound = clip(4, 1, vec![0.1, 0.2, 0.3]);
        controller
            .play_one_shot(sound, SoundCategory::Effects, 1.0)
            .unwrap();

        assert_eq!(mixer.render_frame(), (0.1, 0.1));
        assert_eq!(mixer.render_frame(), (0.2, 0.2));
        assert_eq!(mixer.render_frame(), (0.3, 0.3));
        assert_eq!(mixer.render_frame(), (0.0, 0.0));
        assert_eq!(controller.status().active_voices, 0);
    }

    #[test]
    fn clip_is_linearly_resampled_to_the_mixer_rate() {
        let (controller, mut mixer) = system(4, 1);
        let sound = clip(2, 1, vec![0.0, 1.0]);
        controller
            .play_one_shot(sound, SoundCategory::Effects, 1.0)
            .unwrap();

        assert_eq!(mixer.render_frame(), (0.0, 0.0));
        assert_eq!(mixer.render_frame(), (0.5, 0.5));
        assert_eq!(mixer.render_frame(), (1.0, 1.0));
        assert_eq!(mixer.render_frame(), (1.0, 1.0));
        assert_eq!(mixer.render_frame(), (0.0, 0.0));
    }

    #[test]
    fn looping_wraps_and_same_key_replaces_without_accumulating_voices() {
        let (controller, mut mixer) = system(4, 2);
        controller
            .play_loop(
                LoopKey(7),
                clip(4, 1, vec![0.1, 0.2]),
                SoundCategory::Ambience,
                1.0,
                Duration::ZERO,
            )
            .unwrap();
        assert_eq!(mixer.render_frame(), (0.1, 0.1));
        assert_eq!(mixer.render_frame(), (0.2, 0.2));
        assert_eq!(mixer.render_frame(), (0.1, 0.1));

        controller
            .play_loop(
                LoopKey(7),
                clip(4, 1, vec![0.7, 0.8]),
                SoundCategory::Ambience,
                1.0,
                Duration::ZERO,
            )
            .unwrap();
        assert_eq!(mixer.render_frame(), (0.7, 0.7));
        assert_eq!(controller.status().active_voices, 1);
    }

    #[test]
    fn queued_loop_commands_are_applied_in_submission_order() {
        let (controller, mut mixer) = system(4, 1);
        controller
            .play_loop(
                LoopKey(19),
                clip(4, 1, vec![0.1; 4]),
                SoundCategory::Ambience,
                1.0,
                Duration::ZERO,
            )
            .unwrap();
        controller
            .play_loop(
                LoopKey(19),
                clip(4, 1, vec![0.8; 4]),
                SoundCategory::Ambience,
                1.0,
                Duration::ZERO,
            )
            .unwrap();

        assert_eq!(mixer.render_frame(), (0.8, 0.8));
        assert_eq!(controller.status().active_voices, 1);
    }

    #[test]
    fn controller_enforces_a_single_active_mixer() {
        let controller = SoundController::new(SoundMixerConfig::new(4, 1)).unwrap();
        let mixer = controller.create_mixer();
        assert!(matches!(
            controller.try_create_mixer(),
            Err(SoundMixerCreateError::AlreadyCreated)
        ));
        drop(mixer);
        assert!(controller.try_create_mixer().is_ok());
    }

    #[test]
    fn loop_fades_and_stops_at_the_requested_frame() {
        let (controller, mut mixer) = system(4, 1);
        controller
            .play_loop(
                LoopKey(1),
                clip(4, 1, vec![1.0; 8]),
                SoundCategory::Ambience,
                1.0,
                Duration::ZERO,
            )
            .unwrap();
        assert_eq!(mixer.render_frame(), (1.0, 1.0));

        controller
            .stop_loop(LoopKey(1), Duration::from_millis(500))
            .unwrap();
        assert_eq!(mixer.render_frame(), (1.0, 1.0));
        assert_eq!(mixer.render_frame(), (0.5, 0.5));
        assert_eq!(mixer.render_frame(), (0.0, 0.0));
        assert_eq!(controller.status().active_voices, 0);
    }

    #[test]
    fn category_gain_and_mute_are_independent() {
        let (controller, mut mixer) = system(4, 2);
        controller.set_category_volume(SoundCategory::Effects, 0.25);
        controller.set_category_volume(SoundCategory::Ambience, 0.75);
        controller.set_category_muted(SoundCategory::Effects, true);
        controller
            .play_one_shot(
                clip(4, 1, vec![1.0; 2]),
                SoundCategory::Effects,
                1.0,
            )
            .unwrap();
        controller
            .play_one_shot(
                clip(4, 1, vec![1.0; 2]),
                SoundCategory::Ambience,
                1.0,
            )
            .unwrap();

        assert_eq!(mixer.render_frame(), (0.75, 0.75));
        controller.set_category_muted(SoundCategory::Effects, false);
        assert_eq!(mixer.render_frame(), (1.0, 1.0));
    }

    #[test]
    fn voice_limit_rejects_newest_sound_and_preserves_active_voice() {
        let (controller, mut mixer) = system(4, 1);
        controller
            .play_one_shot(
                clip(4, 1, vec![0.2; 8]),
                SoundCategory::Effects,
                1.0,
            )
            .unwrap();
        controller
            .play_one_shot(
                clip(4, 1, vec![0.8; 8]),
                SoundCategory::Effects,
                1.0,
            )
            .unwrap();

        assert_eq!(mixer.render_frame(), (0.2, 0.2));
        assert_eq!(controller.status().active_voices, 1);
        assert_eq!(controller.status().rejected_voices, 1);
    }

    #[test]
    fn command_queue_is_bounded_and_reports_overflow() {
        let (controller, _mixer) = system(4, 1);
        let sound = clip(4, 1, vec![0.5; 8]);
        for _ in 0..COMMAND_QUEUE_CAPACITY {
            controller
                .play_one_shot(sound.clone(), SoundCategory::Effects, 1.0)
                .unwrap();
        }
        assert_eq!(
            controller.play_one_shot(sound, SoundCategory::Effects, 1.0),
            Err(SoundCommandError::QueueFull)
        );
    }

    #[test]
    fn identical_command_sequences_render_identically() {
        fn render_sequence() -> Vec<f32> {
            let (controller, mut mixer) = system(8, 4);
            let ambience = clip(4, 1, vec![0.2, 0.4, 0.6, 0.8]);
            let effect = clip(4, 2, vec![0.7, 0.3, 0.5, 0.1]);
            controller
                .play_loop(
                    LoopKey(4),
                    ambience,
                    SoundCategory::Ambience,
                    0.5,
                    Duration::from_millis(250),
                )
                .unwrap();
            controller
                .play_one_shot(effect, SoundCategory::Effects, 0.8)
                .unwrap();

            let mut output = Vec::new();
            for _ in 0..12 {
                output.extend_from_slice(&{
                    let (left, right) = mixer.render_frame();
                    [left, right]
                });
            }
            output
        }

        assert_eq!(render_sequence(), render_sequence());
    }

    #[test]
    fn invalid_or_extreme_gains_are_safely_clamped() {
        let (controller, mut mixer) = system(4, 1);
        controller
            .play_one_shot(
                clip(4, 1, vec![1.0]),
                SoundCategory::Effects,
                2.0,
            )
            .unwrap();
        assert_eq!(mixer.render_frame(), (1.0, 1.0));
        controller.set_category_volume(SoundCategory::Effects, f32::NAN);
        assert_eq!(controller.category_volume(SoundCategory::Effects), 0.0);
    }
}
