//! Render the Realistic procedural sound family set to a local WAV audition pack.
//!
//! Run with:
//! cargo run --release --no-default-features --example procedural_realistic_audition
//! cargo run --release --no-default-features --example procedural_realistic_audition -- /path/to/realistic-output 1234

use std::env;
use std::error::Error;
use std::fs::{self, File};
use std::io::{BufWriter, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};

use verdandi::procedural::realistic::{
    BreakageConfig, BreakageGenerator, BreakageMaterial, CharacterActionConfig,
    CharacterActionGenerator, CharacterActionKind, CharacterVocalizationConfig,
    CharacterVocalizationGenerator, CharacterVocalizationKind, DamageEffectConfig,
    DamageEffectGenerator, DamageEffectKind, EquipmentMovementConfig, EquipmentMovementGenerator,
    EquipmentMovementKind, FootstepConfig, FootstepGenerator, FootstepSurface, ImpactConfig,
    ImpactGenerator, ImpactMaterial, MechanicalActionConfig, MechanicalActionGenerator,
    MechanicalActionKind, ObjectInteractionConfig, ObjectInteractionGenerator,
    ObjectInteractionKind, ProceduralGenerator, RainConfig, RainGenerator, ToolActionConfig,
    ToolActionGenerator, ToolActionKind, ToolActionMaterial, WeaponHandlingConfig,
    WeaponHandlingGenerator, WeaponHandlingKind, WeaponImpactConfig, WeaponImpactGenerator,
    WeaponImpactKind, WindConfig, WindGenerator,
};

const SAMPLE_RATE: u32 = 48_000;
const CHANNELS: u16 = 2;
const BITS_PER_SAMPLE: u16 = 16;
const CHUNK_FRAMES: usize = 256;
const AMBIENCE_SECONDS: usize = 5;
const DEFAULT_OUTPUT: &str = "target/realistic-audition-pack";
const DEFAULT_SEED: u64 = 0x5eed;

fn main() {
    if let Err(error) = run() {
        eprintln!("{error}");
        eprintln!(
            "Usage: procedural_realistic_audition [output-directory] [seed]\n\
             Defaults: target/realistic-audition-pack and seed 0x5eed"
        );
        std::process::exit(2);
    }
}

