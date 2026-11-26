# Verdandi Audio Quality Roadmap

This document is the durable plan for improving Verdandi's game-audio capabilities and audio quality. It tracks instrument realism, procedural sound families, and playback quality. Update the milestone checkboxes and decisions here as work is completed so future development sessions can resume from the repository rather than relying on memory.

## Goal and design direction

Prioritize the best musical result, not a sample-free implementation. Verdandi combines user-supplied recorded samples and procedural synthesis where each is useful. The SFZ parser and sampler are first-party code in this repository; audio libraries remain external to the crate and are never fetched automatically.

Keep Verdandi explicitly game-audio-oriented while preserving application independence. The composer decides what notes to play and when; instrument backends decide how those notes sound; procedural sound generators provide reusable effects for common gameplay events. Game concepts such as weapons, clashes, material impacts, character actions, and destruction belong in the library when they can be expressed without depending on a particular game's entity model, event bus, combat rules, or world simulation. Preserve existing sounds unless a replacement or addition is demonstrably better.

## Current baseline

- [x] Seeded, layered procedural composition and four named presets: Ethereal, Warbound, Orchestral, and Contemplative.
- [x] Runtime preset/pace transitions.
- [x] Built-in deterministic musical stingers.
- [x] Improved procedural plucked-string and bowed-string voices.
- [x] First-party SFZ parsing, WAV sample loading, seeded region selection, polyphonic playback, and layer routing are implemented as an initial compatibility slice.
- [x] The procedural backend remains the default and fallback when a sampled instrument is absent or has no matching region.
- [ ] Real-instrument playback still needs broader SFZ compatibility, audition validation, and tuning against the real libraries.

The current procedural voices remain the baseline and fallback while the sample-based path is evaluated. Contemplative's established sound and the distorted Guitar voice should not be changed without an audible reason.

## Milestones

### M1 — Inspect sample mappings and audition candidates

Status: **In progress**

The user has auditioned the available sample libraries except MerryOrks and FreePats Electric Guitar FSBS Clean. The existing Python audition helper reports a generic error for those mappings; that does not establish that the libraries are invalid. It is a limitation of the helper, which is not a complete SFZ player.

The small SFZ mapping text files under `samples/` are committed as parser fixtures. Their referenced audio remains on the user's device and is not part of this repository.

Tasks:
- [x] Commit the small SFZ mappings as parser fixtures.
- [x] Add a parser test that loads every committed mapping and reports unsupported features.
- [ ] Identify the unsupported opcodes and sample formats needed by MerryOrks and Electric Guitar FSBS.
- [ ] Render representative notes from the remaining mappings using the native audition example once the required audio formats/features are supported.
- [ ] Record each library's useful note ranges, velocity zones, articulations, and playback limitations.

**Exit criteria:** the feature requirements of the sample libraries are documented, with parser compatibility and actual sample playback clearly distinguished.

### M2 — Implement the first-party SFZ engine

Status: **In progress**

**Decision:** the parser, region model, sample loading, voice management, and playback engine live inside Verdandi. Do not add a third-party sampler dependency or a separate `verdandi-sfz` crate.

Implemented:
- [x] Public SFZ definition/region model and parser with line-aware errors and unsupported-opcode diagnostics.
- [x] Global/group/region inheritance, quoted sample paths, key/velocity zones, tuning, volume, amplitude envelope, random-region ranges, sample/loop bounds, and one-shot/sustain/continuous loop modes.
- [x] RIFF/WAVE PCM (8/16/24/32-bit) and IEEE float (32/64-bit) loading, mono/stereo handling, sample caching, and up-front sample-range validation.
- [x] Bounded polyphonic sample playback with deterministic random-region selection and no file access or decoding in the render callback.
- [x] Optional instrument mapping by musical layer, including procedural fallback for unmatched notes.
- [x] Offline WAV audition example and parser/voice regression fixtures.
- [x] Explicit sample-root override so a committed SFZ mapping can reference samples in a separate local library directory.

