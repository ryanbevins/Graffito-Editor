//! Sorts the harvest into the library an author browses.
//!
//! The harvest is every material in the game, which is too many to look
//! through and says nothing about what any of them is for. This reads it and
//! files each one under a concept and a category, using the words Sunshine
//! itself puts in its material names -- `yuka` for a floor, `kabe` for a wall,
//! `hunsui` for a fountain. The arrangement is authored here; the contents are
//! the game's.
//!
//! ```text
//! cargo run -p sms-xtask -- material-library --index <harvest.json> --out <library.json>
//! ```

use std::collections::BTreeMap;
use std::ffi::OsString;
use std::path::{Path, PathBuf};

use serde_json::{json, Value};

/// Concept, category, and the words that put a material in it.
///
/// Ordered: the first match wins, so a name carrying two words lands under the
/// more specific one -- `hunsuimizu` is a fountain before it is water.
const TAXONOMY: &[(&str, &str, &[&str])] = &[
    // Effects: what moves.
    ("Effects", "Fountains", &["hunsui", "funsui"]),
    ("Effects", "Waterfalls", &["taki"]),
    ("Effects", "Spray", &["sibuki", "shibuki"]),
    ("Effects", "Foam", &["awa", "nami"]),
    ("Effects", "Sea", &["umi", "sea"]),
    ("Effects", "Water surface", &["suimen"]),
    ("Effects", "Water", &["mizu", "water"]),
    ("Effects", "Fire", &["fire", "honoo", "hono", "kaen"]),
    ("Effects", "Smoke", &["kemuri", "smoke"]),
    ("Effects", "Pollution", &["pollution", "dorodoro"]),
    // Shading: how a surface takes light.
    ("Shading", "Reflection", &["env", "reflect"]),
    (
        "Shading",
        "Shine",
        &["tekari", "kirakira", "hikari", "shine"],
    ),
    ("Shading", "Specular", &["spec"]),
    // Structures: what a stage is built out of.
    ("Structures", "Glass", &["glass", "garasu"]),
    ("Structures", "Windows", &["mado"]),
    ("Structures", "Doors", &["doa", "door", "tobira"]),
    ("Structures", "Roofs", &["yane", "tras"]),
    ("Structures", "Walls", &["kabe", "wall"]),
    ("Structures", "Floors", &["yuka", "floor", "yuka"]),
    ("Structures", "Stairs", &["kaidan"]),
    ("Structures", "Pillars", &["hasira", "hashira"]),
    ("Structures", "Brick", &["renga", "brick"]),
    ("Structures", "Stone", &["ishi", "stone"]),
    ("Structures", "Tile", &["tile", "taile"]),
    ("Structures", "Planks", &["ita", "board", "borad"]),
    ("Structures", "Wood", &["ki_", "wood", "kigi"]),
    ("Structures", "Sand", &["suna", "sand"]),
    ("Structures", "Rock", &["iwa", "rock"]),
    ("Structures", "Grass", &["kusa", "grass", "shiba"]),
    ("Structures", "Foliage", &["ha_", "leaf", "happa"]),
    (
        "Structures",
        "Metal",
        &["tetsu", "metal", "silver", "kinzoku"],
    ),
    ("Structures", "Cloth", &["nuno", "cloth", "hata"]),
    ("Structures", "Patterns", &["moyou"]),
    // Characters: Mario, the Piantas, and everything wearing a face.
    ("Characters", "Eyes", &["eye", "manako"]),
    ("Characters", "Faces", &["kao", "mouth", "kuchi"]),
    ("Characters", "Hair", &["kami_", "hair"]),
    ("Characters", "Hats", &["boushi", "bousi", "cap_"]),
    ("Characters", "Clothes", &["fuku", "huku", "shirt", "obi"]),
    ("Characters", "Glasses", &["megane"]),
    ("Characters", "Hands", &["hand", "te_"]),
    (
        "Characters",
        "Bodies",
        &["body", "head", "ude", "asi_", "ashi_"],
    ),
    // Sky: what a stage is wrapped in.
    ("Sky", "Clouds", &["kumo", "usugumo", "cloud"]),
    ("Sky", "Sun", &["taiyou", "sun_", "sunset"]),
    ("Sky", "Sky", &["sora", "sky", "spline"]),
    // Sprites: drawn flat, in front of everything.
    ("Sprites", "Lens flare", &["lens", "flare"]),
    ("Sprites", "Glow", &["glow", "starglow", "hikari_"]),
    ("Sprites", "Collectables", &["coin", "shine_", "coin_"]),
    ("Sprites", "Effects sheets", &["fx", "efect", "effect"]),
    // Nothing named: every material still belongs somewhere, so the ones whose
    // names say nothing are filed by what they are made of instead. These three
    // match no word and are chosen by `fallback_category`.
    ("Shading", "Everything else", &[]),
    ("Effects", "Everything else", &[]),
    ("Structures", "Everything else", &[]),
];

