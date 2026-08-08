//! Harvests every animated material in the retail game into an index.
//!
//! The Material Library's effects come from the game rather than from a
//! hand-written preset list: it cannot drift from retail because it *is*
//! retail, it covers effects nobody thought to write a preset for, and its
//! parameters are measured rather than guessed.
//!
//! Run against an extracted game:
//!
//! ```text
//! cargo run -p sms-xtask -- material-index --base-root <extracted> --out <json>
//! ```

use std::collections::BTreeMap;
use std::ffi::OsString;
use std::path::{Path, PathBuf};

use serde_json::{json, Value};
use sms_formats::{J3dAnimationRebuildDocument, J3dAnimationSection, RarcArchive};

pub(super) fn run(_repo_root: &Path, arguments: &[OsString]) -> Result<(), String> {
    let mut base_root: Option<PathBuf> = None;
    let mut out: Option<PathBuf> = None;
    let mut arguments = arguments.iter();
    while let Some(argument) = arguments.next() {
        match argument.to_str() {
            Some("--base-root") => {
                base_root = Some(PathBuf::from(
                    arguments
                        .next()
                        .ok_or_else(|| "--base-root needs a path".to_string())?,
                ));
            }
            Some("--out") => {
                out = Some(PathBuf::from(
                    arguments
                        .next()
                        .ok_or_else(|| "--out needs a path".to_string())?,
                ));
            }
            Some(other) => return Err(format!("unknown argument '{other}'")),
            None => return Err("arguments must be valid Unicode".to_string()),
        }
    }
    let base_root = base_root.ok_or_else(|| "missing --base-root".to_string())?;
    let out = out.ok_or_else(|| "missing --out".to_string())?;

    let scenes = base_root.join("files").join("data").join("scene");
    let mut archives: Vec<PathBuf> = std::fs::read_dir(&scenes)
        .map_err(|error| format!("read {}: {error}", scenes.display()))?
        .filter_map(|entry| entry.ok().map(|entry| entry.path()))
        .filter(|path| path.extension().is_some_and(|ext| ext == "szs"))
        .collect();
    archives.sort();

    let mut effects: Vec<Value> = Vec::new();
    let mut materials: Vec<Value> = Vec::new();
    let mut skipped: BTreeMap<String, usize> = BTreeMap::new();

    for archive_path in &archives {
        let stage = archive_path
            .file_stem()
            .map(|stem| stem.to_string_lossy().to_string())
            .unwrap_or_default();
        let raw = match std::fs::read(archive_path) {
            Ok(raw) => raw,
            Err(error) => {
                *skipped.entry(format!("read: {error}")).or_default() += 1;
                continue;
            }
        };
        let decompressed = match sms_formats::decode_yaz0(&raw) {
            Ok(bytes) => bytes,
            Err(error) => {
                *skipped.entry(format!("yaz0: {error}")).or_default() += 1;
                continue;
            }
        };
        let archive = match RarcArchive::parse(&decompressed) {
            Ok(archive) => archive,
            Err(error) => {
                *skipped.entry(format!("rarc: {error}")).or_default() += 1;
                continue;
            }
        };
        let files = match archive.files() {
            Ok(files) => files,
            Err(error) => {
                *skipped.entry(format!("files: {error}")).or_default() += 1;
                continue;
            }
        };

        for file in files {
            let lower = file.path.to_ascii_lowercase();
            // A model carries the material itself: its TEV stages, texgens,
            // konst colours and blend state. An animation only says how one
            // number moves over time, so an index of animations alone can name
            // an effect but never reproduce it.
            if matches!(lower.rsplit('.').next(), Some("bmd") | Some("bdl")) {
                match archive.file_bytes_raw(&file.raw_path) {
                    Ok(bytes) => match harvest_model_materials(&stage, &file.path, &bytes) {
                        Ok(mut found) => materials.append(&mut found),
                        Err(error) => {
                            *skipped.entry(format!("model: {error}")).or_default() += 1;
                        }
                    },
                    Err(error) => {
                        *skipped.entry(format!("extract: {error}")).or_default() += 1;
                    }
                }
                continue;
            }
            let kind = match lower.rsplit('.').next() {
                Some("btk") => "scroll",
                Some("btp") => "pattern",
                Some("brk") => "register",
                Some("bpk") => "colour",
                _ => continue,
            };
            let bytes = match archive.file_bytes_raw(&file.raw_path) {
                Ok(bytes) => bytes,
                Err(error) => {
                    *skipped.entry(format!("extract: {error}")).or_default() += 1;
                    continue;
                }
            };
            let document = match J3dAnimationRebuildDocument::parse(&bytes) {
                Ok(document) => document,
                Err(error) => {
                    *skipped.entry(format!("{kind}: {error}")).or_default() += 1;
                    continue;
                }
            };
            let (materials, frames) = describe(&document.section);
            if materials.is_empty() {
                continue;
            }
            effects.push(json!({
                "stage": stage,
                "file": file.path,
                "kind": kind,
                "frames": frames,
                "materials": materials,
            }));
        }
    }

    let index = json!({
        "note": "Every animated material in the retail game, harvested rather than \
                 authored. The Material Library offers these as effects; what stays \
                 hand-written is which of them belong to one concept and what to call \
                 them in words an author recognises.",
        "archives": archives.len(),
        "effects": effects,
        "materials": materials,
        "skipped": skipped,
    });

    if let Some(parent) = out.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|error| format!("create {}: {error}", parent.display()))?;
    }
    std::fs::write(
        &out,
        serde_json::to_vec_pretty(&index).map_err(|error| error.to_string())?,
    )
    .map_err(|error| format!("write {}: {error}", out.display()))?;

    println!(
        "harvested {} effect(s) and {} material(s) from {} archive(s) -> {}",
        index["effects"].as_array().map(Vec::len).unwrap_or(0),
        index["materials"].as_array().map(Vec::len).unwrap_or(0),
        archives.len(),
        out.display()
    );
    if !skipped.is_empty() {
        println!("skipped:");
        for (reason, count) in &skipped {
            println!("  {count:>4}  {reason}");
        }
    }
    Ok(())
}

