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
/// Maximum number of simultaneous sound voices permitted by [`SoundMixerConfig`].
pub const MAX_SOUND_VOICES: usize = 128;

/// The independent gain/mute buses exposed by the unified audio controller.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AudioBus {
    /// Procedurally composed background music and its synthesized stingers.
    Music,
    /// Discrete one-shot sound effects.
    Effects,
    /// Persistent environmental sounds.
    Ambience,
}

/// A non-musical routing category handled by [`SoundMixer`].
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum SoundCategory {
    /// Discrete one-shot effects such as impacts or footsteps.
    #[default]
    Effects,
    /// Persistent or long-running environmental sounds.
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
    /// Construct a clip from host-decoded floating-point PCM samples.
    ///
    /// Stereo samples are interleaved left/right. Mono samples are duplicated to both output
    /// channels during playback. Only mono and stereo are supported.
    pub fn from_samples(
        sample_rate: u32,
        channels: u16,
        samples: Vec<f32>,
    ) -> Result<Self, AudioClipError> {
        Self::from_shared_samples(sample_rate, channels, Arc::from(samples))
    }

    /// Construct a clip from shared host-owned PCM without copying the sample buffer.
    ///
    /// The host should retain another owning `Arc<AudioClip>` while the clip may be queued or
    /// playing, so its final sample allocation is not released by the render callback.
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
        if !samples.len().is_multiple_of(usize::from(channels)) {
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

    /// Source sample rate, in frames per second.
    pub fn sample_rate(&self) -> u32 {
        self.sample_rate
    }

    /// Number of interleaved channels: one for mono or two for stereo.
    pub fn channels(&self) -> u16 {
        self.channels
    }

    /// Number of complete PCM frames in this clip.
    pub fn frame_count(&self) -> usize {
        self.samples.len() / usize::from(self.channels)
    }

    /// Playback duration at the clip's source sample rate.
    pub fn duration(&self) -> Duration {
        Duration::from_secs_f64(self.frame_count() as f64 / f64::from(self.sample_rate))
    }
}

/// Validation failures returned when constructing an [`AudioClip`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum AudioClipError {
    /// The supplied sample rate was zero.
    InvalidSampleRate,
    /// The clip has a channel count other than mono or stereo.
    UnsupportedChannelCount(u16),
    /// The clip contains no PCM frames.
    Empty,
    /// The sample count is not divisible by the channel count.
    IncompleteFrame { sample_count: usize, channels: u16 },
    /// The indexed PCM sample is NaN or infinite.
    NonFiniteSample { index: usize },
}

impl fmt::Display for AudioClipError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidSampleRate => write!(f, "sample rate must be greater than zero"),
            Self::UnsupportedChannelCount(channels) => {
                write!(
                    f,
                    "only mono and stereo clips are supported, got {channels} channels"
                )
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

/// Maximum number of decoded frames permitted in one stream's prebuffer.
pub const MAX_STREAM_BUFFER_FRAMES: usize = 262_144;

/// Configuration for a bounded decoded-PCM stream buffer.
///
/// The producer must write frames at the mixer output sample rate. Decode and any required
/// resampling on the host's worker thread before calling StreamWriter::write_interleaved.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct StreamBufferConfig {
    /// PCM sample rate. Must equal the destination mixer sample rate.
    pub sample_rate: u32,
    /// Interleaved channel count: one for mono or two for stereo.
    pub channels: u16,
    /// Maximum number of frames retained by the bounded ring.
    pub capacity_frames: usize,
    /// Number of queued frames required before playback begins. Zero starts immediately.
    pub prebuffer_frames: usize,
}

impl StreamBufferConfig {
    /// Configure a stream buffer. Validation occurs when [`StreamBuffer::create`] is called.
    pub const fn new(
        sample_rate: u32,
        channels: u16,
        capacity_frames: usize,
        prebuffer_frames: usize,
    ) -> Self {
        Self {
            sample_rate,
            channels,
            capacity_frames,
            prebuffer_frames,
        }
    }
}

/// Defaults to 44.1 kHz stereo, two seconds of capacity, and a 100 ms startup prebuffer.
impl Default for StreamBufferConfig {
    fn default() -> Self {
        Self {
            sample_rate: 44_100,
            channels: 2,
            capacity_frames: 88_200,
            prebuffer_frames: 4_410,
        }
    }
}

/// Validation failures returned by [`StreamBuffer::create`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StreamBufferConfigError {
    /// The configured sample rate was zero.
    InvalidSampleRate,
    /// The configured channel count was not one or two.
    UnsupportedChannelCount(u16),
    /// Capacity was zero or exceeded [`MAX_STREAM_BUFFER_FRAMES`].
    InvalidCapacity { requested: usize, maximum: usize },
    /// The requested startup prebuffer exceeded the ring capacity.
    InvalidPrebuffer { requested: usize, capacity: usize },
}