/// Where a material goes when its name says nothing.
///
/// Measured rather than guessed: a normal-sourced texgen is a reflection
/// whatever it is called, an animated material is an effect, and what is left
/// is a surface. Most of the game's materials are named for the object they
/// dress rather than the kind of thing they are, so this is the common path,
/// not the exception.
fn fallback_category(material: &Value, animated: bool) -> &'static str {
    let normal_sourced = material["material"]["tex_gens"]
        .as_array()
        .into_iter()
        .flatten()
        .any(|gen| gen["source"].as_u64() == Some(1));
    if normal_sourced {
        "Shading"
    } else if animated {
        "Effects"
    } else {
        "Structures"
    }
}

/// Splits a name the way its author wrote it: on separators, on digits, and
/// where the case changes.
///
/// `_RockInWater_m` becomes `rock in water m`, and `_0011hunsuimizu_1` becomes
/// `hunsuimizu`. Matching against whole segments rather than anywhere in the
/// string is what stops `ha` filing every `hanachan` under foliage.
fn segments(name: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut current = String::new();
    let mut previous_lower = false;
    for character in name.chars() {
        let boundary = !character.is_ascii_alphanumeric()
            || character.is_ascii_digit()
            || (character.is_ascii_uppercase() && previous_lower);
        if boundary && !current.is_empty() {
            out.push(std::mem::take(&mut current));
        }
        if character.is_ascii_alphabetic() {
            current.push(character.to_ascii_lowercase());
        }
        previous_lower = character.is_ascii_lowercase();
    }
    if !current.is_empty() {
        out.push(current);
    }
    out
}

/// Whether a word names one of these segments.
///
/// A short word has to be the whole segment -- `ki` must not match `kinoko`.
/// A long one may sit inside a compound, because Japanese names run together:
/// `hunsui` is inside `hunsuimizu`, and that really is a fountain.
fn names(segments: &[String], word: &str) -> bool {
    segments.iter().any(|segment| {
        if word.len() < 4 {
            segment == word
        } else {
            segment.contains(word)
        }
    })
}