/// The materials an animation targets, and how long it runs.
fn describe(section: &J3dAnimationSection) -> (Vec<String>, u16) {
    let names = |table: &sms_formats::J3dAnimationNameTable| {
        table
            .entries
            .iter()
            .map(|entry| entry.name.clone())
            .collect::<Vec<_>>()
    };
    match section {
        J3dAnimationSection::TextureSrt(srt) => (names(&srt.primary.material_names), srt.max_frame),
        J3dAnimationSection::TexturePattern(pattern) => {
            (names(&pattern.material_names), pattern.max_frame)
        }
        // Joint animation targets bones rather than materials, so it is not
        // something the Material Library can offer.
        J3dAnimationSection::JointKey(_) => (Vec::new(), 0),
        J3dAnimationSection::MaterialColor(colour) => {
            (names(&colour.material_names), colour.max_frame)
        }
        J3dAnimationSection::TevRegister(register) => (
            names(&register.color_registers.material_names),
            register.max_frame,
        ),
    }
}

/// Every material a model carries, with enough of its state to tell one kind of
/// surface from another.
///
/// The whole `J3dMaterial` is kept rather than a summary: applying an effect
/// means installing that material, and a summary cannot be installed. The
/// derived fields beside it are what the library sorts and searches on.
fn harvest_model_materials(stage: &str, path: &str, bytes: &[u8]) -> Result<Vec<Value>, String> {
    let file = sms_formats::J3dFile::parse(bytes).map_err(|error| error.to_string())?;
    let preview = file.geometry_preview().map_err(|error| error.to_string())?;
    let mut found = Vec::new();
    for material in &preview.materials {
        let textures = material
            .texture_indices
            .iter()
            .flatten()
            .filter_map(|index| preview.textures.get(*index))
            .map(|texture| {
                json!({
                    "name": texture.name,
                    "width": texture.width,
                    "height": texture.height,
                    "format": texture.format,
                })
            })
            .collect::<Vec<_>>();
        found.push(json!({
            "stage": stage,
            "model": path,
            "name": material.name,
            // What the library sorts on, so a search does not have to walk the
            // whole material to answer "is this water".
            "tev_stages": material.tev_stages.len(),
            "tex_gens": material.tex_gen_count,
            "indirect_stages": material.indirect.stage_count,
            "textures": textures,
            "material": material,
        }));
    }
    Ok(found)
}