impl fmt::Display for StreamBufferConfigError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidSampleRate => write!(f, "stream sample rate must be greater than zero"),
            Self::UnsupportedChannelCount(channels) => {
                write!(f, "stream channels must be mono or stereo, got {channels}")
            }
            Self::InvalidCapacity { requested, maximum } => write!(
                f,
                "stream capacity must be in 1..={maximum} frames, got {requested}"
            ),
            Self::InvalidPrebuffer {
                requested,
                capacity,
            } => write!(
                f,
                "prebuffer must not exceed capacity ({requested} > {capacity})"
            ),
        }
    }
}

impl Error for StreamBufferConfigError {}

/// Invalid-input failures returned by [`StreamWriter::write_interleaved`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StreamWriteError {
    /// The producer has already signaled end-of-stream with [`StreamWriter::finish`].
    Finished,
    /// The sample slice does not contain complete channel frames.
    IncompleteFrame { sample_count: usize, channels: u16 },
    /// The indexed sample is NaN or infinite.
    NonFiniteSample { index: usize },
}

impl fmt::Display for StreamWriteError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Finished => write!(f, "stream has already been finished"),
            Self::IncompleteFrame {
                sample_count,
                channels,
            } => write!(
                f,
                "{sample_count} samples do not form complete {channels}-channel frames"
            ),
            Self::NonFiniteSample { index } => write!(f, "sample {index} is not finite"),
        }
    }
}

impl Error for StreamWriteError {}

/// Creates a bounded stream shared by one decoder worker and one audio consumer.
pub struct StreamBuffer;

impl StreamBuffer {
    /// Allocate the ring before playback and return its single producer and shared stream handle.
    pub fn create(
        config: StreamBufferConfig,
    ) -> Result<(StreamWriter, Arc<AudioStream>), StreamBufferConfigError> {
        if config.sample_rate == 0 {
            return Err(StreamBufferConfigError::InvalidSampleRate);
        }
        if !matches!(config.channels, 1 | 2) {
            return Err(StreamBufferConfigError::UnsupportedChannelCount(
                config.channels,
            ));
        }
        if config.capacity_frames == 0 || config.capacity_frames > MAX_STREAM_BUFFER_FRAMES {
            return Err(StreamBufferConfigError::InvalidCapacity {
                requested: config.capacity_frames,
                maximum: MAX_STREAM_BUFFER_FRAMES,
            });
        }
        if config.prebuffer_frames > config.capacity_frames {
            return Err(StreamBufferConfigError::InvalidPrebuffer {
                requested: config.prebuffer_frames,
                capacity: config.capacity_frames,
            });
        }

        let shared = Arc::new(StreamShared {
            config,
            frames: SpscFrameRing::new(config.capacity_frames),
            finished: AtomicBool::new(false),
        });
        let status = Arc::new(StreamStatus {
            consumer_active: AtomicBool::new(false),
            playing: AtomicBool::new(false),
            underrun_frames: AtomicU64::new(0),
        });
        Ok((
            StreamWriter {
                shared: shared.clone(),
                _status: status.clone(),
            },
            Arc::new(AudioStream { shared, status }),
        ))
    }
}

/// Host-side handle used to feed decoded frames from a worker thread.
///
/// Writes are non-blocking and may be partial. A return value of zero means the ring is full;
/// retry after the audio consumer has advanced it. The writer is intentionally not cloneable.
pub struct StreamWriter {
    shared: Arc<StreamShared>,
    _status: Arc<StreamStatus>,
}

impl StreamWriter {
    /// Write interleaved mono or stereo frames. Returns the number of frames accepted.
    pub fn write_interleaved(&mut self, samples: &[f32]) -> Result<usize, StreamWriteError> {
        if self.shared.finished.load(Ordering::Acquire) {
            return Err(StreamWriteError::Finished);
        }
        let channels = usize::from(self.shared.config.channels);
        if !samples.len().is_multiple_of(channels) {
            return Err(StreamWriteError::IncompleteFrame {
                sample_count: samples.len(),
                channels: self.shared.config.channels,
            });
        }
        if let Some(index) = samples.iter().position(|sample| !sample.is_finite()) {
            return Err(StreamWriteError::NonFiniteSample { index });
        }

        let mut accepted = 0;
        for frame in samples.chunks_exact(channels) {
            let frame = if channels == 1 {
                [frame[0], 0.0]
            } else {
                [frame[0], frame[1]]
            };
            if !self.shared.frames.push(frame) {
                break;
            }
            accepted += 1;
        }
        Ok(accepted)
    }

    /// Mark the producer side complete. The mixer drains queued frames before ending playback.
    pub fn finish(&mut self) {
        self.shared.finished.store(true, Ordering::Release);
    }

    /// Number of frames currently waiting in the ring.
    pub fn available_frames(&self) -> usize {
        self.shared.frames.available()
    }

    /// Whether the producer has signaled end-of-stream.
    pub fn is_finished(&self) -> bool {
        self.shared.finished.load(Ordering::Acquire)
    }
}

/// Shared handle submitted to the mixer. Keep this handle and its StreamWriter alive until
/// is_active() becomes false; that keeps ring-buffer storage from being deallocated by the
/// render callback when a stream finishes or is stopped.
pub struct AudioStream {
    shared: Arc<StreamShared>,
    status: Arc<StreamStatus>,
}