fn run() -> Result<(), Box<dyn Error>> {
    let mut args = env::args().skip(1);
    let output_dir = PathBuf::from(args.next().unwrap_or_else(|| DEFAULT_OUTPUT.to_owned()));
    let base_seed = match args.next() {
        Some(value) => value
            .parse::<u64>()
            .map_err(|_| format!("invalid seed {value:?}: expected an unsigned 64-bit integer"))?,
        None => DEFAULT_SEED,
    };
    if args.next().is_some() {
        return Err("too many arguments".into());
    }

    fs::create_dir_all(&output_dir)?;
    let mut pack = PackWriter::new(&output_dir);
    let mut sample_index = 0_u64;

    // Surface-specific footsteps.
    let config = FootstepConfig {
        sample_rate: SAMPLE_RATE,
        ..FootstepConfig::default()
    };
    for (surface, name) in [
        (FootstepSurface::Dirt, "dirt"),
        (FootstepSurface::Grass, "grass"),
        (FootstepSurface::Gravel, "gravel"),
        (FootstepSurface::Wood, "wood"),
        (FootstepSurface::Stone, "stone"),
        (FootstepSurface::Metal, "metal"),
        (FootstepSurface::ShallowWater, "shallow-water"),
        (FootstepSurface::Snow, "snow"),
    ] {
        let seed = next_seed(base_seed, &mut sample_index);
        let mut generator = FootstepGenerator::new_for_surface(config, surface, seed)?;
        pack.write_sample(
            "footsteps",
            name,
            &format!("{surface:?}"),
            seed,
            &mut generator,
            None,
        )?;
    }

    // Generic impact baseline.
    let config = ImpactConfig {
        sample_rate: SAMPLE_RATE,
        ..ImpactConfig::default()
    };
    let seed = next_seed(base_seed, &mut sample_index);
    let mut generator = ImpactGenerator::new(config, seed)?;
    pack.write_sample(
        "impacts",
        "generic",
        "generic impact",
        seed,
        &mut generator,
        None,
    )?;

    // Weapon-contact kinds use representative target materials.
    let config = WeaponImpactConfig {
        sample_rate: SAMPLE_RATE,
        ..WeaponImpactConfig::default()
    };
    for (kind, material, name) in [
        (
            WeaponImpactKind::Strike,
            ImpactMaterial::Metal,
            "strike-metal",
        ),
        (
            WeaponImpactKind::BladeClash,
            ImpactMaterial::Metal,
            "blade-clash-metal",
        ),
        (
            WeaponImpactKind::Parry,
            ImpactMaterial::Metal,
            "parry-metal",
        ),
        (
            WeaponImpactKind::Block,
            ImpactMaterial::Shield,
            "block-shield",
        ),
        (
            WeaponImpactKind::ProjectileImpact,
            ImpactMaterial::Stone,
            "projectile-impact-stone",
        ),
        (
            WeaponImpactKind::ShieldBash,
            ImpactMaterial::Shield,
            "shield-bash",
        ),
        (
            WeaponImpactKind::ArmorStrike,
            ImpactMaterial::Armor,
            "armor-strike",
        ),
        (
            WeaponImpactKind::ProjectileDeflection,
            ImpactMaterial::Metal,
            "projectile-deflection",
        ),
    ] {
        let seed = next_seed(base_seed, &mut sample_index);
        let mut sample_config = config;
        sample_config.kind = kind;
        sample_config.material = material;
        let mut generator = WeaponImpactGenerator::new(sample_config, seed)?;
        pack.write_sample(
            "weapon-impacts",
            name,
            &format!("{kind:?} / {material:?}"),
            seed,
            &mut generator,
            None,
        )?;
    }

    // Breakage material profiles.
    let config = BreakageConfig {
        sample_rate: SAMPLE_RATE,
        ..BreakageConfig::default()
    };
    for (material, name) in [
        (BreakageMaterial::Wood, "wood"),
        (BreakageMaterial::Glass, "glass"),
        (BreakageMaterial::Stone, "stone"),
        (BreakageMaterial::Ceramic, "ceramic"),
        (BreakageMaterial::Metal, "metal"),
    ] {
        let seed = next_seed(base_seed, &mut sample_index);
        let mut sample_config = config;
        sample_config.material = material;
        let mut generator = BreakageGenerator::new(sample_config, seed)?;
        pack.write_sample(
            "breakage",
            name,
            &format!("{material:?}"),
            seed,
            &mut generator,
            None,
        )?;
    }

    // Non-vocal damage profiles.
    let config = DamageEffectConfig {
        sample_rate: SAMPLE_RATE,
        ..DamageEffectConfig::default()
    };
    for (kind, name) in [
        (DamageEffectKind::BloodSplatter, "blood-splatter"),
        (DamageEffectKind::BoneBreak, "bone-break"),
        (DamageEffectKind::BluntTrauma, "blunt-trauma"),
    ] {
        let seed = next_seed(base_seed, &mut sample_index);
        let mut sample_config = config;
        sample_config.kind = kind;
        let mut generator = DamageEffectGenerator::new(sample_config, seed)?;
        pack.write_sample(
            "damage",
            name,
            &format!("{kind:?}"),
            seed,
            &mut generator,
            None,
        )?;
    }

    // Nonverbal synthetic vocalizations.
    let config = CharacterVocalizationConfig {
        sample_rate: SAMPLE_RATE,
        ..CharacterVocalizationConfig::default()
    };
    for (kind, name) in [
        (CharacterVocalizationKind::Inhale, "inhale"),
        (CharacterVocalizationKind::Exhale, "exhale"),
        (CharacterVocalizationKind::Exertion, "exertion"),
        (CharacterVocalizationKind::Pain, "pain"),
        (CharacterVocalizationKind::DeathGroan, "death-groan"),
    ] {
        let seed = next_seed(base_seed, &mut sample_index);
        let mut sample_config = config;
        sample_config.kind = kind;
        let mut generator = CharacterVocalizationGenerator::new(sample_config, seed)?;
        pack.write_sample(
            "vocalizations",
            name,
            &format!("{kind:?} (nonverbal synthetic cue)"),
            seed,
            &mut generator,
            None,
        )?;
    }

    // Character movement.
    let config = CharacterActionConfig {
        sample_rate: SAMPLE_RATE,
        ..CharacterActionConfig::default()
    };
    for (kind, name) in [
        (CharacterActionKind::Jump, "jump"),
        (CharacterActionKind::Landing, "landing"),
        (CharacterActionKind::Fall, "fall"),
    ] {
        let seed = next_seed(base_seed, &mut sample_index);
        let mut sample_config = config;
        sample_config.kind = kind;
        let mut generator = CharacterActionGenerator::new(sample_config, seed)?;
        pack.write_sample(
            "character-actions",
            name,
            &format!("{kind:?}"),
            seed,
            &mut generator,
            None,
        )?;
    }

    // Weapon handling.
    let config = WeaponHandlingConfig {
        sample_rate: SAMPLE_RATE,
        ..WeaponHandlingConfig::default()
    };
    for (kind, name) in [
        (WeaponHandlingKind::Draw, "draw"),
        (WeaponHandlingKind::Sheathe, "sheathe"),
        (WeaponHandlingKind::Reload, "reload"),
    ] {
        let seed = next_seed(base_seed, &mut sample_index);
        let mut sample_config = config;
        sample_config.kind = kind;
        let mut generator = WeaponHandlingGenerator::new(sample_config, seed)?;
        pack.write_sample(
            "weapon-handling",
            name,
            &format!("{kind:?} / Metal"),
            seed,
            &mut generator,
            None,
        )?;
    }

    // Clothing and carried-equipment movement.
    let config = EquipmentMovementConfig {
        sample_rate: SAMPLE_RATE,
        ..EquipmentMovementConfig::default()
    };
    for (kind, name) in [
        (EquipmentMovementKind::ClothRustle, "cloth-rustle"),
        (EquipmentMovementKind::GearJostle, "gear-jostle"),
        (EquipmentMovementKind::ArmorRattle, "armor-rattle"),
    ] {
        let seed = next_seed(base_seed, &mut sample_index);
        let mut sample_config = config;
        sample_config.kind = kind;
        let mut generator = EquipmentMovementGenerator::new(sample_config, seed)?;
        pack.write_sample(
            "equipment-movement",
            name,
            &format!("{kind:?}"),
            seed,
            &mut generator,
            None,
        )?;
    }

    // General object interactions use the default wood material.
    let config = ObjectInteractionConfig {
        sample_rate: SAMPLE_RATE,
        ..ObjectInteractionConfig::default()
    };
    for (kind, name) in [
        (ObjectInteractionKind::Pickup, "pickup"),
        (ObjectInteractionKind::Place, "place"),
        (ObjectInteractionKind::Drop, "drop"),
        (ObjectInteractionKind::Open, "open"),
        (ObjectInteractionKind::Close, "close"),
        (ObjectInteractionKind::Toggle, "toggle"),
    ] {
        let seed = next_seed(base_seed, &mut sample_index);
        let mut sample_config = config;
        sample_config.kind = kind;
        let mut generator = ObjectInteractionGenerator::new(sample_config, seed)?;
        pack.write_sample(
            "object-interactions",
            name,
            &format!("{kind:?} / Wood"),
            seed,
            &mut generator,
            None,
        )?;
    }

    // Tool actions use representative target materials.
    let config = ToolActionConfig {
        sample_rate: SAMPLE_RATE,
        ..ToolActionConfig::default()
    };
    for (kind, material, name) in [
        (ToolActionKind::Chop, ToolActionMaterial::Wood, "chop-wood"),
        (ToolActionKind::Dig, ToolActionMaterial::Earth, "dig-earth"),
        (
            ToolActionKind::Scrape,
            ToolActionMaterial::Stone,
            "scrape-stone",
        ),
        (ToolActionKind::Saw, ToolActionMaterial::Wood, "saw-wood"),
    ] {
        let seed = next_seed(base_seed, &mut sample_index);
        let mut sample_config = config;
        sample_config.kind = kind;
        sample_config.material = material;
        let mut generator = ToolActionGenerator::new(sample_config, seed)?;
        pack.write_sample(
            "tool-actions",
            name,
            &format!("{kind:?} / {material:?}"),
            seed,
            &mut generator,
            None,
        )?;
    }

    // Generic hinges, latches, locks, and cranks use the default metal profile.
    let config = MechanicalActionConfig {
        sample_rate: SAMPLE_RATE,
        ..MechanicalActionConfig::default()
    };
    for (kind, name) in [
        (MechanicalActionKind::HingeOpen, "hinge-open"),
        (MechanicalActionKind::HingeClose, "hinge-close"),
        (MechanicalActionKind::LatchEngage, "latch-engage"),
        (MechanicalActionKind::LatchRelease, "latch-release"),
        (MechanicalActionKind::LockTurn, "lock-turn"),
        (MechanicalActionKind::Crank, "crank"),
    ] {
        let seed = next_seed(base_seed, &mut sample_index);
        let mut sample_config = config;
        sample_config.kind = kind;
        let mut generator = MechanicalActionGenerator::new(sample_config, seed)?;
        pack.write_sample(
            "mechanical-actions",
            name,
            &format!("{kind:?} / Metal"),
            seed,
            &mut generator,
            None,
        )?;
    }

    // Continuous ambience must retain one generator instance across its full render.
    let ambience_frames = SAMPLE_RATE as usize * AMBIENCE_SECONDS;
    let config = WindConfig {
        sample_rate: SAMPLE_RATE,
        ..WindConfig::default()
    };
    let seed = next_seed(base_seed, &mut sample_index);
    let mut generator = WindGenerator::new(config, seed)?;
    pack.write_sample(
        "ambience",
        "wind-5s",
        "wind (five-second continuous render)",
        seed,
        &mut generator,
        Some(ambience_frames),
    )?;

    let config = RainConfig {
        sample_rate: SAMPLE_RATE,
        ..RainConfig::default()
    };
    let seed = next_seed(base_seed, &mut sample_index);
    let mut generator = RainGenerator::new(config, seed)?;
    pack.write_sample(
        "ambience",
        "rain-5s",
        "rain (five-second continuous render)",
        seed,
        &mut generator,
        Some(ambience_frames),
    )?;

    let sample_count = pack.finish()?;
    println!(
        "Rendered {sample_count} WAV files to {}",
        output_dir.display()
    );
    Ok(())
}