Known gaps:
- [ ] FLAC decoding is not implemented yet; the FSBS electric guitar mapping parses but references FLAC samples that cannot currently be loaded.
- [ ] Expand opcode compatibility and improve release/loop semantics against representative instruments.
- [ ] Validate realtime safety, polyphony/voice-stealing behavior, and deterministic results with rendered output.
- [ ] Add or refine compatibility fixtures when previously unsupported mappings are brought into playback.

**Exit criteria:** the engine can load and audibly render selected real libraries, its limits are documented, and both the parser and voice renderer have automated coverage.

### M3 — User-supplied samples and procedural fallback

Status: **Decided; validation in progress**

Verdandi never bundles sample audio, downloads libraries, or manages third-party sample distribution. Users obtain their own libraries and explicitly load a patch or supply an external sample root. The committed SFZ files under `samples/` are small text fixtures only.

Tasks:
- [x] Keep sample audio out of the repository and published crate.
- [x] Allow an explicit sample root when the SFZ mapping text is stored separately from its referenced samples.
- [x] Expose sample instruments through `MusicConfig` and the same engine-agnostic and Bevy integration paths.
- [x] Preserve procedural playback as the default and use it when a configured SFZ instrument does not match a note.
- [ ] Document supported formats, opcode diagnostics, memory/loading costs, and setup with both a patch-local library and a separate sample root.

Licensing review for recordings later used in a downstream game such as Ashen Chronicle is a separate concern; Verdandi itself does not redistribute those recordings.

**Exit criteria:** users can opt into local sample libraries without changing the default dependency or asset footprint.

### M4 — Implement and audition acoustic guitar

Status: **In progress — native playback path exists; listening acceptance pending**

Implement the first sampled instrument as a small, isolated vertical slice. Do not replace the existing Pluck or distorted Guitar voice at the outset.

Tasks:
- [ ] Map available recordings to pitch/velocity zones and choose sensible fallback behavior for uncovered ranges.
- [x] Implement basic note-on/tail handling, gain, interpolation, and deterministic region selection.
- [ ] Validate note-off/tail and sustain-loop behavior by listening to actual library samples.
- [ ] Produce isolated WAV renders across low, middle, and high registers and several velocities.
- [ ] Compare sample playback against the existing procedural Pluck at matched pitch and loudness.
- [ ] Listen for unnatural repetition, pitch-shifting artifacts, clicks, abrupt tails, noise, and inconsistent loudness.
- [ ] Tune the mapping and playback behavior before integrating it into a musical layer.

**Exit criteria:** the sampled voice is audibly preferable for its intended acoustic-guitar role, passes deterministic/bounded-output and lifecycle tests, and can be auditioned independently.

### M5 — Implement and audition violin / bowed strings

Status: **Not started**

Build on the selected backend and asset pipeline. Begin with useful, well-supported articulations rather than promising a complete orchestral library immediately.

Tasks:
- [ ] Evaluate recorded violin/ensemble material for sustain quality, bow changes, release, pitch range, and velocity layers.
- [ ] Implement short and sustained articulations where the recordings support them.
- [ ] Implement smooth sustain looping and release tails where appropriate.
- [ ] Avoid artificial repetition through deterministic round-robin selection when the source offers multiple takes.
- [ ] Produce isolated WAV renders and compare against the current Strings and ClassicalStrings voices.
- [ ] Document gaps in the source material, including unsupported articulations or ranges.

**Exit criteria:** the sampled bowed-string voice sounds better for its intended role and has documented articulation/range limits. Do not imply true legato or other articulations unless the assets and implementation support them.

### M6 — Integrate accepted voices into arrangements

Status: **In progress — routing and procedural fallback exist; instrument acceptance pending**

Only integrate a sampled voice after its isolated audition succeeds.

Tasks:
- [ ] Choose the preset layers and musical roles that benefit from sampled acoustic guitar and bowed strings.
- [ ] Compare old and new renders at matched seed, tempo, intensity, and output level.
- [ ] Preserve Warbound's distorted guitar character and Contemplative's established character unless listening demonstrates a clear improvement.
- [ ] Verify that layering does not cause clipping, masking, excessive CPU use, or undesirable stereo placement.
- [x] Keep existing procedural voices available as fallbacks for missing instruments or uncovered notes.