impl AudioStream {
    /// PCM sample rate required by this stream and its destination mixer.
    pub fn sample_rate(&self) -> u32 {
        self.shared.config.sample_rate
    }
    /// Number of interleaved channels: one for mono or two for stereo.
    pub fn channels(&self) -> u16 {
        self.shared.config.channels
    }
    /// Maximum number of frames the ring can hold.
    pub fn capacity_frames(&self) -> usize {
        self.shared.config.capacity_frames
    }
    /// Number of queued frames required before the mixer starts consuming.
    pub fn prebuffer_frames(&self) -> usize {
        self.shared.config.prebuffer_frames
    }
    /// Number of decoded frames currently queued for playback.
    pub fn available_frames(&self) -> usize {
        self.shared.frames.available()
    }
    /// Whether the producer has declared end-of-stream.
    pub fn is_finished(&self) -> bool {
        self.shared.finished.load(Ordering::Acquire)
    }
    /// True from successful command submission until the mixer releases the stream voice.
    pub fn is_active(&self) -> bool {
        self.status.consumer_active.load(Ordering::Acquire)
    }
    /// True after the startup prebuffer has been reached and frames are being consumed.
    pub fn is_playing(&self) -> bool {
        self.status.playing.load(Ordering::Acquire)
    }
    /// Cumulative output frames for which playback had started but no input frame was available.
    pub fn underrun_frames(&self) -> u64 {
        self.status.underrun_frames.load(Ordering::Relaxed)
    }
}

struct StreamShared {
    config: StreamBufferConfig,
    frames: SpscFrameRing,
    finished: AtomicBool,
}

struct StreamStatus {
    consumer_active: AtomicBool,
    playing: AtomicBool,
    underrun_frames: AtomicU64,
}

/// Fixed-capacity SPSC ring of decoded stereo frame slots. Mono input uses the left slot.
struct SpscFrameRing {
    slots: Vec<UnsafeCell<MaybeUninit<[f32; 2]>>>,
    write: AtomicUsize,
    read: AtomicUsize,
    capacity: usize,
}

// SAFETY: the writer owns the write index and the mixer owns the read index. Release/acquire
// publication transfers each initialized frame without concurrent access to a slot.
unsafe impl Send for SpscFrameRing {}
// SAFETY: StreamBuffer creates one non-cloneable writer and the mixer enforces one consumer.
unsafe impl Sync for SpscFrameRing {}

impl SpscFrameRing {
    fn new(capacity: usize) -> Self {
        Self {
            slots: (0..capacity)
                .map(|_| UnsafeCell::new(MaybeUninit::uninit()))
                .collect(),
            write: AtomicUsize::new(0),
            read: AtomicUsize::new(0),
            capacity,
        }
    }

    fn available(&self) -> usize {
        let write = self.write.load(Ordering::Acquire);
        let read = self.read.load(Ordering::Relaxed);
        write.wrapping_sub(read).min(self.capacity)
    }

    fn push(&self, frame: [f32; 2]) -> bool {
        let write = self.write.load(Ordering::Relaxed);
        let read = self.read.load(Ordering::Acquire);
        if write.wrapping_sub(read) >= self.capacity {
            return false;
        }
        // SAFETY: the producer owns this slot until publishing the next write index.
        unsafe {
            (*self.slots[write % self.capacity].get()).write(frame);
        }
        self.write.store(write.wrapping_add(1), Ordering::Release);
        true
    }

    fn pop(&self) -> Option<[f32; 2]> {
        let read = self.read.load(Ordering::Relaxed);
        let write = self.write.load(Ordering::Acquire);
        if read == write {
            return None;
        }
        // SAFETY: acquire observes the initialized slot; only the consumer advances read.
        let frame = unsafe { (*self.slots[read % self.capacity].get()).assume_init_read() };
        self.read.store(read.wrapping_add(1), Ordering::Release);
        Some(frame)
    }
}

/// Configuration for a bounded, engine-independent sound mixer.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SoundMixerConfig {
    /// Stereo output sample rate, in frames per second.
    pub sample_rate: u32,
    /// Maximum simultaneous resident/streamed voices. Valid range is 1 through 128.
    pub max_voices: usize,
}

impl SoundMixerConfig {
    /// Set the output rate and maximum number of simultaneous sound voices.
    pub const fn new(sample_rate: u32, max_voices: usize) -> Self {
        Self {
            sample_rate,
            max_voices,
        }
    }
}

/// Defaults to 44.1 kHz output and a 32-voice pool.
impl Default for SoundMixerConfig {
    fn default() -> Self {
        Self {
            sample_rate: 44_100,
            max_voices: 32,
        }
    }
}

/// Validation failures returned by [`SoundController::new`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SoundMixerConfigError {
    /// The configured output sample rate was zero.
    InvalidSampleRate,
    /// The voice limit was zero or exceeded [`MAX_SOUND_VOICES`].
    InvalidVoiceLimit { requested: usize, maximum: usize },
}

impl fmt::Display for SoundMixerConfigError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidSampleRate => write!(f, "mixer sample rate must be greater than zero"),
            Self::InvalidVoiceLimit { requested, maximum } => {
                write!(f, "voice limit must be in 1..={maximum}, got {requested}")
            }
        }
    }
}

