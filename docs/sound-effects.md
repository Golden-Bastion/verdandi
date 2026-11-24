# Sound effects and environmental ambience

Verdandi includes an engine-independent PCM mixer for discrete effects and persistent environmental sounds. The host owns the asset catalogue and supplies decoded PCM. Verdandi provides playback, looping, fades, category routing, bounded voice management, and an optional bounded streaming path.

## Categories and buses

A sound command uses one of two categories:

- **SoundCategory::Effects** for one-shot effects such as footsteps, impacts, or interface sounds.
- **SoundCategory::Ambience** for persistent world sounds such as rain, wind, fire, or machinery.

The unified Bevy controller exposes three independent buses: **AudioBus::Music**, **AudioBus::Effects**, and **AudioBus::Ambience**. Each bus has independent gain and mute controls. A per-sound gain is multiplied into its category output.

## Prepare a resident clip

The host decodes an asset away from the audio callback, then constructs an immutable **AudioClip**. Samples are floating-point PCM in the -1.0 to 1.0 range. Only mono and stereo are accepted. Stereo samples are interleaved left/right; mono samples are duplicated to both output channels.

~~~rust
use std::sync::Arc;
use verdandi::prelude::{
    AudioClip, AudioController, SoundCategory, SoundCommandError,
};

fn make_footstep(
    decoded_mono_samples: Vec<f32>,
) -> Result<Arc<AudioClip>, verdandi::sound::AudioClipError> {
    Ok(Arc::new(AudioClip::from_samples(
        48_000,
        1,
        decoded_mono_samples,
    )?))
}

// Keep the catalogue-owned Arc alive after submitting the playback request.
fn play_footstep(
    audio: &AudioController,
    footstep: &Arc<AudioClip>,
) -> Result<(), SoundCommandError> {
    audio.play_one_shot(footstep.clone(), SoundCategory::Effects, 0.85)
}
~~~

**AudioClip::from_shared_samples** accepts an existing **Arc<[f32]>** when the host already stores PCM in shared immutable memory. Clips must have a non-zero sample rate, at least one complete frame, one or two channels, and finite sample values.

Resident clips may use a sample rate different from the mixer output rate; the mixer linearly resamples them during playback. Retain an owning **Arc<AudioClip>** in the host's asset catalogue while a clip may be queued or playing. This keeps the final sample-buffer release away from the render callback.

## One-shots and keyed loops

~~~rust
use std::sync::Arc;
use std::time::Duration;
use verdandi::prelude::{
    AudioClip, AudioController, LoopKey, SoundCategory, SoundCommandError,
};

fn start_rain(
    audio: &AudioController,
    rain: &Arc<AudioClip>,
) -> Result<(), SoundCommandError> {
    audio.play_loop(
        LoopKey(100),
        rain.clone(),
        SoundCategory::Ambience,
        0.55,
        Duration::from_secs(2),
    )
}

fn quiet_rain(audio: &AudioController) -> Result<(), SoundCommandError> {
    audio.fade_loop(LoopKey(100), 0.25, Duration::from_secs(3))
}

fn stop_rain(audio: &AudioController) -> Result<(), SoundCommandError> {
    audio.stop_loop(LoopKey(100), Duration::from_secs(2))
}
~~~

A **LoopKey** is an application-defined stable identifier. Starting another loop with the same key replaces the existing voice at the next rendered frame; the replacement starts from the beginning of its clip. A non-zero fade-in starts the new voice at silence. Loop and master fades are linear. A later fade replaces the current fade from the gain already reached. A zero-duration stop removes the loop immediately.

Use different keys for sounds that must coexist. One-shot sounds do not need a key and end when the clip reaches its end.

## Bus controls and monitoring

With Bevy's **AudioController**:

~~~rust
use verdandi::prelude::{AudioBus, AudioController};

fn balance_buses(audio: &AudioController) {
    audio.set_bus_volume(AudioBus::Effects, 0.65);
    audio.set_bus_muted(AudioBus::Ambience, true);

    let effects_gain = audio.bus_volume(AudioBus::Effects);
    let ambience_is_muted = audio.bus_muted(AudioBus::Ambience);
}
~~~

The engine-independent **SoundController** has equivalent category-level controls through **set_category_volume**, **category_volume**, **set_category_muted**, and **category_muted**.

The mixer has a bounded voice pool. The maximum is configurable from 1 through **MAX_SOUND_VOICES** (128). The default configuration allows 32 voices; the Bevy plugin uses that default. When the voice pool is full, the newest play request is rejected and existing voices continue. This is a voice-limit rejection, not a queue-submission error. Inspect **SoundMixerStatus.active_voices** and **rejected_voices** to monitor it.

## Long audio: bounded streaming

Use streaming when an asset is too large to keep as one resident clip or when a host decoder produces audio incrementally. Verdandi does not decode the file. The host worker must decode and resample chunks to the mixer's output sample rate, then write interleaved mono/stereo frames to a fixed-capacity stream.

Create the stream before starting playback:

~~~rust
use std::sync::Arc;
use verdandi::prelude::{
    AudioController, AudioStream, SoundCategory, StreamBuffer,
    StreamBufferConfig, StreamWriter,
};

