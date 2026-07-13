//! Decomp-derived object and parameter registry generation.

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

use regex::Regex;
use serde::{Deserialize, Serialize};
use thiserror::Error;
use walkdir::WalkDir;

#[derive(Debug, Error)]
pub enum SchemaError {
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),
    #[error("toml error: {0}")]
    Toml(#[from] toml::de::Error),
    #[error("schema source is missing: {0}")]
    MissingSource(PathBuf),
}

pub type Result<T> = std::result::Result<T, SchemaError>;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct ObjectRegistry {
    pub objects: Vec<ObjectDefinition>,
    pub params: Vec<ParamDefinition>,
    pub asset_hints: Vec<AssetHint>,
    #[serde(default)]
    pub particle_resources: Vec<ParticleResourceDefinition>,
    #[serde(default)]
    pub actor_particle_bindings: Vec<ActorParticleBinding>,
    #[serde(default)]
    pub npc_actors: Vec<NpcActorDefinition>,
}

impl ObjectRegistry {
    pub fn find_object(&self, factory_name: &str) -> Option<&ObjectDefinition> {
        self.objects
            .iter()
            .find(|object| object.factory_name == factory_name)
    }

    pub fn find_npc_actor(&self, factory_name: &str) -> Option<&NpcActorDefinition> {
        let actor_key = factory_name
            .strip_prefix("NPC")
            .or_else(|| factory_name.strip_prefix("npc"))?;
        self.npc_actors
            .iter()
            .filter(|definition| {
                actor_key
                    .to_ascii_lowercase()
                    .starts_with(&definition.actor_key.to_ascii_lowercase())
            })
            .max_by_key(|definition| definition.actor_key.len())
    }

