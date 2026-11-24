# Custom audio hosts

Verdandi's core does not require Bevy or an audio device. A host supplies the device/output layer and repeatedly asks Verdandi to render stereo floating-point frames. The same approach works with an application-specific audio callback or another audio framework.

## Music-only renderer

Create **MusicConfig**, **MusicShared**, and **Engine** before starting the audio callback:

~~~rust
use std::sync::Arc;
use verdandi::prelude::{Engine, MusicConfig, MusicShared};

fn render_one_block() {
    let config = MusicConfig::default();
    let shared = Arc::new(MusicShared::new(&config));
    let mut engine = Engine::new(shared.clone(), &config);

    // In the host's callback, fill an even-length f32 slice.
    // Samples are interleaved stereo: L, R, L, R, ...
    let mut output = [0.0_f32; 128 * 2];
    engine.render_interleaved(&mut output);

    // Forward output to the host's device/backend.
}
~~~

The callback owns the mutable **Engine**. Game/control code may retain a clone of the **Arc<MusicShared>** and request changes without mutably accessing the engine. For example, use **request_seed**, **request_preset**, **request_stinger**, **set_layer_enabled**, and the intensity/transport/gain controls on **MusicShared**.

## Mix procedural music with effects

Use one sound controller and create its one mixer before starting the callback. Keep the control handle on the game side and move the mixer into the callback:

~~~rust
use std::sync::Arc;
use verdandi::prelude::{
    Engine, MusicConfig, MusicShared, SoundController, SoundMixer,
    SoundMixerConfig,
};

struct AudioPath {
    music: Engine,
    sounds: SoundMixer,
    controls: SoundController,
}

fn build_audio_path() -> Result<AudioPath, Box<dyn std::error::Error>> {
    let config = MusicConfig::default();
    let music_shared = Arc::new(MusicShared::new(&config));
    let music = Engine::new(music_shared, &config);

    let controls = SoundController::new(SoundMixerConfig::new(
        config.sample_rate,
        32,
    ))?;
    let sounds = controls.create_mixer();

    // Keep controls on the game side; move music and sounds into the callback.
    Ok(AudioPath {
        music,
        sounds,
        controls,
    })
}
~~~

For each output frame, render music and effects and combine them:

~~~rust
use verdandi::prelude::{Engine, SoundMixer};

fn render_stereo_block(
    music: &mut Engine,
    sounds: &mut SoundMixer,
    output: &mut [f32],
) {
    // The host must provide an even number of samples.
    for frame in output.chunks_exact_mut(2) {
        let (music_left, music_right) = music.render_frame();
        let (sound_left, sound_right) = sounds.render_frame();

        frame[0] = (music_left + sound_left).clamp(-1.0, 1.0);
        frame[1] = (music_right + sound_right).clamp(-1.0, 1.0);
    }
}
~~~

The host remains responsible for the output device's required sample format, channel layout, and any final limiter or conversion. The example above uses stereo f32 and hard clipping as a minimal combination step, not as a recommended mastering stage.

## Submit sounds from the game side

Prepare clips on a host loading/decoding worker, retain their owning **Arc<AudioClip>** references in the asset catalogue, and submit commands through the cloned controller:

~~~rust
use std::sync::Arc;
use verdandi::prelude::{AudioClip, SoundCategory, SoundController};

fn play_effect(
    audio: &SoundController,
    pcm: Vec<f32>,
) -> Result<Arc<AudioClip>, Box<dyn std::error::Error>> {
    let clip = Arc::new(AudioClip::from_samples(48_000, 1, pcm)?);
    audio.play_one_shot(clip.clone(), SoundCategory::Effects, 0.9)?;
    Ok(clip) // store this in the host's asset catalogue
}
~~~

Sound submission is non-blocking. It can return **QueueFull** or **ProducerBusy**; handle these on the control/worker side and retry when appropriate. The host must not wait, decode, allocate, or perform file access in its real-time callback. Verdandi also performs arrangement generation at loop boundaries and prepares a new engine when a preset transition is processed. Those operations are part of the current renderer's callback workload; budget and profile callback deadlines around loop boundaries and preset changes rather than assuming the entire engine is strictly allocation-free.

## Streaming with a host decoder

The streaming workflow is shared with the Bevy integration:

1. Create a **StreamBuffer** and retain its **StreamWriter** and **Arc<AudioStream>**.
2. Submit the stream to the controller with **play_stream** or **play_stream_loop**.
3. On one host worker, decode and resample chunks to the mixer's sample rate.
4. Write chunks with **write_interleaved**, advancing by the accepted frame count. Retry zero/partial writes when the ring has room.
5. Call **finish()** at end-of-stream for a one-shot source.
6. Retain both handles until **AudioStream::is_active()** becomes false.

The stream sample rate must match the mixer's sample rate exactly; the stream mixer does not resample. A finite stream does not repeat itself. For looping streamed ambience, the host producer must loop its source and keep supplying frames until the application stops it.

See [sound effects and ambience](sound-effects.md) for the complete write/retry example, buffer limits, prebuffer semantics, underflow handling, and error reference.

## Audio callback checklist

- Create **Engine**, **SoundController**, and the one **SoundMixer** before starting the device callback.
- Give the callback exclusive mutable ownership of **Engine** and **SoundMixer**.
- Do not create more than one active mixer from a **SoundController**.
- Use even-length stereo output buffers for **render_interleaved**.
- Keep clip and stream storage alive according to the ownership rules.
- Do not decode, read files, block, or perform unbounded work in the callback.
- Match the music engine, sound mixer, and output stream sample rates.
