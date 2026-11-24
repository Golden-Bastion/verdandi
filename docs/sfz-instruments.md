# SFZ sample instruments

Verdandi synthesizes all musical layers procedurally by default. You can optionally assign host-provided SFZ instruments to layers. Verdandi includes the SFZ parser, WAV sample loader, and sampler; you do not need a separate sampler backend.

Sample files and libraries are not bundled with the crate. Obtain and manage them separately, and check their licenses before distributing or using them in a game.

## Load an instrument

Load and decode the patch before constructing the engine or starting audio rendering:

~~~rust
use std::sync::Arc;
use verdandi::prelude::{
    Layer, MusicConfig, SfzInstrument,
};

fn configure_guitar() -> Result<MusicConfig, Box<dyn std::error::Error>> {
    let guitar = Arc::new(SfzInstrument::load(
        "/path/to/library/instrument.sfz",
    )?);

    if !guitar.unsupported_opcodes().is_empty() {
        eprintln!(
            "This patch uses unsupported SFZ features: {:?}",
            guitar.unsupported_opcodes()
        );
    }

    let mut config = MusicConfig::default();
    config.instruments.set_sfz(Layer::RhythmGuitar, guitar);
    Ok(config)
}
~~~

**SfzInstrument::load** resolves relative sample paths from the patch's directory, applying the patch's **default_path** when present.

If the SFZ mapping text and samples are stored in different locations, pass an explicit sample root:

~~~rust
let guitar = Arc::new(SfzInstrument::load_with_sample_root(
    "/app/patches/guitar.sfz",
    "/app/audio-libraries/guitar",
)?);
~~~

The mapping's **default_path**, if present, is appended beneath the explicit root. The resulting instrument owns its decoded sample data, so keep the **Arc<SfzInstrument>** in the MusicConfig/engine lifetime.

## Assign and remove instruments

**MusicConfig.instruments** is an **InstrumentConfig**. Assign a loaded instrument to a layer with **set_sfz**; remove it with **clear**. Query an assignment with **sfz**.

~~~rust
config.instruments.set_sfz(Layer::Lead, lead_instrument);
config.instruments.clear(Layer::Lead);
~~~

Any layer without a configured sample instrument uses the existing procedural voice. If a note does not match a region in its assigned SFZ patch, Verdandi also falls back to the procedural voice for that event.

## Supported SFZ subset

The parser supports the common sections **control**, **global**, **group**, and **region**, plus the following opcodes:

- Sample mapping and key ranges: **sample**, **key**, **lokey**, **hikey**, **pitch_keycenter**.
- Velocity/random selection: **lovel**, **hivel**, **lorand**, **hirand**.
- Tuning and gain: **volume**, **tune**, **transpose**, **amp_veltrack**.
- Looping and sample range: **loop_mode**, **loop_start**, **loop_end**, **offset**, **end**.
- Amplitude envelope: **ampeg_attack**, **ampeg_decay**, **ampeg_sustain**, **ampeg_release**.
- Sample path configuration: **default_path**.

This is a deliberately scoped implementation, not a claim of complete SFZ compatibility. Unsupported opcodes and section types are collected by **unsupported_opcodes()** and ignored by the parser. Inspect that list before depending on a patch whose sound relies on a feature the parser does not implement.

## Supported sample formats

The bundled sample decoder reads mono/stereo RIFF/WAVE files with:

- PCM integer samples: 8-bit, 16-bit, 24-bit, or 32-bit.
- IEEE floating-point samples: 32-bit or 64-bit.

FLAC and other compressed sample formats are not supported by this decoder. A patch may parse successfully while still failing to load if a referenced file is missing or uses an unsupported encoding.

## Lower-level use

For custom instrument workflows, the crate also exposes **parse_sfz**, **SfzDefinition**, **SfzRegion**, and **SfzSampler**. **parse_sfz** parses mapping text without loading sample files. **SfzSampler** is the lower-level voice renderer; most applications should use **SfzInstrument** through **MusicConfig.instruments** instead.

SFZ loading and WAV decoding are setup-time operations. Do not call them from an audio callback.