    pub fn apply_overlay(&mut self, overlay: SchemaOverlay) {
        let mut by_name: BTreeMap<String, ObjectOverlay> = overlay
            .objects
            .into_iter()
            .map(|object| (object.factory_name.clone(), object))
            .collect();

        for object in &mut self.objects {
            if let Some(overlay) = by_name.remove(&object.factory_name) {
                if let Some(category) = overlay.category {
                    object.category = category;
                }
                if let Some(display_name) = overlay.display_name {
                    object.display_name = Some(display_name);
                }
                if let Some(preview_model) = overlay.preview_model {
                    object.preview_model = Some(preview_model);
                }
                object.hidden |= overlay.hidden.unwrap_or(false);
                object.unsafe_to_edit |= overlay.unsafe_to_edit.unwrap_or(false);
            }
        }

        for (_, overlay) in by_name {
            self.objects.push(ObjectDefinition {
                factory_name: overlay.factory_name,
                class_name: overlay.class_name.unwrap_or_else(|| "Unknown".to_string()),
                category: overlay.category.unwrap_or_else(|| "Overlay".to_string()),
                source: SchemaSource::Overlay,
                display_name: overlay.display_name,
                preview_model: overlay.preview_model,
                hidden: overlay.hidden.unwrap_or(false),
                unsafe_to_edit: overlay.unsafe_to_edit.unwrap_or(false),
            });
        }

        self.objects.sort_by(|a, b| {
            a.category
                .cmp(&b.category)
                .then_with(|| a.factory_name.cmp(&b.factory_name))
        });
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ObjectDefinition {
    pub factory_name: String,
    pub class_name: String,
    pub category: String,
    pub source: SchemaSource,
    pub display_name: Option<String>,
    pub preview_model: Option<String>,
    pub hidden: bool,
    pub unsafe_to_edit: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ParamDefinition {
    pub owner_hint: Option<String>,
    pub member_name: String,
    pub default_value: String,
    pub source_file: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AssetHint {
    pub path: String,
    pub source_file: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ParticleResourceDefinition {
    pub effect_id: u16,
    pub path: String,
    pub source_file: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ActorParticleBinding {
    pub class_name: String,
    pub effect_id: u16,
    pub target: ParticleBindingTarget,
    pub source_file: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct NpcActorDefinition {
    pub actor_key: String,
    pub source_file: String,
    pub parts: Vec<NpcPartDefinition>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct NpcPartDefinition {
    pub bit_index: u8,
    pub color_index_channel: u8,
    pub models: Vec<NpcPartModelDefinition>,
    pub color_changes: Vec<NpcColorChangeDefinition>,
    pub uses_pollution: bool,
    pub uses_shared_materials: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct NpcPartModelDefinition {
    pub joint_name: Option<String>,
    pub model_name: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct NpcColorChangeDefinition {
    pub mode: u8,
    pub material_name: String,
    pub colors0: Vec<[i16; 4]>,
    pub colors1: Vec<[i16; 4]>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ParticleBindingTarget {
    ActorOrigin,
    ModelJoint(usize),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum SchemaSource {
    MarNameRefGen,
    MapObjManager,
    ParamInit,
    Overlay,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct SchemaOverlay {
    #[serde(default)]
    pub objects: Vec<ObjectOverlay>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ObjectOverlay {
    pub factory_name: String,
    pub class_name: Option<String>,
    pub category: Option<String>,
    pub display_name: Option<String>,
    pub preview_model: Option<String>,
    pub hidden: Option<bool>,
    pub unsafe_to_edit: Option<bool>,
}

pub struct SchemaGenerator {
    repo_root: PathBuf,
}

impl SchemaGenerator {
    pub fn new(repo_root: impl AsRef<Path>) -> Self {
        Self {
            repo_root: repo_root.as_ref().to_path_buf(),
        }
    }

    pub fn generate(&self) -> Result<ObjectRegistry> {
        let mut registry = ObjectRegistry::default();
        self.scan_mar_name_ref_gen(&mut registry)?;
        self.scan_map_obj_manager(&mut registry)?;
        self.scan_params_and_assets(&mut registry)?;
        self.scan_particle_bindings(&mut registry)?;
        self.scan_npc_init_data(&mut registry)?;
        dedup_registry(&mut registry);
        Ok(registry)
    }

    pub fn load_overlay(&self, overlay_path: impl AsRef<Path>) -> Result<SchemaOverlay> {
        let text = fs::read_to_string(overlay_path)?;
        Ok(toml::from_str(&text)?)
    }

    fn scan_mar_name_ref_gen(&self, registry: &mut ObjectRegistry) -> Result<()> {
        let path = self.repo_root.join("src/System/MarNameRefGen.cpp");
        let text = read_required(&path)?;
        extract_string_factory_returns(&text, "System", SchemaSource::MarNameRefGen, registry);
        Ok(())
    }

    fn scan_map_obj_manager(&self, registry: &mut ObjectRegistry) -> Result<()> {
        let path = self.repo_root.join("src/MoveBG/MapObjManager.cpp");
        let text = read_required(&path)?;
        extract_string_factory_returns(&text, "MapObj", SchemaSource::MapObjManager, registry);
        Ok(())
    }

    fn scan_params_and_assets(&self, registry: &mut ObjectRegistry) -> Result<()> {
        let param_re = Regex::new(r"PARAM_INIT\s*\(\s*([A-Za-z_][A-Za-z0-9_]*)\s*,\s*([^)]+)\)")
            .expect("valid param regex");
        let asset_re =
            Regex::new(r#""(/(?:scene|common|select|game_6|guide|option|subtitle)[^"]+)""#)
                .expect("valid asset regex");

        for entry in WalkDir::new(self.repo_root.join("src"))
            .into_iter()
            .chain(WalkDir::new(self.repo_root.join("include")))
            .filter_map(std::result::Result::ok)
            .filter(|entry| entry.file_type().is_file())
        {
            let path = entry.path();
            let extension = path.extension().and_then(|ext| ext.to_str());
            if !matches!(extension, Some("cpp" | "hpp" | "c" | "h")) {
                continue;
            }

            let text = fs::read_to_string(path)?;
            let source_file = normalize_source_path(&self.repo_root, path);
            let owner_hint = path
                .file_stem()
                .and_then(|stem| stem.to_str())
                .map(|stem| stem.to_string());

            for cap in param_re.captures_iter(&text) {
                registry.params.push(ParamDefinition {
                    owner_hint: owner_hint.clone(),
                    member_name: cap[1].to_string(),
                    default_value: cap[2].trim().to_string(),
                    source_file: source_file.clone(),
                });
            }

            for cap in asset_re.captures_iter(&text) {
                registry.asset_hints.push(AssetHint {
                    path: cap[1].to_string(),
                    source_file: source_file.clone(),
                });
            }
        }

        Ok(())
    }

    fn scan_particle_bindings(&self, registry: &mut ObjectRegistry) -> Result<()> {
        for entry in WalkDir::new(self.repo_root.join("src"))
            .into_iter()
            .filter_map(std::result::Result::ok)
            .filter(|entry| entry.file_type().is_file())
        {
            let path = entry.path();
            if path.extension().and_then(|ext| ext.to_str()) != Some("cpp") {
                continue;
            }
            let text = fs::read_to_string(path)?;
            let source_file = normalize_source_path(&self.repo_root, path);
            extract_particle_resources(&text, &source_file, registry);
            extract_calc_particle_bindings(&text, &source_file, registry);
        }
        Ok(())
    }

    fn scan_npc_init_data(&self, registry: &mut ObjectRegistry) -> Result<()> {
        let path = self.repo_root.join("src/NPC/NpcInitData.cpp");
        let text = read_required(&path)?;
        let source_file = normalize_source_path(&self.repo_root, &path);
        registry.npc_actors = extract_npc_actor_definitions(&text, &source_file);
        Ok(())
    }
}

#[derive(Clone)]
struct ParsedNpcModelData {
    joints: [Option<String>; 2],
    model_names: Vec<String>,
    color_changes: Vec<NpcColorChangeDefinition>,
    color_index_channel: u8,
    uses_pollution: bool,
    uses_shared_materials: bool,
}

fn extract_npc_actor_definitions(text: &str, source_file: &str) -> Vec<NpcActorDefinition> {
    let color_arrays = extract_npc_color_arrays(text);
    let color_changes = extract_npc_color_changes(text, &color_arrays);
    let model_data = extract_npc_model_data(text, &color_changes);
    let initializer_re =
        Regex::new(r"static\s+const\s+TNpcInitInfo\s+([A-Za-z_][A-Za-z0-9_]*)\s*=\s*\{")
            .expect("valid NPC initializer regex");
    let reference_re =
        Regex::new(r"&([A-Za-z_][A-Za-z0-9_]*)|nullptr").expect("valid NPC model reference regex");
    let mut actors = Vec::new();

    for captures in initializer_re.captures_iter(text) {
        let Some(whole_match) = captures.get(0) else {
            continue;
        };
        let Some(body) = braced_body(text, whole_match.end() - 1) else {
            continue;
        };
        let fields = split_cpp_initializer_fields(body);
        let Some(parts_field) = fields.get(1) else {
            continue;
        };
        let actor_key = captures[1]
            .strip_prefix('s')
            .and_then(|name| name.strip_suffix("_InitData"))
            .unwrap_or(&captures[1])
            .to_string();
        let mut parts = Vec::new();
        for (bit_index, reference) in reference_re.captures_iter(parts_field).enumerate() {
            let Some(name) = reference.get(1).map(|capture| capture.as_str()) else {
                continue;
            };
            let Some(model) = model_data.get(name) else {
                continue;
            };
            let models = model
                .model_names
                .iter()
                .enumerate()
                .map(|(index, model_name)| NpcPartModelDefinition {
                    joint_name: model.joints.get(index).cloned().flatten(),
                    model_name: model_name.clone(),
                })
                .collect();
            parts.push(NpcPartDefinition {
                bit_index: bit_index as u8,
                color_index_channel: model.color_index_channel,
                models,
                color_changes: model.color_changes.clone(),
                uses_pollution: model.uses_pollution,
                uses_shared_materials: model.uses_shared_materials,
            });
        }
        actors.push(NpcActorDefinition {
            actor_key,
            source_file: source_file.to_string(),
            parts,
        });
    }
    actors
}

fn extract_npc_color_arrays(text: &str) -> BTreeMap<String, Vec<[i16; 4]>> {
    let initializer_re =
        Regex::new(r"static\s+const\s+GXColorS10\s+([A-Za-z_][A-Za-z0-9_]*)\s*\[\s*\]\s*=\s*\{")
            .expect("valid NPC color array regex");
    let color_re =
        Regex::new(r"\{\s*(-?[0-9]+)\s*,\s*(-?[0-9]+)\s*,\s*(-?[0-9]+)\s*,\s*(-?[0-9]+)\s*\}")
            .expect("valid NPC color regex");
    let mut arrays = BTreeMap::new();
    for captures in initializer_re.captures_iter(text) {
        let Some(whole_match) = captures.get(0) else {
            continue;
        };
        let Some(body) = braced_body(text, whole_match.end() - 1) else {
            continue;
        };
        let colors = color_re
            .captures_iter(body)
            .filter_map(|color| {
                Some([
                    color[1].parse().ok()?,
                    color[2].parse().ok()?,
                    color[3].parse().ok()?,
                    color[4].parse().ok()?,
                ])
            })
            .collect::<Vec<_>>();
        arrays.insert(captures[1].to_string(), colors);
    }
    arrays
}

fn extract_npc_color_changes(
    text: &str,
    arrays: &BTreeMap<String, Vec<[i16; 4]>>,
) -> BTreeMap<String, NpcColorChangeDefinition> {
    let initializer_re =
        Regex::new(r"static\s+const\s+TColorChangeInfo\s+([A-Za-z_][A-Za-z0-9_]*)\s*=\s*\{")
            .expect("valid NPC color-change regex");
    let mut changes = BTreeMap::new();
    for captures in initializer_re.captures_iter(text) {
        let Some(whole_match) = captures.get(0) else {
            continue;
        };
        let Some(body) = braced_body(text, whole_match.end() - 1) else {
            continue;
        };
        let fields = split_cpp_initializer_fields(body);
        if fields.len() < 4 {
            continue;
        }
        let Some(mode) = parse_cpp_u32(fields[0]).and_then(|value| u8::try_from(value).ok()) else {
            continue;
        };
        let Some(material_name) = parse_cpp_string(fields[1]) else {
            continue;
        };
        let colors_for = |field: &str| {
            cpp_identifier(field)
                .and_then(|name| arrays.get(name))
                .cloned()
                .unwrap_or_default()
        };
        changes.insert(
            captures[1].to_string(),
            NpcColorChangeDefinition {
                mode,
                material_name,
                colors0: colors_for(fields[2]),
                colors1: colors_for(fields[3]),
            },
        );
    }
    changes
}

fn extract_npc_model_data(
    text: &str,
    color_changes: &BTreeMap<String, NpcColorChangeDefinition>,
) -> BTreeMap<String, ParsedNpcModelData> {
    let initializer_re =
        Regex::new(r"static\s+(?:const\s+)?TNpcModelData\s+([A-Za-z_][A-Za-z0-9_]*)\s*=\s*\{")
            .expect("valid NPC model-data regex");
    let string_re = Regex::new(r#""([^"]+)""#).expect("valid C++ string regex");
    let reference_re = Regex::new(r"&([A-Za-z_][A-Za-z0-9_]*)").expect("valid C++ reference regex");
    let mut models = BTreeMap::new();
    for captures in initializer_re.captures_iter(text) {
        let Some(whole_match) = captures.get(0) else {
            continue;
        };
        let Some(body) = braced_body(text, whole_match.end() - 1) else {
            continue;
        };
        let fields = split_cpp_initializer_fields(body);
        if fields.len() < 7 {
            continue;
        }
        let model_names = string_re
            .captures_iter(fields[2])
            .map(|model| model[1].to_string())
            .collect::<Vec<_>>();
        if model_names.is_empty() {
            continue;
        }
        let parsed_changes = reference_re
            .captures_iter(fields[3])
            .filter_map(|reference| color_changes.get(&reference[1]).cloned())
            .collect::<Vec<_>>();
        let Some(color_index_channel) =
            parse_cpp_u32(fields[4]).and_then(|value| u8::try_from(value).ok())
        else {
            continue;
        };
        models.insert(
            captures[1].to_string(),
            ParsedNpcModelData {
                joints: [parse_npc_joint(fields[0]), parse_npc_joint(fields[1])],
                model_names,
                color_changes: parsed_changes,
                color_index_channel,
                uses_pollution: parse_cpp_u32(fields[5]).is_some_and(|value| value != 0),
                uses_shared_materials: parse_cpp_u32(fields[6]).is_some_and(|value| value != 0),
            },
        );
    }
    models
}

fn parse_npc_joint(field: &str) -> Option<String> {
    if field.contains("cNpcPartsNameRootJoint") || field.trim() == "0" || field.trim() == "nullptr"
    {
        return None;
    }
    parse_cpp_string(field)
}

fn parse_cpp_string(field: &str) -> Option<String> {
    let start = field.find('"')? + 1;
    let end = field[start..].find('"')? + start;
    Some(field[start..end].to_string())
}

fn cpp_identifier(field: &str) -> Option<&str> {
    let field = field.trim().trim_start_matches('&').trim();
    (!field.is_empty() && field != "nullptr" && field != "0").then_some(field)
}

fn split_cpp_initializer_fields(body: &str) -> Vec<&str> {
    let bytes = body.as_bytes();
    let mut fields = Vec::new();
    let mut start = 0usize;
    let mut depth = 0usize;
    let mut index = 0usize;
    while index < bytes.len() {
        match bytes[index] {
            b'{' | b'(' | b'[' => depth += 1,
            b'}' | b')' | b']' => depth = depth.saturating_sub(1),
            b'"' | b'\'' => {
                let quote = bytes[index];
                index += 1;
                while index < bytes.len() && bytes[index] != quote {
                    if bytes[index] == b'\\' {
                        index += 1;
                    }
                    index += 1;
                }
            }
            b',' if depth == 0 => {
                fields.push(body[start..index].trim());
                start = index + 1;
            }
            _ => {}
        }
        index += 1;
    }
    let tail = body[start..].trim();
    if !tail.is_empty() {
        fields.push(tail);
    }
    fields
}

fn parse_cpp_u32(value: &str) -> Option<u32> {
    let value = value.trim();
    if let Some(hex) = value
        .strip_prefix("0x")
        .or_else(|| value.strip_prefix("0X"))
    {
        u32::from_str_radix(hex, 16).ok()
    } else {
        value.parse().ok()
    }
}

fn extract_particle_resources(text: &str, source_file: &str, registry: &mut ObjectRegistry) {
    let load_re = Regex::new(
        r#"(?:gpResourceManager|[A-Za-z_][A-Za-z0-9_]*ResourceManager)\s*->\s*load\s*\(\s*\"([^\"]+\.jpa)\"\s*,\s*(0[xX][0-9A-Fa-f]+|[0-9]+)"#,
    )
    .expect("valid particle resource regex");
    for captures in load_re.captures_iter(text) {
        let Some(effect_id) = parse_cpp_u16(&captures[2]) else {
            continue;
        };
        registry
            .particle_resources
            .push(ParticleResourceDefinition {
                effect_id,
                path: captures[1].to_string(),
                source_file: source_file.to_string(),
            });
    }
}

fn extract_calc_particle_bindings(text: &str, source_file: &str, registry: &mut ObjectRegistry) {
    let calc_re = Regex::new(r"([A-Za-z_][A-Za-z0-9_:]*)::calc\s*\([^)]*\)\s*(?:const\s*)?\{")
        .expect("valid calc method regex");
    let matrix_re = Regex::new(
        r"(?:MtxPtr|Mtx\s*\*)\s+([A-Za-z_][A-Za-z0-9_]*)\s*=\s*[^;]*mNodeMatrices\s*\[\s*([0-9]+)\s*\]",
    )
    .expect("valid particle matrix regex");
    let emit_re = Regex::new(
        r"emitAndBind(ToPosPtr|ToMtxPtr|ToSRTMtxPtr|ToMtx)\s*\(\s*(0[xX][0-9A-Fa-f]+|[0-9]+)\s*,\s*([^,\n]+)",
    )
    .expect("valid actor particle emission regex");
    let direct_joint_re =
        Regex::new(r"mNodeMatrices\s*\[\s*([0-9]+)\s*\]").expect("valid direct joint regex");

    for captures in calc_re.captures_iter(text) {
        let Some(whole_match) = captures.get(0) else {
            continue;
        };
        let Some(body) = braced_body(text, whole_match.end() - 1) else {
            continue;
        };
        let matrix_joints = matrix_re
            .captures_iter(body)
            .filter_map(|captures| {
                Some((captures[1].to_string(), captures[2].parse::<usize>().ok()?))
            })
            .collect::<BTreeMap<_, _>>();
        for emission in emit_re.captures_iter(body) {
            let Some(effect_id) = parse_cpp_u16(&emission[2]) else {
                continue;
            };
            let target = if &emission[1] == "ToPosPtr" {
                Some(ParticleBindingTarget::ActorOrigin)
            } else {
                let argument = emission[3].trim();
                matrix_joints
                    .get(argument)
                    .copied()
                    .map(ParticleBindingTarget::ModelJoint)
                    .or_else(|| {
                        direct_joint_re
                            .captures(argument)
                            .and_then(|captures| captures[1].parse::<usize>().ok())
                            .map(ParticleBindingTarget::ModelJoint)
                    })
            };
            let Some(target) = target else {
                continue;
            };
            registry.actor_particle_bindings.push(ActorParticleBinding {
                class_name: captures[1].to_string(),
                effect_id,
                target,
                source_file: source_file.to_string(),
            });
        }
    }
}

fn parse_cpp_u16(value: &str) -> Option<u16> {
    let value = value.trim();
    if let Some(hex) = value
        .strip_prefix("0x")
        .or_else(|| value.strip_prefix("0X"))
    {
        u16::from_str_radix(hex, 16).ok()
    } else {
        value.parse().ok()
    }
}

fn braced_body(text: &str, open_brace: usize) -> Option<&str> {
    let bytes = text.as_bytes();
    if bytes.get(open_brace).copied() != Some(b'{') {
        return None;
    }
    let mut depth = 0usize;
    let mut index = open_brace;
    while index < bytes.len() {
        match bytes[index] {
            b'{' => depth += 1,
            b'}' => {
                depth = depth.checked_sub(1)?;
                if depth == 0 {
                    return text.get(open_brace + 1..index);
                }
            }
            b'"' | b'\'' => {
                let quote = bytes[index];
                index += 1;
                while index < bytes.len() && bytes[index] != quote {
                    if bytes[index] == b'\\' {
                        index += 1;
                    }
                    index += 1;
                }
            }
            b'/' if bytes.get(index + 1).copied() == Some(b'/') => {
                index += 2;
                while index < bytes.len() && bytes[index] != b'\n' {
                    index += 1;
                }
            }
            b'/' if bytes.get(index + 1).copied() == Some(b'*') => {
                index += 2;
                while index + 1 < bytes.len() && !(bytes[index] == b'*' && bytes[index + 1] == b'/')
                {
                    index += 1;
                }
                index += 1;
            }
            _ => {}
        }
        index += 1;
    }
    None
}

fn extract_string_factory_returns(
    text: &str,
    category: &str,
    source: SchemaSource,
    registry: &mut ObjectRegistry,
) {
    let factory_return_re = Regex::new(
        r#"strcmp\s*\(\s*name\s*,\s*"([^"]+)"\s*\)\s*==\s*0\s*\)\s*(?:\{[^}]*?)?return\s+(?:[A-Za-z0-9_:]+\s*=\s*)?new\s+([A-Za-z_:][A-Za-z0-9_:]*)"#,
    )
    .expect("valid factory regex");

    for cap in factory_return_re.captures_iter(text) {
        let factory_name = cap[1].to_string();
        let class_name = cap[2].to_string();
        registry.objects.push(ObjectDefinition {
            factory_name,
            class_name,
            category: category.to_string(),
            source: source.clone(),
            display_name: None,
            preview_model: None,
            hidden: false,
            unsafe_to_edit: false,
        });
    }

    let compare_re = Regex::new(r#"strcmp\s*\(\s*name\s*,\s*"([^"]+)"\s*\)\s*==\s*0"#)
        .expect("valid strcmp regex");
    for cap in compare_re.captures_iter(text) {
        let factory_name = cap[1].to_string();
        if registry
            .objects
            .iter()
            .any(|object| object.factory_name == factory_name)
        {
            continue;
        }

        registry.objects.push(ObjectDefinition {
            factory_name,
            class_name: "Unknown".to_string(),
            category: category.to_string(),
            source: source.clone(),
            display_name: None,
            preview_model: None,
            hidden: false,
            unsafe_to_edit: false,
        });
    }

    let static_array_re = Regex::new(r#""([A-Za-z0-9_./-]+)""#).expect("valid string regex");
    for cap in static_array_re.captures_iter(text) {
        let factory_name = cap[1].to_string();
        if !looks_like_factory_name(&factory_name)
            || registry
                .objects
                .iter()
                .any(|object| object.factory_name == factory_name)
        {
            continue;
        }

        registry.objects.push(ObjectDefinition {
            factory_name,
            class_name: "Unknown".to_string(),
            category: category.to_string(),
            source: source.clone(),
            display_name: None,
            preview_model: None,
            hidden: false,
            unsafe_to_edit: false,
        });
    }
}

fn looks_like_factory_name(value: &str) -> bool {
    !value.contains('/')
        && !value.contains('.')
        && value
            .chars()
            .all(|ch| ch.is_ascii_alphanumeric() || ch == '_' || ch == '-')
        && value.chars().any(|ch| ch.is_ascii_alphabetic())
}

fn dedup_registry(registry: &mut ObjectRegistry) {
    let mut objects = BTreeMap::<String, ObjectDefinition>::new();
    for object in registry.objects.drain(..) {
        objects.entry(object.factory_name.clone()).or_insert(object);
    }
    registry.objects = objects.into_values().collect();
    registry.objects.sort_by(|a, b| {
        a.category
            .cmp(&b.category)
            .then_with(|| a.factory_name.cmp(&b.factory_name))
    });

    registry.params.sort_by(|a, b| {
        a.source_file
            .cmp(&b.source_file)
            .then_with(|| a.member_name.cmp(&b.member_name))
    });
    registry.params.dedup();

    registry.asset_hints.sort_by(|a, b| {
        a.path
            .cmp(&b.path)
            .then_with(|| a.source_file.cmp(&b.source_file))
    });
    registry.asset_hints.dedup();

    registry.particle_resources.sort_by(|a, b| {
        a.effect_id
            .cmp(&b.effect_id)
            .then_with(|| a.path.cmp(&b.path))
            .then_with(|| a.source_file.cmp(&b.source_file))
    });
    registry.particle_resources.dedup();

    registry.actor_particle_bindings.sort_by(|a, b| {
        a.class_name
            .cmp(&b.class_name)
            .then_with(|| a.effect_id.cmp(&b.effect_id))
            .then_with(|| a.source_file.cmp(&b.source_file))
    });
    registry.actor_particle_bindings.dedup();

    registry
        .npc_actors
        .sort_by(|a, b| a.actor_key.cmp(&b.actor_key));
    registry
        .npc_actors
        .dedup_by(|a, b| a.actor_key == b.actor_key);
}

fn read_required(path: &Path) -> Result<String> {
    if !path.exists() {
        return Err(SchemaError::MissingSource(path.to_path_buf()));
    }
    Ok(fs::read_to_string(path)?)
}

fn normalize_source_path(root: &Path, path: &Path) -> String {
    path.strip_prefix(root)
        .unwrap_or(path)
        .to_string_lossy()
        .replace('\\', "/")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extracts_simple_factory_return() {
        let text = r#"
            if (strcmp(name, "Mario") == 0)
                return new TMario;
            if (strcmp(name, "MarScene") == 0)
                return new JDrama::TSmJ3DScn;
        "#;
        let mut registry = ObjectRegistry::default();
        extract_string_factory_returns(text, "System", SchemaSource::MarNameRefGen, &mut registry);
        assert_eq!(registry.objects.len(), 2);
        assert_eq!(registry.objects[0].factory_name, "Mario");
        assert_eq!(registry.objects[1].class_name, "JDrama::TSmJ3DScn");
    }

    #[test]
    fn keeps_compare_only_factory_names() {
        let text = r#"
            if (strcmp(name, "coin") == 0)
                return gpItemManager->unk78;
        "#;
        let mut registry = ObjectRegistry::default();
        extract_string_factory_returns(text, "MapObj", SchemaSource::MapObjManager, &mut registry);
        assert_eq!(registry.objects[0].factory_name, "coin");
        assert_eq!(registry.objects[0].class_name, "Unknown");
    }

    #[test]
    fn discovers_particle_resources_and_calc_joint_bindings() {
        let resources = r#"
            gpResourceManager->load("ms_glow.jpa", 7);
        "#;
        let actor = r#"
            void TExample::calc()
            {
                MtxPtr effectMtx = mMActor->getModel()->mNodeMatrices[2];
                gpMarioParticleManager->emitAndBindToMtxPtr(7, effectMtx, 1, this);
            }
        "#;
        let mut registry = ObjectRegistry::default();
        extract_particle_resources(resources, "src/System/Resources.cpp", &mut registry);
        extract_calc_particle_bindings(actor, "src/MoveBG/Example.cpp", &mut registry);

        assert_eq!(registry.particle_resources[0].effect_id, 7);
        assert_eq!(registry.particle_resources[0].path, "ms_glow.jpa");
        assert_eq!(registry.actor_particle_bindings[0].class_name, "TExample");
        assert_eq!(
            registry.actor_particle_bindings[0].target,
            ParticleBindingTarget::ModelJoint(2)
        );
    }

    #[test]
    fn ignores_transient_particle_emissions_outside_calc() {
        let actor = r#"
            void TExample::explode()
            {
                gpMarioParticleManager->emitAndBindToPosPtr(0x80, &mPosition, 1, this);
            }
        "#;
        let mut registry = ObjectRegistry::default();
        extract_calc_particle_bindings(actor, "src/MoveBG/Example.cpp", &mut registry);

        assert!(registry.actor_particle_bindings.is_empty());
    }

    #[test]
    fn extracts_npc_parts_and_palettes_from_initializers() {
        let text = r#"
            static const GXColorS10 sHatColors0[] = {
                { 10, 20, 30, 255 }, { -40, 50, 60, 255 },
            };
            static const GXColorS10 sHatColors1[] = {
                { 70, 80, 90, 255 }, { 100, 110, 120, 255 },
            };
            static const TColorChangeInfo sHatChange = {
                0x00000002, "_hat", sHatColors0, sHatColors1
            };
            static const TNpcModelData sHatData = {
                "kubi", 0, { "customHat.bmd" }, { { &sHatChange, 0 } }, 1, 1, 1,
            };
            static TNpcModelData sRodData = {
                cNpcPartsNameRootJoint, 0, { "customRod.bmd" }, {}, 0, 0, 0,
            };
            static const TNpcInitInfo sMareM_InitData = {
                nullptr, { &sHatData, nullptr, &sRodData }, {}, 1.0f, 2.0f, 3.0f, 4.0f,
            };
        "#;
        let actors = extract_npc_actor_definitions(text, "src/NPC/NpcInitData.cpp");
        let actor = &actors[0];
        assert_eq!(actor.actor_key, "MareM");
        assert_eq!(actor.parts.len(), 2);
        assert_eq!(actor.parts[0].bit_index, 0);
        assert_eq!(actor.parts[0].color_index_channel, 1);
        assert_eq!(actor.parts[0].models[0].joint_name.as_deref(), Some("kubi"));
        assert_eq!(actor.parts[0].models[0].model_name, "customHat.bmd");
        assert_eq!(actor.parts[0].color_changes[0].mode, 2);
        assert_eq!(
            actor.parts[0].color_changes[0].colors0[1],
            [-40, 50, 60, 255]
        );
        assert!(actor.parts[0].uses_pollution);
        assert!(actor.parts[0].uses_shared_materials);
        assert_eq!(actor.parts[1].bit_index, 2);
        assert_eq!(actor.parts[1].models[0].joint_name, None);
    }

    #[test]
    fn npc_schema_lookup_prefers_the_longest_actor_key() {
        let registry = ObjectRegistry {
            npc_actors: vec![
                NpcActorDefinition {
                    actor_key: "MareM".to_string(),
                    source_file: String::new(),
                    parts: Vec::new(),
                },
                NpcActorDefinition {
                    actor_key: "MareMB".to_string(),
                    source_file: String::new(),
                    parts: Vec::new(),
                },
            ],
            ..ObjectRegistry::default()
        };
        assert_eq!(
            registry.find_npc_actor("NPCMareMA").unwrap().actor_key,
            "MareM"
        );
        assert_eq!(
            registry.find_npc_actor("NPCMareMB").unwrap().actor_key,
            "MareMB"
        );
    }

    #[test]
    fn overlay_updates_existing_objects() {
        let mut registry = ObjectRegistry {
            objects: vec![ObjectDefinition {
                factory_name: "coin".to_string(),
                class_name: "TCoin".to_string(),
                category: "MapObj".to_string(),
                source: SchemaSource::MapObjManager,
                display_name: None,
                preview_model: None,
                hidden: false,
                unsafe_to_edit: false,
            }],
            ..Default::default()
        };

        registry.apply_overlay(SchemaOverlay {
            objects: vec![ObjectOverlay {
                factory_name: "coin".to_string(),
                class_name: None,
                category: Some("Item".to_string()),
                display_name: Some("Coin".to_string()),
                preview_model: Some("/scene/mapObj/coin.bmd".to_string()),
                hidden: None,
                unsafe_to_edit: Some(true),
            }],
        });

        let coin = registry.find_object("coin").unwrap();
        assert_eq!(coin.category, "Item");
        assert!(coin.unsafe_to_edit);
    }
}