fn next_seed(base_seed: u64, sample_index: &mut u64) -> u64 {
    let seed = base_seed.wrapping_add(*sample_index);
    *sample_index = sample_index.wrapping_add(1);
    seed
}

struct PackWriter<'a> {
    output_dir: &'a Path,
    manifest: String,
    sample_count: usize,
}

impl<'a> PackWriter<'a> {
    fn new(output_dir: &'a Path) -> Self {
        Self {
            output_dir,
            manifest: String::from(
                "# Verdandi procedural audition pack\n\n\
                 Generated by procedural_realistic_audition.\n\n\
                 - Format: 48,000 Hz, stereo, signed 16-bit PCM WAV\n\
                 - Finite sounds: rendered until the generator reports completion\n\
                 - Wind and rain: five seconds each, rendered from one persistent generator instance\n\
                 - Seed: each file uses the base seed plus its stable position in the export sequence\n\
                 - Levels: generator defaults are retained; files are not independently normalized\n\
                 - Conversion: samples are only bounded to the WAV PCM range when encoded\n\n\
                 Files are grouped by generator family. These are audition candidates, not approved final assets.\n\n",
            ),
            sample_count: 0,
        }
    }

    fn write_sample<G: ProceduralGenerator>(
        &mut self,
        family: &str,
        name: &str,
        description: &str,
        seed: u64,
        generator: &mut G,
        frame_limit: Option<usize>,
    ) -> Result<(), Box<dyn Error>> {
        let relative_path = Path::new(family).join(format!("{name}.wav"));
        let output_path = self.output_dir.join(&relative_path);
        if let Some(parent) = output_path.parent() {
            fs::create_dir_all(parent)?;
        }

        let file = File::create(&output_path)?;
        let mut writer = BufWriter::new(file);
        writer.write_all(&[0_u8; 44])?;

        let mut samples = vec![0.0_f32; CHUNK_FRAMES * usize::from(CHANNELS)];
        let mut pcm_bytes = Vec::with_capacity(samples.len() * 2);
        let mut frames_written = 0_u64;

        loop {
            let frames = match frame_limit {
                Some(limit) => {
                    let remaining = limit.saturating_sub(frames_written as usize);
                    if remaining == 0 {
                        break;
                    }
                    remaining.min(CHUNK_FRAMES)
                }
                None => {
                    if generator.is_finished() {
                        break;
                    }
                    CHUNK_FRAMES
                }
            };

            let sample_count = frames * usize::from(CHANNELS);
            generator.render(&mut samples[..sample_count])?;

            pcm_bytes.clear();
            for &sample in &samples[..sample_count] {
                let bounded = if sample.is_finite() {
                    sample.clamp(-1.0, 1.0)
                } else {
                    0.0
                };
                let pcm = (bounded * 32_768.0)
                    .round()
                    .clamp(f32::from(i16::MIN), f32::from(i16::MAX))
                    as i16;
                pcm_bytes.extend_from_slice(&pcm.to_le_bytes());
            }
            writer.write_all(&pcm_bytes)?;
            frames_written = frames_written
                .checked_add(frames as u64)
                .ok_or("WAV frame count overflow")?;
        }

        let data_bytes = frames_written
            .checked_mul(u64::from(CHANNELS) * u64::from(BITS_PER_SAMPLE / 8))
            .ok_or("WAV data size overflow")?;
        let data_size =
            u32::try_from(data_bytes).map_err(|_| "WAV exceeds the standard RIFF size limit")?;
        let riff_size = u32::try_from(
            36_u64
                .checked_add(data_bytes)
                .ok_or("WAV RIFF size overflow")?,
        )
        .map_err(|_| "WAV exceeds the standard RIFF size limit")?;

        writer.seek(SeekFrom::Start(0))?;
        writer.write_all(&wav_header(SAMPLE_RATE, data_size, riff_size))?;
        writer.flush()?;

        self.manifest.push_str(&format!(
            "- {} — {description}; seed 0x{seed:016x}\n",
            relative_path.display()
        ));
        self.sample_count += 1;
        Ok(())
    }

