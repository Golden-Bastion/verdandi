# AGENTS.md

Instructions for AI agents contributing to **Verdandi**.

## 1. Project identity and priorities

Verdandi is a reusable game-audio engine written in Rust. It provides deterministic, seed-driven procedural music; procedural sound-effect and environmental-audio generation; host-provided PCM playback and streaming; optional SFZ sample instruments; an engine-independent core; and an optional Bevy 0.20 integration.

Its priorities are:

- Reproducible musical and sound-effect output: the same seed, configuration, and render sequence must produce the same result.
- Musically coherent, expressive layers and presets rather than merely random note generation.
- Useful, game-oriented sound capabilities such as weapons, impacts, clashes, material response, character actions, destruction, and environmental ambience.
- A reliable real-time audio path suitable for integration into games.
- A useful engine-independent core, with Bevy integration kept optional.
- Procedural instruments by default, with user-provided sample libraries as an optional enhancement.
- A small, maintainable crate with clear public APIs.

Game-audio concepts are first-class library concerns, not out of scope. Build reusable generators and APIs for common gameplay sounds when they belong in an audio engine. Keep Verdandi independent of any particular game's entities, event identifiers, combat rules, world simulation, asset catalogue, or application architecture.

## 2. Documentation authority

Before changing code, inspect the relevant source, tests, examples, and documentation.

Use repository documentation according to its responsibility:

- `README.md`: concise project overview, capabilities, dependency/quick-start example, links to guides, examples, and current boundaries.
- `docs/`: the user-facing integration manual. Explain how to install, configure, and use each supported subsystem with complete workflows, examples, ownership rules, error handling, and real-time constraints. Organize pages by user task rather than source-file layout.
- Rust API documentation and source comments: symbol-level contracts, parameter meanings, invariants, and precise method behaviour. Public types and methods should have useful rustdoc comments; the guides do not replace API reference documentation.
- `CHANGELOG.md`: record of completed changes, following the existing format when the file is present.
- Tests and fixtures: executable expectations for deterministic behaviour, parsing, synthesis, and integration.

Documentation should let an application developer integrate Verdandi without reading the implementation. Keep examples aligned with the current public API, explain who owns each resource and on which thread work occurs, and state limitations explicitly. Avoid duplicating long guides in the README; link to the responsible page in `docs/`.

Do not assume an old issue, conversation, or plan overrides the current repository state. When documentation conflicts with implementation or tests, investigate the discrepancy rather than silently choosing one.

## 3. Development workflow

For every development task:

1. Inspect the latest repository state, relevant documentation, tests, examples, and related issues.
2. Understand the current implementation and its performance and determinism constraints.
3. State a focused implementation plan and identify any meaningful trade-offs.
4. Implement the requested change without unnecessarily changing unrelated systems.
5. Add or update automated tests for the behaviour being changed.
6. Review all modified files for correctness, consistency, and unintended effects.
7. Update relevant documentation and `CHANGELOG.md` when warranted.
8. Prepare a focused branch and pull request.

Prefer small, independently understandable changes and commits. Preserve existing functionality unless the requested change intentionally modifies it.

If a requirement would change musical direction, public API semantics, compatibility guarantees, or architectural boundaries and the intended choice is unclear, ask the project owner rather than inventing a decision.

## 4. Git and pull request workflow

- Work on dedicated feature or fix branches.
- Prefer pull requests over direct changes to `main`.
- Keep commits focused and descriptive.
- Avoid unrelated formatting changes and opportunistic refactoring.
- Explain the problem, implementation, and relevant design decisions in pull request descriptions.
- Link related issues where appropriate.
- Do not merge pull requests manually.

The project owner has local automation that monitors pull request checks, merges a PR when its checks pass, and reports failures. Do not wait for CI, repeatedly poll checks, or duplicate that automation. Once the implementation and PR are ready, report the PR and hand control back to the owner.

## 5. Build and test responsibilities

Use the repository's CI workflows as the source of truth for builds and automated validation.

- Do not attempt local builds, compilation, or test execution as part of the agent workflow.
- Add or update appropriate automated tests with implementation changes.
- Review test code and coverage for consistency with the implementation.
- Do not weaken or rewrite valid tests merely to make a failing implementation pass. When a test fails, determine whether the defect is in production code, the test, or an outdated contract; change the test only when its expectation is demonstrably incorrect.
- Do not claim that validation passed unless CI or another authoritative result confirms it.

## 6. Architecture and real-time audio constraints