**Exit criteria:** each integration has an audible rationale and does not regress other presets or event behavior.

### M7 — Validate, document, and release

Status: **In progress**

- [x] Add tests for mapping parsing, deterministic sample selection, and finite rendered output.
- [ ] Expand tests for sample boundaries, note-off/release, voice exhaustion, and broader SFZ fixtures.
- [ ] Confirm realtime rendering does not perform file access, decoding, locking, or avoidable allocation.
- [ ] Check preset transitions, stingers, pause/resume, and concurrent notes with sampled voices.
- [ ] Verify offline WAV rendering works with both procedural-only and sample-enabled configurations.
- [x] Document user-supplied SFZ setup, supported formats, opcode limitations, and procedural fallback.
- [ ] Document memory/resource usage and validate the rendered output against the real libraries.
- [ ] Update examples so individual instruments and musical layers can be rendered for listening tests.
- [ ] Update the crate version according to SemVer if public API, dependency, or distribution changes warrant it.
- [ ] Open focused PRs and record validation results; do not mark tests as passed unless they were actually run.

**Exit criteria:** the feature is documented, tested to the extent supported by the environment, audibly reviewed, and distributable under the verified licenses.

## Quality requirements for every milestone

- **Determinism:** the same seed and same event sequence produce the same musical decisions and deterministic sample choices. Document any platform-level differences in floating-point rendering if relevant.
- **Realtime safety:** prepare assets and allocate playback resources before rendering; avoid disk access, decoding, locks, and avoidable allocations in the audio callback.
- **Robust output:** prevent invalid sample data from propagating NaNs or infinities, and maintain the engine's output bounds.
- **Compatibility:** preserve the current composer, preset API, transitions, stingers, and procedural voices unless a change is deliberate and documented.
- **Audition before acceptance:** unit tests cannot establish that an instrument sounds realistic. Keep isolated WAV renders and listening notes as part of the review.
- **Licensing:** track provenance and redistribution permissions for every included sample set.

## Decision log

- **Composition and rendering are separate:** the seeded composition engine determines musical events; each layer can use a procedural voice or a sampled instrument.
- **Both backends belong to Verdandi:** the SFZ parser, sample loader, polyphonic sampler, and procedural synthesizer live in the same crate. No separate `verdandi-sfz` repository or external sampler runtime is planned.
- **User-managed sample libraries:** Verdandi ships only the small SFZ mapping fixtures committed under `samples/`, not their audio data. No automatic downloads or bundled sample packs.
- **Explicit sample roots:** loading may use the patch's directory and `default_path`, or an explicit library root when the mapping text is stored separately from its audio files.
- **Initial format limitation:** the native player currently decodes WAV only. The committed FreePats FSBS Clean Electric Guitar mapping references FLAC, so successful parsing is not evidence that this instrument is playable yet.
- **Compatibility strategy:** support and test the subset required by real libraries, report unsupported opcodes, and expand incrementally rather than claiming full SFZ compatibility.
- **First instrument order:** validate the Spanish Classical Guitar, then electric guitar and bowed-string mappings according to their feature and audio-format requirements.
- **Downstream licensing:** licensing review for audio use in Ashen Chronicle or other games is outside Verdandi's sample-engine implementation milestone because Verdandi does not distribute sample recordings.
- **Versioning:** the new public SFZ and instrument configuration API is on the 0.6.0 development line.


## Runtime transport and gain controls

- [x] Expose engine-independent pause/resume controls that freeze musical transport without discarding synthesis state.
- [x] Expose bounded master gain with immediate setting and linear fades.
- [x] Define overlapping fade requests as replacement from the current gain.
- [x] Apply master gain after music, stinger, and preset-transition mixing.
- [x] Cover pause position preservation, gain clamping, fade progression, and replacement semantics with regression tests.


## Environmental ambience and sound effects