/// A sentence for a material, from what it is measurably made of.
///
/// Nobody wrote descriptions for eighteen thousand materials and nobody is
/// going to, but every one of them can say what it does: how many steps it
/// takes, what it samples, and whether anything moves it.
fn describe(material: &Value, animation: &str) -> String {
    let stages = material["tev_stages"].as_u64().unwrap_or(0);
    let textures = material["textures"].as_array().map(Vec::len).unwrap_or(0);
    let normal_sourced = material["material"]["tex_gens"]
        .as_array()
        .into_iter()
        .flatten()
        .any(|gen| gen["source"].as_u64() == Some(1));
    // An intensity-only texture in a later slot is a mask: no colour to give,
    // sampled only to say how much of something else gets through.
    let mask = material["textures"]
        .as_array()
        .into_iter()
        .flatten()
        .skip(1)
        .any(|texture| matches!(texture["format"].as_u64(), Some(0) | Some(1)));
    let indirect = material["indirect_stages"].as_u64().unwrap_or(0) > 0;

    let mut parts: Vec<String> = Vec::new();
    parts.push(match textures {
        0 => "No texture".to_string(),
        1 => "One texture".to_string(),
        count => format!("{count} textures"),
    });
    parts.push(match stages {
        0 | 1 => "one step".to_string(),
        count => format!("{count} steps"),
    });
    if normal_sourced {
        parts.push("reflection follows the normal".to_string());
    }
    if mask {
        parts.push("shaped by a mask".to_string());
    }
    if indirect {
        parts.push("warped by an indirect stage".to_string());
    }
    match animation.rsplit('.').next() {
        Some("btk") => parts.push("scrolled by a BTK".to_string()),
        Some("btp") => parts.push("swapped by a BTP".to_string()),
        Some("brk") => parts.push("its registers animated".to_string()),
        Some("bpk") => parts.push("its colours animated".to_string()),
        _ => {}
    }
    let mut sentence = parts.join(", ");
    if let Some(first) = sentence.get_mut(0..1) {
        first.make_ascii_uppercase();
    }
    sentence
}

/// The game's words, in English.
///
/// Sunshine names its materials and models in romaji, so a library that shows
/// those names is a library only its authors can read. These are the words that
/// actually occur, taken from the harvest rather than from a dictionary --
/// anything not here is left as it was written rather than guessed at.
const WORDS: &[(&str, &str)] = &[
    // Water and weather
    ("mizu", "water"),
    ("umi", "sea"),
    ("suimen", "water surface"),
    ("hunsui", "fountain"),
    ("funsui", "fountain"),
    ("taki", "waterfall"),
    ("sibuki", "spray"),
    ("shibuki", "spray"),
    ("awa", "foam"),
    ("nami", "wave"),
    ("bashira", "column"),
    ("hashira", "column"),
    ("hasira", "column"),
    ("tobikomi", "dive"),
    ("kumo", "cloud"),
    ("usugumo", "thin cloud"),
    ("sora", "sky"),
    ("taiyou", "sun"),
    ("kaze", "wind"),
    ("kiri", "mist"),
    ("honoo", "flame"),
    ("hono", "flame"),
    ("kaen", "flame"),
    ("kemuri", "smoke"),
    ("yuge", "steam"),
    ("koori", "ice"),
    ("yuki", "snow"),
    // Surfaces
    ("kabe", "wall"),
    ("yuka", "floor"),
    ("yane", "roof"),
    ("tras", "roof"),
    ("suna", "sand"),
    ("iwa", "rock"),
    ("ishi", "stone"),
    ("renga", "brick"),
    ("ita", "plank"),
    ("kaidan", "stairs"),
    ("mado", "window"),
    ("doa", "door"),
    ("tobira", "door"),
    ("garasu", "glass"),
    ("kusa", "grass"),
    ("shiba", "turf"),
    ("happa", "leaf"),
    ("moyou", "pattern"),
    ("tetsu", "iron"),
    ("nuno", "cloth"),
    ("hata", "flag"),
    ("yuka", "floor"),
    ("tunnel", "tunnel"),
    ("hasi", "bridge"),
    ("hashi", "bridge"),
    ("michi", "path"),
    ("jimen", "ground"),
    ("tenjou", "ceiling"),
    ("mizuumi", "lake"),
    ("shibafu", "lawn"),
    // Things
    ("hanachan", "Wiggler"),
    ("boss", "boss"),
    ("colum", "column"),
    ("kinoko", "mushroom"),
    ("ki", "tree"),
    ("hana", "flower"),
    ("mi", "fruit"),
    ("fune", "boat"),
    ("hune", "boat"),
    ("kago", "basket"),
    ("taru", "barrel"),
    ("hako", "box"),
    ("kan", "can"),
    ("isu", "chair"),
    ("tsukue", "desk"),
    ("kanban", "sign"),
    ("dokan", "pipe"),
    ("dokangate", "pipe gate"),
    ("shine", "Shine Sprite"),
    ("coin", "coin"),
    ("lens", "lens flare"),
    ("glow", "glow"),
    ("starglow", "star glow"),
    // People
    ("body", "body"),
    ("kao", "face"),
    ("kami", "hair"),
    ("boushi", "hat"),
    ("bousi", "hat"),
    ("fuku", "clothes"),
    ("huku", "clothes"),
    ("megane", "glasses"),
    ("hand", "hand"),
    ("head", "head"),
    ("ude", "arm"),
    ("asi", "leg"),
    ("ashi", "leg"),
    ("kuchi", "mouth"),
    ("mouth", "mouth"),
    ("eye", "eye"),
    ("obi", "sash"),
    ("mash", "mesh"),
    // Qualities
    ("tekari", "shine"),
    ("env", "reflection"),
    ("kage", "shadow"),
    ("hikari", "light"),
    ("kirakira", "sparkle"),
    ("nure", "wet"),
    ("mask", "mask"),
    ("noise", "noise"),
    ("basha", "splash"),
];

