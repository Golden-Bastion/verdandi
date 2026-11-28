# Verdandi documentation

Verdandi is a reusable Rust game-audio engine for deterministic procedural music, gameplay-oriented sound generation, and host-provided sound effects. This directory is the user-facing guide to integrating and operating the library. You should not need to inspect the source tree to learn the supported workflows.

## Choose a guide

| Guide | What it covers |
|---|---|
| [Getting started](getting-started.md) | Installation, feature flags, first render, and example commands |
| [Procedural music](procedural-music.md) | Presets, seeds, musical themes, layers, intensity, transitions, and stingers |
| [Procedural sound generation](procedural-audio.md) | Retro and Realistic procedural sound sets, effects, ambience, and audition packs |
| [Sound effects and ambience](sound-effects.md) | PCM clips, one-shots, keyed loops, buses, streaming, errors, and ownership |
| [SFZ instruments](sfz-instruments.md) | Loading sample libraries, supported WAV formats, patch limitations, and layer assignment |
| [Bevy integration](bevy-integration.md) | Bevy 0.20 plugin setup, resources, gameplay control, and unified audio buses |
| [Custom audio hosts](custom-audio-hosts.md) | Using Verdandi without Bevy and mixing it into your own audio callback |
| [Architecture and real-time contract](architecture.md) | System boundaries, thread ownership, callback rules, and resource lifetimes |

## Reference material

- [Rust API reference](https://docs.rs/verdandi/latest/verdandi/) for every public module, type, and method.
- [Repository examples](../examples/) for the interactive demo, offline WAV renderer, Retro and Realistic procedural audition packs, and SFZ audition tool.
- [Project roadmap](../ROADMAP.md) for planned work and current implementation boundaries.

## Supported integration choices

- **Bevy 0.20:** use the default **bevy-plugin** feature and **ProceduralMusicPlugin**. It provides a unified **AudioController** resource for music, effects, and ambience.
- **Custom host:** disable default features and use **Engine**, **MusicShared**, **SoundController**, and **SoundMixer** directly.
- **Procedural instruments:** work without downloaded sample libraries.
- **Optional SFZ instruments:** load external SFZ patches and their supported WAV samples before audio rendering begins.

Verdandi provides reusable gameplay sound concepts without depending on a particular game's entities, event system, or world simulation. It does not provide an asset catalogue or a general-purpose file decoder for game audio. The host owns its assets and decoding pipeline. Verdandi owns composition, procedural sound generation, prepared sample playback, mixing, stream buffering, routing, and the real-time playback rules described in these guides.

When a guide and the compiled Rust API disagree, treat the API exposed by the current crate version as authoritative and report the documentation discrepancy.