- [x] Add host-owned immutable mono/stereo PCM clips with validation and sample-rate conversion.
- [x] Add bounded one-shot and persistent keyed-loop playback alongside procedural music.
- [x] Define same-key replacement, per-loop fades, fade-out stopping, and no-stale-loop semantics.
- [x] Add independent effects and ambience gain/mute controls; expose music/effects/ambience buses through the Bevy controller.
- [x] Define a bounded 64-command queue, non-blocking concurrent producer behavior, and a configurable voice limit up to 128.
- [x] Reject newest play requests when the voice pool is full; retain existing voices and publish active/rejected counters.
- [x] Add regression coverage for clip validation, resampling, ordering, replacement, fades, category isolation, voice limits, and queue overflow.
- [x] Add fixed-capacity SPSC decoded-PCM stream buffers with configurable startup prebuffering and non-blocking partial writes.
- [x] Define underflow as silence with automatic resume, and drain queued frames after explicit end-of-stream.
- [x] Support keyed streamed ambience with existing fade/stop controls; keep decoding and resampling on the host worker at the mixer output rate.

**Current boundary:** the host owns its asset catalogue and performs file access, decoding, and stream resampling on worker threads. It supplies immutable resident clips or writes decoded output-rate frames into bounded stream buffers. Verdandi owns playback, prebuffer thresholds, underflow handling, fades, routing, and voice limits; it never accesses files or decodes in the render callback.


## Procedural sound generation

- [x] Add a dedicated engine-independent procedural sound module with caller-buffer stereo rendering.
- [x] Add deterministic wind and rain ambience generators.
- [x] Add deterministic one-shot footstep and generic impact generators.
- [x] Add surface-specific footstep profiles for dirt, grass, gravel, wood, stone, metal, shallow water, and snow while preserving the generic footstep constructor.
- [x] Add weapon-contact generation for strikes, blade clashes, parries, and blocks with material, force, and intensity controls.
- [x] Add a shield material profile and projectile-impact kind to the weapon-contact generator.
- [x] Validate sample rates, normalized parameters, and stereo buffer shape.
- [x] Cover deterministic block rendering, finite-effect completion, output bounds, and parameter response with regression tests.
- [x] Document finite-clip and continuous-stream integration.
- [ ] Audition generated sounds at representative sample rates and tune their acoustic character.
- [x] Add material-specific breakage with deterministic, fixed-capacity fragment events and size/force/intensity controls.
- [x] Add deterministic character movement sounds for jumps, landings, and falls with force and body-weight controls.
- [x] Add deterministic weapon draw, sheathe, and reload handling sounds with material, weight, and speed controls.
- [x] Add deterministic equipment movement sounds for cloth rustle, carried-gear jostle, and armor rattle with weight and activity controls.
- [ ] Expand the generator toolkit with vocal pain/exertion, breathing and death sounds, blood-splatter effects, bone breaks, and further object destruction (tracked in [issue #28](https://github.com/Golden-Bastion/verdandi/issues/28)).

**Current boundary:** procedural generators render PCM outside the playback callback. Finite sounds can be wrapped in `AudioClip`; continuous output can be written to the existing bounded `StreamBuffer`. The initial generators are stylized and centered in stereo; they do not claim physically accurate simulation or advanced spatialization.


## Game-audio direction

Verdandi is a reusable game-audio engine, not a generic audio toolkit that avoids gameplay concepts. Its public API may expose reusable concepts such as weapon impacts and clashes, parries, armor and shield hits, material response, footsteps, character exertion and death sounds, blood-splatter effects, breakage, and debris. These are library-level sound-design concepts; a consuming game remains responsible for deciding when they occur and how they relate to its entities, combat rules, world, and content catalogue.

Prioritize cohesive generator families with shared synthesis primitives and meaningful controls over isolated one-off effects. The first combat-contact family now covers strikes, blade clashes, parries, and blocks with stylized material profiles. Keep deterministic variation, bounded caller-buffer rendering, and the existing separation between generation and playback. See [issue #28](https://github.com/Golden-Bastion/verdandi/issues/28) for the remaining expansion scope.