impl Error for SoundMixerConfigError {}

/// Failure to create a second active consumer for one [`SoundController`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SoundMixerCreateError {
    /// This controller already has an active mixer consuming its queue.
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
    /// The stream's PCM rate does not match this mixer's output rate.
    StreamSampleRateMismatch {
        stream_sample_rate: u32,
        mixer_sample_rate: u32,
    },
    /// The stream is already queued for or attached to a mixer voice.
    StreamAlreadyActive,
}

impl fmt::Display for SoundCommandError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::QueueFull => write!(f, "sound command queue is full"),
            Self::ProducerBusy => write!(f, "another thread is submitting a sound command"),
            Self::StreamSampleRateMismatch { stream_sample_rate, mixer_sample_rate } => write!(f, "stream sample rate {stream_sample_rate} does not match mixer rate {mixer_sample_rate}"),
            Self::StreamAlreadyActive => write!(f, "stream is already queued or playing"),
        }
    }
}

impl Error for SoundCommandError {}

/// Snapshot of the mixer's bounded voice usage and rejected-play count.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SoundMixerStatus {
    /// Number of voices currently active in the mixer.
    pub active_voices: usize,
    /// Configured maximum number of simultaneous voices.
    pub max_voices: usize,
    /// Cumulative play requests rejected because all voices were occupied.
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
    /// Create a control handle and its bounded command queue.
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

    /// Try to create the controller's single queue consumer without panicking.
    ///
    /// The mixer must be created before the host starts its audio callback.
    pub fn try_create_mixer(&self) -> Result<SoundMixer, SoundMixerCreateError> {
        self.shared
            .consumer_active
            .compare_exchange(false, true, Ordering::Acquire, Ordering::Relaxed)
            .map_err(|_| SoundMixerCreateError::AlreadyCreated)?;
        Ok(SoundMixer::new(self.shared.clone()))
    }

    /// Configured output sample rate, in frames per second.
    pub fn sample_rate(&self) -> u32 {
        self.shared.config.sample_rate
    }

    /// Configured maximum number of simultaneous voices.
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
    pub fn stop_loop(&self, key: LoopKey, fade_out: Duration) -> Result<(), SoundCommandError> {
        self.submit(AudioCommand::StopLoop {
            key,
            duration_frames: duration_frames(fade_out, self.sample_rate()),
        })
    }

    /// Queue a decoded stream as a one-shot. The stream must use this mixer's sample rate.
    pub fn play_stream(
        &self,
        stream: Arc<AudioStream>,
        category: SoundCategory,
        gain: f32,
    ) -> Result<(), SoundCommandError> {
        self.play_stream_inner(None, stream, category, gain, Duration::ZERO)
    }

    /// Start or replace a keyed streaming source. The host worker may continuously feed a
    /// looping source; stop_loop applies the normal fade semantics to this keyed voice.
    pub fn play_stream_loop(
        &self,
        key: LoopKey,
        stream: Arc<AudioStream>,
        category: SoundCategory,
        gain: f32,
        fade_in: Duration,
    ) -> Result<(), SoundCommandError> {
        self.play_stream_inner(Some(key), stream, category, gain, fade_in)
    }

    fn play_stream_inner(
        &self,
        key: Option<LoopKey>,
        stream: Arc<AudioStream>,
        category: SoundCategory,
        gain: f32,
        fade_in: Duration,
    ) -> Result<(), SoundCommandError> {
        if stream.sample_rate() != self.sample_rate() {
            return Err(SoundCommandError::StreamSampleRateMismatch {
                stream_sample_rate: stream.sample_rate(),
                mixer_sample_rate: self.sample_rate(),
            });
        }
        if stream
            .status
            .consumer_active
            .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
            .is_err()
        {
            return Err(SoundCommandError::StreamAlreadyActive);
        }
        let status = stream.status.clone();
        let result = self.submit(AudioCommand::PlayStream {
            key,
            stream,
            category,
            gain: sanitize_gain(gain),
            fade_in_frames: duration_frames(fade_in, self.sample_rate()),
        });
        if result.is_err() {
            status.consumer_active.store(false, Ordering::Release);
        }
        result
    }

    /// Set a category's linear gain. The value is sanitized and clamped to 0.0..=1.0.
    pub fn set_category_volume(&self, category: SoundCategory, gain: f32) {
        self.shared.category_gain[category.index()]
            .store(sanitize_gain(gain).to_bits(), Ordering::Release);
    }

    /// Current configured linear gain for the selected category.
    pub fn category_volume(&self, category: SoundCategory) -> f32 {
        f32::from_bits(self.shared.category_gain[category.index()].load(Ordering::Acquire))
    }

    /// Mute or unmute a category without changing its configured gain.
    pub fn set_category_muted(&self, category: SoundCategory, muted: bool) {
        self.shared.category_muted[category.index()].store(muted, Ordering::Release);
    }

    /// Whether the selected category is muted.
    pub fn category_muted(&self, category: SoundCategory) -> bool {
        self.shared.category_muted[category.index()].load(Ordering::Acquire)
    }

    /// Snapshot voice usage and the number of play requests rejected by the voice limit.
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
    PlayStream {
        key: Option<LoopKey>,
        stream: Arc<AudioStream>,
        category: SoundCategory,
        gain: f32,
        fade_in_frames: u64,
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
/// 64 queued commands per frame, reads immutable resident PCM or prebuffered stream frames,
/// mixes active voices, and performs no file access or decoding.
pub struct SoundMixer {
    shared: Arc<SoundShared>,
    voices: Vec<Voice>,
    sample_rate: u32,
    _consumer_guard: ConsumerGuard,
}

struct ConsumerGuard(Arc<SoundShared>);

impl Drop for ConsumerGuard {
    fn drop(&mut self) {
        while let Some(command) = self.0.commands.pop() {
            if let AudioCommand::PlayStream { stream, .. } = command {
                release_stream(stream);
            }
        }
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

    /// Render one stereo frame, consuming queued commands and active sound voices.
    ///
    /// The host is responsible for mixing this with music and applying any final master
    /// limiter/clipping policy.
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
            if voice.is_idle() {
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
            if voice.is_active() {
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
    /// Fill an interleaved stereo slice ordered L, R, L, R, ...
    ///
    /// Any trailing sample that does not form a complete stereo frame is left untouched.
    pub fn render_interleaved(&mut self, out: &mut [f32]) {
        for frame in out.as_chunks_mut::<2>().0 {
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
                if let Some(voice) = self.voices.iter_mut().find(|voice| voice.is_idle()) {
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
                    voice.is_active() && voice.looping && voice.loop_key == Some(key)
                });
                if let Some(voice) = existing {
                    voice.start(clip, Some(key), true, category, gain, fade_in_frames);
                } else if let Some(voice) = self.voices.iter_mut().find(|voice| voice.is_idle()) {
                    voice.start(clip, Some(key), true, category, gain, fade_in_frames);
                } else {
                    self.shared.rejected_voices.fetch_add(1, Ordering::Relaxed);
                }
            }
            AudioCommand::PlayStream {
                key,
                stream,
                category,
                gain,
                fade_in_frames,
            } => {
                let existing = key.and_then(|loop_key| {
                    self.voices.iter_mut().find(|voice| {
                        voice.is_active() && voice.looping && voice.loop_key == Some(loop_key)
                    })
                });
                if let Some(voice) = existing {
                    voice.start_stream(stream, key, category, gain, fade_in_frames);
                } else if let Some(voice) = self.voices.iter_mut().find(|voice| voice.is_idle()) {
                    voice.start_stream(stream, key, category, gain, fade_in_frames);
                } else {
                    stream.status.playing.store(false, Ordering::Release);
                    stream
                        .status
                        .consumer_active
                        .store(false, Ordering::Release);
                    self.shared.rejected_voices.fetch_add(1, Ordering::Relaxed);
                }
            }
            AudioCommand::FadeLoop {
                key,
                gain,
                duration_frames,
            } => {
                if let Some(voice) = self
                    .voices
                    .iter_mut()
                    .find(|voice| voice.is_active() && voice.looping && voice.loop_key == Some(key))
                {
                    voice.fade_to(gain, duration_frames, false);
                }
            }
            AudioCommand::StopLoop {
                key,
                duration_frames,
            } => {
                if let Some(voice) = self
                    .voices
                    .iter_mut()
                    .find(|voice| voice.is_active() && voice.looping && voice.loop_key == Some(key))
                {
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
    stream: Option<Arc<AudioStream>>,
    stream_started: bool,
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
    fn is_active(&self) -> bool {
        self.clip.is_some() || self.stream.is_some()
    }

    fn is_idle(&self) -> bool {
        !self.is_active()
    }

    fn start(
        &mut self,
        clip: Arc<AudioClip>,
        loop_key: Option<LoopKey>,
        looping: bool,
        category: SoundCategory,
        gain: f32,
        fade_in_frames: u64,
    ) {
        if let Some(previous) = self.stream.take() {
            release_stream(previous);
        }
        self.clip = Some(clip);
        self.stream_started = false;
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

    fn start_stream(
        &mut self,
        stream: Arc<AudioStream>,
        loop_key: Option<LoopKey>,
        category: SoundCategory,
        gain: f32,
        fade_in_frames: u64,
    ) {
        if let Some(previous) = self.stream.take() {
            release_stream(previous);
        }
        self.clip = None;
        self.stream = Some(stream);
        self.stream_started = false;
        self.category = category;
        self.loop_key = loop_key;
        self.looping = loop_key.is_some();
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
        if let Some(stream) = self.stream.take() {
            let (output, finished) = self.render_stream(&stream, bus_gain);
            if finished {
                release_stream(stream);
                self.clear();
            } else {
                self.stream = Some(stream);
            }
            return output;
        }

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
        if self.advance_fade() {
            self.clear();
        }
        if !self.looping && self.position >= frame_count as f64 {
            self.clear();
        }
        output
    }

    fn render_stream(&mut self, stream: &AudioStream, bus_gain: f32) -> ((f32, f32), bool) {
        let available = stream.shared.frames.available();
        let finished = stream.shared.finished.load(Ordering::Acquire);
        if !self.stream_started {
            if available < stream.shared.config.prebuffer_frames && !finished {
                return ((0.0, 0.0), false);
            }
            if available == 0 && finished {
                return ((0.0, 0.0), true);
            }
            self.stream_started = true;
            stream.status.playing.store(true, Ordering::Release);
        }

        let Some(frame) = stream.shared.frames.pop() else {
            if finished {
                return ((0.0, 0.0), true);
            }
            stream
                .status
                .underrun_frames
                .fetch_add(1, Ordering::Relaxed);
            return ((0.0, 0.0), false);
        };
        let left = frame[0];
        let right = if stream.shared.config.channels == 1 {
            left
        } else {
            frame[1]
        };
        let output = (left * self.gain * bus_gain, right * self.gain * bus_gain);
        let stop_after_fade = self.advance_fade();
        let drained =
            stream.shared.finished.load(Ordering::Acquire) && stream.shared.frames.available() == 0;
        (output, stop_after_fade || drained)
    }

    /// Returns true when a completed fade requested voice removal.
    fn advance_fade(&mut self) -> bool {
        if self.fade_total_frames == 0 {
            return false;
        }
        self.fade_elapsed_frames = self.fade_elapsed_frames.saturating_add(1);
        let progress =
            (self.fade_elapsed_frames as f32 / self.fade_total_frames as f32).clamp(0.0, 1.0);
        self.gain =
            self.fade_start_gain + (self.fade_target_gain - self.fade_start_gain) * progress;
        if self.fade_elapsed_frames >= self.fade_total_frames {
            self.gain = self.fade_target_gain;
            self.fade_total_frames = 0;
            return self.stop_after_fade;
        }
        false
    }

    fn clear(&mut self) {
        self.clip = None;
        if let Some(stream) = self.stream.take() {
            release_stream(stream);
        }
        self.stream_started = false;
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

// Release the large ring allocation before publishing inactivity. The host retains the
// writer/stream handles while active, so the buffer is reclaimed away from the render callback.
fn release_stream(stream: Arc<AudioStream>) {
    let status = stream.status.clone();
    status.playing.store(false, Ordering::Release);
    drop(stream);
    status.consumer_active.store(false, Ordering::Release);
}

impl Drop for Voice {
    fn drop(&mut self) {
        self.clear();
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
    let frames = duration.as_nanos().saturating_mul(u128::from(sample_rate)) / 1_000_000_000;
    frames.min(u128::from(u64::MAX)) as u64
}

#[cfg(test)]
mod tests {
    use super::*;

    fn clip(sample_rate: u32, channels: u16, samples: Vec<f32>) -> Arc<AudioClip> {
        Arc::new(AudioClip::from_samples(sample_rate, channels, samples).unwrap())
    }

    fn system(sample_rate: u32, max_voices: usize) -> (SoundController, SoundMixer) {
        let controller =
            SoundController::new(SoundMixerConfig::new(sample_rate, max_voices)).unwrap();
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
            .play_one_shot(clip(4, 1, vec![1.0; 2]), SoundCategory::Effects, 1.0)
            .unwrap();
        controller
            .play_one_shot(clip(4, 1, vec![1.0; 2]), SoundCategory::Ambience, 1.0)
            .unwrap();

        assert_eq!(mixer.render_frame(), (0.75, 0.75));
        controller.set_category_muted(SoundCategory::Effects, false);
        assert_eq!(mixer.render_frame(), (1.0, 1.0));
    }

    #[test]
    fn voice_limit_rejects_newest_sound_and_preserves_active_voice() {
        let (controller, mut mixer) = system(4, 1);
        controller
            .play_one_shot(clip(4, 1, vec![0.2; 8]), SoundCategory::Effects, 1.0)
            .unwrap();
        controller
            .play_one_shot(clip(4, 1, vec![0.8; 8]), SoundCategory::Effects, 1.0)
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
    fn stream_waits_for_prebuffer_then_drains_after_finish() {
        let (controller, mut mixer) = system(4, 2);
        let (mut writer, stream) =
            StreamBuffer::create(StreamBufferConfig::new(4, 1, 4, 3)).unwrap();
        assert_eq!(writer.write_interleaved(&[0.1, 0.2]), Ok(2));
        controller
            .play_stream(stream.clone(), SoundCategory::Ambience, 1.0)
            .unwrap();
        assert!(stream.is_active());
        assert_eq!(
            controller.play_stream(stream.clone(), SoundCategory::Ambience, 1.0),
            Err(SoundCommandError::StreamAlreadyActive)
        );
        assert_eq!(mixer.render_frame(), (0.0, 0.0));
        assert!(!stream.is_playing());
        assert_eq!(writer.write_interleaved(&[0.3]), Ok(1));
        assert_eq!(mixer.render_frame(), (0.1, 0.1));
        assert!(stream.is_playing());
        assert_eq!(mixer.render_frame(), (0.2, 0.2));
        assert_eq!(mixer.render_frame(), (0.3, 0.3));
        writer.finish();
        assert_eq!(mixer.render_frame(), (0.0, 0.0));
        assert!(!stream.is_playing());
        assert_eq!(controller.status().active_voices, 0);
    }

    #[test]
    fn stream_underflow_outputs_silence_and_resumes_when_fed() {
        let (controller, mut mixer) = system(4, 1);
        let (mut writer, stream) =
            StreamBuffer::create(StreamBufferConfig::new(4, 1, 4, 1)).unwrap();
        writer.write_interleaved(&[0.5]).unwrap();
        controller
            .play_stream(stream.clone(), SoundCategory::Effects, 1.0)
            .unwrap();
        assert_eq!(mixer.render_frame(), (0.5, 0.5));
        assert_eq!(mixer.render_frame(), (0.0, 0.0));
        assert_eq!(stream.underrun_frames(), 1);
        assert_eq!(writer.write_interleaved(&[0.75]), Ok(1));
        assert_eq!(mixer.render_frame(), (0.75, 0.75));
        writer.finish();
        assert_eq!(mixer.render_frame(), (0.0, 0.0));
    }

    #[test]
    fn stream_writer_is_bounded_and_reports_partial_writes() {
        let (mut writer, stream) =
            StreamBuffer::create(StreamBufferConfig::new(4, 2, 2, 1)).unwrap();
        assert_eq!(
            writer.write_interleaved(&[0.1, 0.2, 0.3, 0.4, 0.5, 0.6]),
            Ok(2)
        );
        assert_eq!(writer.available_frames(), 2);
        assert_eq!(writer.write_interleaved(&[0.7, 0.8]), Ok(0));
        assert_eq!(stream.available_frames(), 2);
        assert_eq!(
            writer.write_interleaved(&[0.1]),
            Err(StreamWriteError::IncompleteFrame {
                sample_count: 1,
                channels: 2
            })
        );
        assert_eq!(
            writer.write_interleaved(&[f32::NAN, 0.0]),
            Err(StreamWriteError::NonFiniteSample { index: 0 })
        );
    }

    #[test]
    fn stream_requires_mixer_sample_rate_and_finishes_short_prebuffer() {
        let (controller, mut mixer) = system(8, 1);
        let (_, stream) = StreamBuffer::create(StreamBufferConfig::new(4, 1, 4, 3)).unwrap();
        assert_eq!(
            controller.play_stream(stream.clone(), SoundCategory::Effects, 1.0),
            Err(SoundCommandError::StreamSampleRateMismatch {
                stream_sample_rate: 4,
                mixer_sample_rate: 8
            })
        );
        let (mut matching_writer, matching_stream) =
            StreamBuffer::create(StreamBufferConfig::new(8, 1, 4, 3)).unwrap();
        matching_writer.write_interleaved(&[0.4]).unwrap();
        matching_writer.finish();
        controller
            .play_stream(matching_stream.clone(), SoundCategory::Effects, 1.0)
            .unwrap();
        assert_eq!(mixer.render_frame(), (0.4, 0.4));
        assert_eq!(mixer.render_frame(), (0.0, 0.0));
        assert!(!matching_stream.is_playing());
    }

    #[test]
    fn keyed_stream_loop_supports_fade_and_stop() {
        let (controller, mut mixer) = system(4, 1);
        let (mut writer, stream) =
            StreamBuffer::create(StreamBufferConfig::new(4, 1, 8, 1)).unwrap();
        writer.write_interleaved(&[1.0; 8]).unwrap();
        controller
            .play_stream_loop(
                LoopKey(88),
                stream.clone(),
                SoundCategory::Ambience,
                1.0,
                Duration::ZERO,
            )
            .unwrap();
        assert_eq!(mixer.render_frame(), (1.0, 1.0));
        controller
            .stop_loop(LoopKey(88), Duration::from_millis(500))
            .unwrap();
        assert_eq!(mixer.render_frame(), (1.0, 1.0));
        assert_eq!(mixer.render_frame(), (0.5, 0.5));
        assert_eq!(mixer.render_frame(), (0.0, 0.0));
        assert!(!stream.is_playing());
    }

    #[test]
    fn stream_reservation_is_released_when_command_queue_is_full() {
        let (controller, _mixer) = system(4, 2);
        let sound = clip(4, 1, vec![0.2; 4]);
        for _ in 0..COMMAND_QUEUE_CAPACITY {
            controller
                .play_one_shot(sound.clone(), SoundCategory::Effects, 1.0)
                .unwrap();
        }
        let (_, stream) = StreamBuffer::create(StreamBufferConfig::new(4, 1, 4, 1)).unwrap();
        assert_eq!(
            controller.play_stream(stream.clone(), SoundCategory::Effects, 1.0),
            Err(SoundCommandError::QueueFull)
        );
        assert!(!stream.is_active());
    }

    #[test]
    fn stream_voice_limit_rejection_releases_stream_reservation() {
        let (controller, mut mixer) = system(4, 1);
        controller
            .play_one_shot(clip(4, 1, vec![0.2; 8]), SoundCategory::Effects, 1.0)
            .unwrap();
        let (_, stream) = StreamBuffer::create(StreamBufferConfig::new(4, 1, 4, 1)).unwrap();
        controller
            .play_stream(stream.clone(), SoundCategory::Effects, 1.0)
            .unwrap();
        mixer.render_frame();
        assert!(!stream.is_active());
        assert_eq!(controller.status().rejected_voices, 1);
    }

    #[test]
    fn streaming_stereo_frames_preserve_channel_separation() {
        let (controller, mut mixer) = system(4, 1);
        let (mut writer, stream) =
            StreamBuffer::create(StreamBufferConfig::new(4, 2, 4, 1)).unwrap();
        writer.write_interleaved(&[0.25, -0.5]).unwrap();
        writer.finish();
        controller
            .play_stream(stream.clone(), SoundCategory::Effects, 1.0)
            .unwrap();
        assert_eq!(mixer.render_frame(), (0.25, -0.5));
        assert_eq!(mixer.render_frame(), (0.0, 0.0));
        assert!(!stream.is_active());
    }

    #[test]
    fn stream_ring_preserves_frame_order_with_concurrent_producer() {
        let (controller, mut mixer) = system(48_000, 1);
        let (mut writer, stream) =
            StreamBuffer::create(StreamBufferConfig::new(48_000, 1, 8, 1)).unwrap();
        controller
            .play_stream(stream.clone(), SoundCategory::Effects, 1.0)
            .unwrap();

        let producer = std::thread::spawn(move || {
            for index in 0..256 {
                let sample = 0.1 + index as f32 * 0.001;
                loop {
                    match writer.write_interleaved(&[sample]) {
                        Ok(1) => break,
                        Ok(0) => std::thread::yield_now(),
                        Ok(_) => unreachable!("one input frame cannot accept multiple frames"),
                        Err(error) => panic!("unexpected stream write error: {error}"),
                    }
                }
            }
            writer.finish();
        });

        let mut rendered = Vec::new();
        while stream.is_active() {
            let (left, _) = mixer.render_frame();
            if left != 0.0 {
                rendered.push(left);
            }
            std::thread::yield_now();
        }
        producer.join().unwrap();

        let expected = (0..256)
            .map(|index| 0.1 + index as f32 * 0.001)
            .collect::<Vec<_>>();
        assert_eq!(rendered, expected);
    }

    #[test]
    fn keyed_stream_replacement_releases_old_source_without_adding_a_voice() {
        let (controller, mut mixer) = system(4, 1);
        let (mut old_writer, old_stream) =
            StreamBuffer::create(StreamBufferConfig::new(4, 1, 4, 1)).unwrap();
        old_writer.write_interleaved(&[0.2; 4]).unwrap();
        controller
            .play_stream_loop(
                LoopKey(91),
                old_stream.clone(),
                SoundCategory::Ambience,
                1.0,
                Duration::ZERO,
            )
            .unwrap();
        assert_eq!(mixer.render_frame(), (0.2, 0.2));

        let (mut new_writer, new_stream) =
            StreamBuffer::create(StreamBufferConfig::new(4, 1, 4, 1)).unwrap();
        new_writer.write_interleaved(&[0.8; 4]).unwrap();
        controller
            .play_stream_loop(
                LoopKey(91),
                new_stream.clone(),
                SoundCategory::Ambience,
                1.0,
                Duration::ZERO,
            )
            .unwrap();

        assert_eq!(mixer.render_frame(), (0.8, 0.8));
        assert!(!old_stream.is_active());
        assert!(new_stream.is_playing());
        assert_eq!(controller.status().active_voices, 1);
    }

    #[test]
    fn stream_config_and_finished_writer_are_validated() {
        assert!(matches!(
            StreamBuffer::create(StreamBufferConfig::new(0, 1, 4, 1)),
            Err(StreamBufferConfigError::InvalidSampleRate)
        ));
        assert!(matches!(
            StreamBuffer::create(StreamBufferConfig::new(4, 3, 4, 1)),
            Err(StreamBufferConfigError::UnsupportedChannelCount(3))
        ));
        assert!(matches!(
            StreamBuffer::create(StreamBufferConfig::new(
                4,
                1,
                MAX_STREAM_BUFFER_FRAMES + 1,
                1
            )),
            Err(StreamBufferConfigError::InvalidCapacity { .. })
        ));
        assert!(matches!(
            StreamBuffer::create(StreamBufferConfig::new(4, 1, 2, 3)),
            Err(StreamBufferConfigError::InvalidPrebuffer { .. })
        ));
        let (mut writer, _) = StreamBuffer::create(StreamBufferConfig::new(4, 1, 2, 1)).unwrap();
        writer.finish();
        assert_eq!(
            writer.write_interleaved(&[0.1]),
            Err(StreamWriteError::Finished)
        );
    }

    #[test]
    fn invalid_or_extreme_gains_are_safely_clamped() {
        let (controller, mut mixer) = system(4, 1);
        controller
            .play_one_shot(clip(4, 1, vec![1.0]), SoundCategory::Effects, 2.0)
            .unwrap();
        assert_eq!(mixer.render_frame(), (1.0, 1.0));
        controller.set_category_volume(SoundCategory::Effects, f32::NAN);
        assert_eq!(controller.category_volume(SoundCategory::Effects), 0.0);
    }
}