/// A readable name for a material, from the model it lives in and its own name.
///
/// `bosshanachan/sunabashira.bmd` and `_mizubashira1r` becomes
/// "Wiggler sand column - water column". Where a word is not one the game uses
/// often enough to have been listed, it is left alone: a name that is half
/// translated is more use than one invented whole.
fn readable_name(material: &str, model: &str) -> String {
    let english = |segment: &str| -> Option<&'static str> {
        WORDS
            .iter()
            .find(|(word, _)| *word == segment)
            .map(|(_, english)| *english)
    };
    // Longest first, so `sunabashira` is read as sand + column rather than
    // stopping at the first short word inside it.
    let read = |text: &str| -> Vec<String> {
        let mut out: Vec<String> = Vec::new();
        for segment in segments(text) {
            if let Some(word) = english(&segment) {
                out.push(word.to_string());
                continue;
            }
            // A compound: take the words off the front until nothing is left.
            let mut rest = segment.as_str();
            let mut taken: Vec<&str> = Vec::new();
            'compound: while !rest.is_empty() {
                let mut candidates: Vec<&(&str, &str)> = WORDS
                    .iter()
                    .filter(|(word, _)| rest.starts_with(word))
                    .collect();
                candidates.sort_by_key(|(word, _)| std::cmp::Reverse(word.len()));
                match candidates.first() {
                    Some((word, english)) => {
                        taken.push(english);
                        rest = &rest[word.len()..];
                    }
                    None => break 'compound,
                }
            }
            if rest.is_empty() && !taken.is_empty() {
                out.push(taken.join(" "));
            } else {
                out.push(segment);
            }
        }
        out
    };

    // The model's own folder says what the thing is; its file name says which
    // part. Where a model lives says nothing at all, so those are dropped.
    let plumbing = |piece: &&str| {
        !matches!(
            *piece,
            "map" | "mapobj" | "scene" | "mario" | "data" | "files" | "pollution"
        )
    };
    let mut parts: Vec<String> = Vec::new();
    let stem = model.trim_end_matches(".bmd").trim_end_matches(".bdl");
    let path: Vec<&str> = stem.split('/').filter(plumbing).collect();
    for piece in path.iter().rev().take(2).rev() {
        parts.extend(read(piece));
    }
    parts.extend(read(material));

    // What is left when the plumbing goes: a lone `m`, the `mat` on every
    // material in the game, the number an exporter appended. None of it tells
    // an author anything.
    let noise = |part: &String| {
        part.len() > 1
            && !matches!(
                part.as_str(),
                "mat" | "mesh" | "obj" | "tev" | "default" | "model" | "part" | "tex"
            )
    };
    // Said twice is said once: `sunabashira` in a model called `sunabashira`.
    let mut seen: Vec<String> = Vec::new();
    for part in parts.into_iter().filter(noise) {
        if seen.iter().any(|kept| kept == &part) {
            continue;
        }
        seen.push(part);
    }
    let mut name = seen.join(" ");
    if let Some(first) = name.get_mut(0..1) {
        first.make_ascii_uppercase();
    }
    if name.trim().is_empty() {
        // Nothing readable in either name: the model's own file is still
        // better than a material called `_mat0`.
        stem.rsplit('/').next().unwrap_or(material).to_string()
    } else {
        name
    }
}