Respect the separation between the composition engine, synthesis/rendering, and host integration.

- Keep the core usable without Bevy or an audio device.
- Keep Bevy-specific functionality behind the existing optional feature and avoid making it a mandatory dependency for core use.
- Keep file access, sample loading, and audio decoding out of the real-time render callback.
- Avoid allocation, blocking operations, and unbounded work on the audio hot path.
- Preserve safe and predictable communication between control code and audio-thread state.
- Give mutable state a clear owner; avoid redundant or unsynchronised state.
- Prefer explicit data flow and small abstractions over premature generalisation.
- Do not add dependencies, frameworks, or new architectural layers without a concrete need.

Performance-sensitive changes must be considered in terms of worst-case callback behaviour, not only average performance.

## 7. Determinism and generated audio behaviour

Determinism is a core contract, not an incidental implementation detail.

- Use the project's seeded random-number facilities and stable seed derivation conventions.
- Do not introduce unseeded randomness into composition or rendering paths where it affects reproducibility.
- Keep independent layers or events isolated from unrelated random-stream consumption where the existing design requires it.
- Preserve repeatability for identical seeds, presets, pace, overrides, and configuration.
- Make intentional changes to generated musical output explicit and cover them with appropriate tests.
- Preserve musical coherence, phrase structure, and preset identity when extending music generators.
- For sound effects, use stable seeded variation and meaningful sound-design controls; avoid coupling generation to unrelated event or entity state.
- Respect loop/bar boundaries and documented transition behaviour unless the requested change intentionally revises them.

Do not silently alter existing preset character or deterministic output while making an unrelated fix.

## 8. Instruments, samples, and resource handling

- Procedural-only operation must remain available without external sample libraries.
- Treat SFZ and WAV support as explicitly scoped implementations; do not imply unsupported opcodes or codecs are supported.
- Report unsupported SFZ features clearly and preserve the existing fallback behaviour where appropriate.
- Resolve sample paths predictably and avoid hidden dependence on machine-specific directories.
- Keep sample loading and decoding outside the real-time audio path.
- Do not commit large third-party sample libraries or assets without explicit approval and clear licensing.
- Do not introduce assets or fixtures with unclear provenance or incompatible licenses.

## 9. Public API, compatibility, and versioning

Verdandi is a reusable library, so public API changes affect downstream users.

- Prefer additive, well-documented API changes when practical.
- Avoid breaking public interfaces without a clear reason.
- When a breaking change is warranted, explain migration implications and update examples and documentation.
- Keep feature flags and optional dependencies coherent.
- Follow Semantic Versioning and the repository's existing pre-release/versioning policy.
- Update the package version only when the scope warrants it; do not bump it mechanically for every commit.
- Record completed user-visible or developer-relevant changes in `CHANGELOG.md` according to its conventions.

## 10. Collaboration and final reporting

The project owner is the final authority on product direction and public API decisions.

- Follow explicit decisions already made.
- Explain meaningful trade-offs when proposing a different approach.
- Ask when requirements are genuinely ambiguous or require an unapproved design decision.
- Be transparent about limitations, incomplete work, and unresolved issues.
- Do not claim a feature is complete when only part of its intended behaviour exists.

When a task is complete, report what changed, important decisions, relevant commits and PR, documentation updates, and remaining work. Distinguish completed implementation from future work.


## Acoustic realism milestone

The Realistic procedural set is being advanced from lightweight texture overlays toward a reusable layered acoustic synthesis system. Keep Retro algorithms and seeded output unchanged.

Planned delivery is grouped into cohesive pull requests:

1. **Shared acoustic synthesis core (completed in merged PR #42):** reusable filters, transient envelopes, and damped modal resonators integrated into the Realistic family texture layer with deterministic variation, bounded state, and regression coverage.
2. **Material and interaction synthesis (implemented in PR #43):** richer contact excitation, friction, body resonance, secondary-contact excitation, and decay for representative event families. Prefer shared primitives and coherent event models over per-generator one-off noise additions.
3. **Environmental acoustics (current implementation):** reusable distance/occlusion and early-reflection/decay processing suitable for open and enclosed spaces, with explicit configuration and bounded processing cost.

Every stage must preserve Retro output, keep rendering allocation-free, maintain block-size determinism, include automated signal-level tests, and provide updated WAV audition packs/documentation. Tests establish invariants, not perceptual quality; do not claim realism acceptance without listening review. Recorded audio may later be supported through the existing host-provided asset path where procedural synthesis reaches its limits.