fn start_stream(
    audio: &AudioController,
) -> Result<(StreamWriter, Arc<AudioStream>), Box<dyn std::error::Error>> {
    let (writer, stream) = StreamBuffer::create(
        StreamBufferConfig::new(
            48_000, // stream / mixer sample rate
            2,      // stereo
            96_000, // capacity in frames: two seconds
            4_800,  // startup prebuffer: 100 ms
        ),
    )?;

    audio.play_stream(stream.clone(), SoundCategory::Ambience, 0.7)?;
    Ok((writer, stream))
}

~~~

Call this from startup/gameplay code, then move both returned handles into the host decoder worker along with its decoder state. The writer belongs on one host worker thread. It is not cloneable. The stream handle is submitted to Verdandi and can also be used by the host to inspect playback status.

### Write every frame without dropping partial writes

**write_interleaved** returns the number of **frames** accepted, not the number of float samples. For stereo, one frame is two floats. A partial result means the remaining samples were not accepted; a result of zero means the ring is full. Retry the unaccepted tail after the mixer consumes more frames.

~~~rust
use std::time::Duration;
use verdandi::prelude::{StreamWriteError, StreamWriter};

fn write_all_frames(
    writer: &mut StreamWriter,
    samples: &[f32],
    channels: usize,
) -> Result<(), StreamWriteError> {
    let mut sample_offset = 0;

    while sample_offset < samples.len() {
        let accepted_frames =
            writer.write_interleaved(&samples[sample_offset..])?;

        sample_offset += accepted_frames * channels;

        if accepted_frames == 0 {
            // This wait happens on the host worker, never in the audio callback.
            std::thread::sleep(Duration::from_millis(1));
        }
    }

    Ok(())
}
~~~

~~~rust
use std::sync::Arc;
use std::time::Duration;
use verdandi::prelude::{
    AudioStream, StreamWriteError, StreamWriter,
};

fn feed_decoded_chunks(
    mut writer: StreamWriter,
    stream: Arc<AudioStream>,
    decoded_chunks: impl IntoIterator<Item = Vec<f32>>,
) -> Result<(), StreamWriteError> {
    // Capture write errors so the writer can still signal EOF and retain both
    // handles until the mixer has drained/released the stream.
    let write_result = (|| {
        for chunk in decoded_chunks {
            write_all_frames(&mut writer, &chunk, 2)?;
        }
        Ok::<(), StreamWriteError>(())
    })();

    writer.finish();

    // Keep both handles alive until the mixer reports that it released the voice.
    while stream.is_active() {
        std::thread::sleep(Duration::from_millis(2));
    }

    write_result
}
~~~

The helper assumes that each decoded chunk contains complete stereo frames and finite samples. The mixer must be running concurrently so it can consume frames and make room in the ring. It signals end-of-stream after the final chunk, then keeps both handles alive on the worker until the mixer releases the voice. Verdandi drains all buffered frames before releasing the voice. If the stream reaches EOF before the startup prebuffer threshold is reached, it starts with the available data and drains it rather than waiting forever.

For a one-shot stream, keep the **StreamWriter** and **Arc<AudioStream>** alive until **stream.is_active()** becomes false. This lifetime rule ensures the ring-buffer allocation is not freed by the render callback. **is_playing()** becomes true after the startup threshold has been reached and frames begin to be consumed. **underrun_frames()** reports frames for which the stream had no data after playback began. Underflow outputs silence and playback resumes automatically when new frames arrive.

### Continuous streamed ambience

Use **play_stream_loop(key, stream, category, gain, fade_in)** for a stream that the host continuously feeds. The stream itself does not loop a finite buffer. The host decoder/producer must continue supplying frames, for example by seeking the source back to its beginning when it reaches EOF. Stop or fade the keyed stream with **stop_loop(key, fade_out)**; coordinate that with stopping the host producer.

### Capacity and validation

- Each stream is capped at **MAX_STREAM_BUFFER_FRAMES** (262,144) frames.
- The default buffer is 44.1 kHz stereo, 88,200 frames (two seconds) with a 4,410-frame (100 ms) prebuffer.
- Capacity must be at least one frame. The prebuffer may be zero but cannot exceed capacity.
- Only mono and stereo streams are supported.
- Stream sample rate must exactly match the mixer sample rate. A mismatch returns **SoundCommandError::StreamSampleRateMismatch**; do resampling on the host worker.
- A stream may have only one active mixer consumer. Attempting to play it again while queued or active returns **StreamAlreadyActive**.
- Writes after **finish()** return **StreamWriteError::Finished**. Incomplete frames and non-finite sample values are rejected.

## Command submission errors

Sound control calls do not wait for the audio thread:

| Error | Meaning | What the host should do |
|---|---|---|
| **QueueFull** | The 64-command queue has no free slot | Retry later from a control/worker thread |
| **ProducerBusy** | Another producer is submitting at the same time | Retry later; no producer is made to wait |
| **StreamSampleRateMismatch** | Stream rate differs from mixer rate | Resample on the host worker and recreate the stream |
| **StreamAlreadyActive** | Stream is already queued or playing | Wait until inactive or create a new stream |

The queue has a fixed capacity of 64 commands. Avoid tight retry loops on the audio callback or other time-sensitive threads. Voice-pool rejection is reported through the rejected counter rather than these submission errors.

For the engine-independent setup, including construction of the one-and-only mixer, see [custom audio hosts](custom-audio-hosts.md). For Bevy resource setup, see [Bevy integration](bevy-integration.md).