pub(super) fn run(_repo_root: &Path, arguments: &[OsString]) -> Result<(), String> {
    let mut index: Option<PathBuf> = None;
    let mut out: Option<PathBuf> = None;
    let mut arguments = arguments.iter();
    while let Some(argument) = arguments.next() {
        match argument.to_str() {
            Some("--index") => {
                index = Some(PathBuf::from(
                    arguments
                        .next()
                        .ok_or_else(|| "--index needs a path".to_string())?,
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
    let index = index.ok_or_else(|| "missing --index".to_string())?;
    let out = out.ok_or_else(|| "missing --out".to_string())?;

    let harvest: Value = serde_json::from_slice(
        &std::fs::read(&index).map_err(|error| format!("read {}: {error}", index.display()))?,
    )
    .map_err(|error| format!("parse the harvest: {error}"))?;

    // Which animations name which material, so a sample carries the file that
    // drives it rather than being offered as a still.
    let mut animations: BTreeMap<(String, String), String> = BTreeMap::new();
    for effect in harvest["effects"].as_array().into_iter().flatten() {
        let (Some(stage), Some(file)) = (effect["stage"].as_str(), effect["file"].as_str()) else {
            continue;
        };
        for material in effect["materials"].as_array().into_iter().flatten() {
            let Some(material) = material.as_str() else {
                continue;
            };
            animations
                .entry((stage.to_string(), material.to_string()))
                .or_insert_with(|| file.to_string());
        }
    }

    // Keyed by concept, category, then the material's own name: one entry per
    // distinct material, remembering every stage it appears in.
    let mut filed: BTreeMap<(usize, String), BTreeMap<String, Value>> = BTreeMap::new();
    let mut unfiled = 0_usize;

    for material in harvest["materials"].as_array().into_iter().flatten() {
        let (Some(name), Some(stage), Some(model)) = (
            material["name"].as_str(),
            material["stage"].as_str(),
            material["model"].as_str(),
        ) else {
            continue;
        };
        let animation = animations
            .get(&(stage.to_string(), name.to_string()))
            .cloned()
            .unwrap_or_default();
        // The material's own name first, then the model it lives in. A material
        // called `_mat1` says nothing; the model called `columwater` says it is
        // a column of water.
        let named = segments(name);
        let from_model = segments(model);
        let (position, category) = match TAXONOMY
            .iter()
            .enumerate()
            .find(|(_, (_, _, words))| words.iter().any(|word| names(&named, word)))
            .or_else(|| {
                TAXONOMY
                    .iter()
                    .enumerate()
                    .find(|(_, (_, _, words))| words.iter().any(|word| names(&from_model, word)))
            }) {
            Some((position, (_, category, _))) => (position, *category),
            None => {
                let concept = fallback_category(material, !animation.is_empty());
                unfiled += 1;
                let position = TAXONOMY
                    .iter()
                    .position(|(owner, category, words)| {
                        *owner == concept && *category == "Everything else" && words.is_empty()
                    })
                    .expect("every concept has somewhere to put the unnamed");
                (position, "Everything else")
            }
        };
        let entry = json!({
            "name": name,
            "readable": readable_name(name, model),
            "stage": stage,
            "model": model,
            "animation": animation,
            "description": describe(material, &animation),
            "tev_stages": material["tev_stages"],
            "tex_gens": material["tex_gens"],
            "textures": material["textures"]
                .as_array()
                .map(|textures| textures.len())
                .unwrap_or(0),
            "stages": [stage],
        });
        filed
            .entry((position, category.to_string()))
            .or_default()
            // Keyed by the model rather than the stage. A course ships the same
            // model in every episode, so keying by stage listed the same
            // material five times over -- once for each episode of Pinna Beach.
            .entry(format!("{name}\u{0000}{model}"))
            .and_modify(|existing| {
                if let Some(stages) = existing["stages"].as_array_mut() {
                    if !stages.iter().any(|seen| seen == stage) {
                        stages.push(Value::String(stage.to_string()));
                    }
                }
            })
            .or_insert(entry);
    }

    let mut ordered_concepts: Vec<&str> = Vec::new();
    for (concept, _, _) in TAXONOMY {
        if !ordered_concepts.contains(concept) {
            ordered_concepts.push(concept);
        }
    }
    let mut concepts: Vec<Value> = Vec::new();
    for concept in ordered_concepts {
        let mut categories: Vec<Value> = Vec::new();
        for (position, (owner, category, words)) in TAXONOMY.iter().enumerate() {
            if *owner != concept {
                continue;
            }
            let Some(samples) = filed.get(&(position, category.to_string())) else {
                continue;
            };
            let mut samples: Vec<Value> = samples.values().cloned().collect();
            // The ones carrying the most machinery first: a material with three
            // stages and a mask is a more interesting sample than a flat one.
            samples.sort_by(|left, right| {
                right["tev_stages"]
                    .as_u64()
                    .cmp(&left["tev_stages"].as_u64())
                    .then_with(|| left["name"].as_str().cmp(&right["name"].as_str()))
            });
            // A model with five glass panels gives five materials the same
            // readable name. Numbering them keeps a name a way of telling them
            // apart, which is the only thing a name is for.
            let mut counts: BTreeMap<String, usize> = BTreeMap::new();
            for sample in &samples {
                *counts
                    .entry(sample["readable"].as_str().unwrap_or_default().to_string())
                    .or_default() += 1;
            }
            let mut seen: BTreeMap<String, usize> = BTreeMap::new();
            for sample in &mut samples {
                let readable = sample["readable"].as_str().unwrap_or_default().to_string();
                if counts.get(&readable).copied().unwrap_or(0) > 1 {
                    let index = seen.entry(readable.clone()).or_default();
                    *index += 1;
                    sample["readable"] = Value::String(format!("{readable} {index}"));
                }
            }
            categories.push(json!({
                "name": category,
                "token": words.first().copied().unwrap_or(""),
                "samples": samples,
            }));
        }
        concepts.push(json!({ "name": concept, "categories": categories }));
    }

    let library = json!({
        "note": "Every material the harvest could file, sorted by the words the game \
                 itself uses. Generated by `sms-xtask material-library`.",
        "concepts": concepts,
    });
    if let Some(parent) = out.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|error| format!("create {}: {error}", parent.display()))?;
    }
    std::fs::write(
        &out,
        serde_json::to_vec(&library).map_err(|error| error.to_string())?,
    )
    .map_err(|error| format!("write {}: {error}", out.display()))?;

    let filed_count: usize = filed.values().map(BTreeMap::len).sum();
    println!(
        "filed {filed_count} material(s), {unfiled} of them by what they are made of \
         rather than by name -> {}",
        out.display()
    );
    for concept in library["concepts"].as_array().into_iter().flatten() {
        println!("  {}", concept["name"].as_str().unwrap_or(""));
        for category in concept["categories"].as_array().into_iter().flatten() {
            println!(
                "    {:16} {}",
                category["name"].as_str().unwrap_or(""),
                category["samples"].as_array().map(Vec::len).unwrap_or(0)
            );
        }
    }
    Ok(())
}