    fn finish(mut self) -> Result<usize, Box<dyn Error>> {
        self.manifest.push_str(&format!(
            "\nTotal: {} WAV files.\n\n\
             To regenerate the same pack, pass the same output path and seed. The default seed is 0x5eed.\n",
            self.sample_count
        ));
        fs::write(self.output_dir.join("README.md"), self.manifest)?;
        Ok(self.sample_count)
    }
}

fn wav_header(sample_rate: u32, data_size: u32, riff_size: u32) -> [u8; 44] {
    let mut header = [0_u8; 44];
    let byte_rate = sample_rate * u32::from(CHANNELS) * u32::from(BITS_PER_SAMPLE) / 8;
    let block_align = CHANNELS * BITS_PER_SAMPLE / 8;

    header[0..4].copy_from_slice(b"RIFF");
    header[4..8].copy_from_slice(&riff_size.to_le_bytes());
    header[8..12].copy_from_slice(b"WAVE");
    header[12..16].copy_from_slice(b"fmt ");
    header[16..20].copy_from_slice(&16_u32.to_le_bytes());
    header[20..22].copy_from_slice(&1_u16.to_le_bytes()); // PCM
    header[22..24].copy_from_slice(&CHANNELS.to_le_bytes());
    header[24..28].copy_from_slice(&sample_rate.to_le_bytes());
    header[28..32].copy_from_slice(&byte_rate.to_le_bytes());
    header[32..34].copy_from_slice(&block_align.to_le_bytes());
    header[34..36].copy_from_slice(&BITS_PER_SAMPLE.to_le_bytes());
    header[36..40].copy_from_slice(b"data");
    header[40..44].copy_from_slice(&data_size.to_le_bytes());
    header
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn writes_a_stereo_48khz_16bit_pcm_wave_header() {
        let header = wav_header(SAMPLE_RATE, 480, 516);

        assert_eq!(&header[0..4], b"RIFF");
        assert_eq!(u32::from_le_bytes(header[4..8].try_into().unwrap()), 516);
        assert_eq!(&header[8..12], b"WAVE");
        assert_eq!(&header[12..16], b"fmt ");
        assert_eq!(u32::from_le_bytes(header[16..20].try_into().unwrap()), 16);
        assert_eq!(u16::from_le_bytes(header[20..22].try_into().unwrap()), 1);
        assert_eq!(
            u16::from_le_bytes(header[22..24].try_into().unwrap()),
            CHANNELS
        );
        assert_eq!(
            u32::from_le_bytes(header[24..28].try_into().unwrap()),
            SAMPLE_RATE
        );
        assert_eq!(
            u32::from_le_bytes(header[28..32].try_into().unwrap()),
            192_000
        );
        assert_eq!(u16::from_le_bytes(header[32..34].try_into().unwrap()), 4);
        assert_eq!(
            u16::from_le_bytes(header[34..36].try_into().unwrap()),
            BITS_PER_SAMPLE
        );
        assert_eq!(&header[36..40], b"data");
        assert_eq!(u32::from_le_bytes(header[40..44].try_into().unwrap()), 480);
    }
}
