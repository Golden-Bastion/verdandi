# Architecture and real-time contract

Verdandi is split into reusable audio systems with a small optional host integration. The engine-independent core does not depend on Bevy or an audio device.

## Subsystems

| Subsystem | Main public types | Responsibility |
|---|---|---|
| Theme and composition | **Theme**, **ThemeOverrides**, **Composer**, **Layer**, **LoopPlan**, **NoteEvent** | Seed-derived musical frame and four-bar layer patterns |
| Music rendering | **MusicConfig**, **MusicShared**, **Engine**, **MusicStatus** | Sequencing, synthesis, transport, stingers, and music gain |
| Sample instruments | **SfzInstrument**, **SfzSampler**, **InstrumentConfig** | Optional decoded WAV sample playback from SFZ mappings |
| Procedural sound generation | **ProceduralGenerator**, **WindGenerator**, **RainGenerator**, **FootstepGenerator**, **ImpactGenerator** | Deterministic, game-oriented effects and ambience rendered into caller-owned buffers |
| Sound playback | **AudioClip**, **SoundController**, **SoundMixer** | Resident PCM one-shots, keyed loops, fades, categories, and voice limits |
| Sound streaming | **StreamBuffer**, **StreamWriter**, **AudioStream** | Bounded decoded-PCM producer/consumer path |
| Bevy adapter | **ProceduralMusicPlugin**, **AudioController**, **MusicTrack** | Bevy audio source, controller resource, and combined music/sound output |

## Ownership model

Verdandi provides reusable game-audio concepts and sound generators, but does not own the application's asset catalogue or decide where the application stores audio files. The host decides when a gameplay sound should occur and supplies any host-owned assets; Verdandi handles the reusable sound-generation and playback behavior.

- The host loads and decodes resident effects, constructs **AudioClip** values, and retains its owning **Arc<AudioClip>** references while they can be queued or playing.
- The host loads SFZ patches and their referenced WAV samples through **SfzInstrument::load** or **load_with_sample_root** before constructing the music engine.
- For streaming, the host owns the decoder/resampler worker and the non-cloneable **StreamWriter**. The stream handle is submitted to the mixer.
- The host owns the audio device/output format and decides how the final music and effects signals are converted or limited.

This separation lets the same music and sound systems be used from Bevy or another host.

## Real-time callback rules

The render path must be treated as a hard real-time boundary:

- Do not perform file access, asset lookup, audio decoding, or stream resampling in the callback.
- Do not wait for a producer, block on a mutex, or use a retry loop that can wait for an unbounded amount of time.
- Prepare engines, mixers, voice storage, and stream buffers before the callback starts.
- Render from immutable resident PCM or already-decoded stream frames.
- Keep ownership arranged so the callback does not release the final large audio-buffer allocation.

Resident clips are linearly resampled by the sound mixer to its output rate. Streaming is different: a stream must already use the mixer's exact output sample rate. Resample streamed source data on the host worker.

The current music renderer also performs composition work at loop boundaries and constructs an incoming engine when a preset transition is processed. Those operations contribute to the render callback's workload and can create allocation/cost spikes. Verdandi keeps file access and decoding out of the callback, but should not yet be represented as a universally allocation-free hard-real-time engine. Hosts with strict callback deadlines should profile the target workload, especially around loop boundaries and preset changes.

## Thread and queue ownership

### Music

Create one **MusicShared** and pass its **Arc** to the engine. The audio callback owns the mutable **Engine**; gameplay code may issue control requests through **MusicShared**. Those controls use atomic state rather than requiring game code to mutate the audio engine.

### Sound playback

A **SoundController** is the control-side handle. Create exactly one active **SoundMixer** from it, before entering the callback. The mixer is the sole consumer of the controller's command queue. **try_create_mixer** returns **AlreadyCreated** when an active mixer already exists.

The command queue has a capacity of 64. Submission does not wait: concurrent control producers can receive **ProducerBusy**, and a full queue returns **QueueFull**. The host should handle these on a control or worker thread.

The mixer voice limit is configurable from 1 to **MAX_SOUND_VOICES** (128). When full, it rejects the newest play request and increments **rejected_voices**; existing voices are not displaced. This voice-limit rejection is visible in **SoundMixerStatus**, not returned by the original queue-submission call.

### Stream playback

Each **StreamBuffer** has one producer and one active mixer consumer. The writer is deliberately not cloneable. Writes are non-blocking and may accept fewer frames than requested.

Keep both the writer and the **Arc<AudioStream>** alive until **AudioStream::is_active()** becomes false. The mixer releases its stream handle before publishing the inactive state, allowing the host to observe completion and release the remaining ring-buffer storage off the callback thread.

## Output format and timing

The music engine renders stereo f32 frames. **Engine::render_frame** returns **(left, right)**; **render_interleaved** fills a slice ordered **L, R, L, R, ...**. Supply an even number of samples to the interleaved renderer.

The sequencer uses sixteenth-note steps, 16 steps per bar, and four bars per loop (64 steps). Tempo determines the wall-clock duration. Seed changes apply at a loop boundary; normal preset changes wait for a bar boundary and crossfade over two seconds.

## Public API map

Most applications should start with:

- **MusicConfig** and either **ProceduralMusicPlugin** or **Engine**.
- **AudioController** when using Bevy, or **MusicShared** for custom-host music control.
- **ProceduralGenerator** and its built-in generators for deterministic game-audio effects and ambience.
- **AudioClip**, **SoundCategory**, and **SoundController** for resident effects.
- **StreamBuffer** and **StreamWriter** for incremental decoded audio.
- **SfzInstrument** only when using external SFZ sample libraries.

The full symbol-level reference is generated from Rust API documentation at [docs.rs](https://docs.rs/verdandi/latest/verdandi/).
