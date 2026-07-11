//! Editable stage document and filesystem mod export model.

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use sha1::{Digest, Sha1};
use sms_formats::{
    parse_jdrama_object_records, read_stage_asset_bytes, scan_stage_assets, SourceLocation,
    StageAsset, StageAssetKind,
};
use sms_schema::ObjectRegistry;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum SceneError {
    #[error("base root does not exist: {0}")]
    MissingBaseRoot(PathBuf),
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),
    #[error("format error: {0}")]
    Format(#[from] sms_formats::FormatError),
    #[error("manifest serialization error: {0}")]
    TomlSer(#[from] toml::ser::Error),
    #[error("scene overlay serialization error: {0}")]
    Json(#[from] serde_json::Error),
}

pub type Result<T> = std::result::Result<T, SceneError>;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SmsModManifest {
    pub game_id: String,
    pub region: String,
    pub base_hash: String,
    pub base_path: PathBuf,
    pub mod_files_path: PathBuf,
    pub created_with: String,
    pub changed_files: Vec<PathBuf>,
}

impl SmsModManifest {
    pub fn new(base_path: PathBuf, mod_files_path: PathBuf) -> Self {
        Self {
            game_id: "GMSJ01".to_string(),
            region: "JP".to_string(),
            base_hash: hash_path_hint(&base_path),
            base_path,
            mod_files_path,
            created_with: env!("CARGO_PKG_VERSION").to_string(),
            changed_files: Vec::new(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct StageDocument {
    pub stage_id: String,
    pub base_root: PathBuf,
    pub assets: Vec<StageAsset>,
    pub objects: Vec<SceneObject>,
    pub changed_files: BTreeMap<PathBuf, Vec<u8>>,
    pub registry: Option<ObjectRegistry>,
}

impl StageDocument {
    pub fn open(base_root: impl AsRef<Path>, stage_id: impl Into<String>) -> Result<Self> {
        let base_root = base_root.as_ref().to_path_buf();
        if !base_root.exists() {
            return Err(SceneError::MissingBaseRoot(base_root));
        }

        let stage_id = stage_id.into();
        let assets = scan_stage_assets(&base_root, &stage_id)?;
        let objects = load_scene_objects_from_assets(&assets);
        Ok(Self {
            stage_id,
            base_root,
            assets,
            objects,
            changed_files: BTreeMap::new(),
            registry: None,
        })
    }

    pub fn with_registry(mut self, registry: ObjectRegistry) -> Self {
        self.registry = Some(registry);
        self
    }

    pub fn add_object(&mut self, object: SceneObject) {
        self.objects.push(object);
    }

    pub fn mark_changed_file(&mut self, relative_path: impl Into<PathBuf>, bytes: Vec<u8>) {
        self.changed_files.insert(relative_path.into(), bytes);
    }

    pub fn queue_editor_overlay_change(&mut self) -> Result<()> {
        let path = self.editor_overlay_path();
        if self.objects.is_empty() {
            self.changed_files.remove(&path);
            return Ok(());
        }

        let overlay = EditorSceneOverlay {
            stage_id: self.stage_id.clone(),
            objects: self.objects.clone(),
        };
        let bytes = serde_json::to_vec_pretty(&overlay)?;
        self.mark_changed_file(path, bytes);
        Ok(())
    }

    pub fn editor_overlay_path(&self) -> PathBuf {
        PathBuf::from("editor")
            .join("stages")
            .join(format!("{}.scene.json", self.stage_id))
    }

    pub fn save_to_mod_folder(&self, mod_root: impl AsRef<Path>) -> Result<SmsModManifest> {
        let mod_root = mod_root.as_ref();
        fs::create_dir_all(mod_root)?;

        let files_root = mod_root.join("files");
        fs::create_dir_all(&files_root)?;

        let mut changed_files = Vec::new();
        for (relative_path, bytes) in &self.changed_files {
            let out_path = files_root.join(relative_path);
            if let Some(parent) = out_path.parent() {
                fs::create_dir_all(parent)?;
            }
            fs::write(&out_path, bytes)?;
            changed_files.push(relative_path.clone());
        }

        changed_files.sort();
        let mut manifest = SmsModManifest::new(self.base_root.clone(), files_root);
        manifest.changed_files = changed_files;

        let manifest_text = toml::to_string_pretty(&manifest)?;
        fs::write(mod_root.join("smsmod.toml"), manifest_text)?;
        Ok(manifest)
    }

    pub fn validate(&self) -> Vec<ValidationIssue> {
        let mut issues = Vec::new();

        if !self.base_root.exists() {
            issues.push(ValidationIssue::error(
                "missing-base-root",
                format!("Base root does not exist: {}", self.base_root.display()),
            ));
        }

        if self.assets.is_empty() {
            issues.push(ValidationIssue::warning(
                "no-stage-assets",
                format!("No assets found for stage '{}'", self.stage_id),
            ));
        }

        for object in &self.objects {
            if object.factory_name.trim().is_empty() {
                issues.push(ValidationIssue::error(
                    "empty-factory-name",
                    format!("Object {} has no factory name", object.id),
                ));
            }

            if !object.transform.is_finite() {
                issues.push(ValidationIssue::error(
                    "invalid-transform",
                    format!("Object {} has a non-finite transform", object.id),
                ));
            }

            if let Some(registry) = &self.registry {
                if registry.find_object(&object.factory_name).is_none() && object.source.is_none() {
                    issues.push(ValidationIssue::warning(
                        "unknown-factory",
                        format!(
                            "Object '{}' is not in the generated registry",
                            object.factory_name
                        ),
                    ));
                }
            }
        }

        issues
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct EditorSceneOverlay {
    pub stage_id: String,
    pub objects: Vec<SceneObject>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SceneObject {
    pub id: String,
    pub source: Option<SourceLocation>,
    pub factory_name: String,
    pub class_name: Option<String>,
    pub transform: Transform,
    pub raw_params: BTreeMap<String, String>,
    pub decoded_params: BTreeMap<String, ParamValue>,
    pub asset_hints: Vec<AssetRef>,
}

impl SceneObject {
    pub fn new(id: impl Into<String>, factory_name: impl Into<String>) -> Self {
        Self {
            id: id.into(),
            source: None,
            factory_name: factory_name.into(),
            class_name: None,
            transform: Transform::default(),
            raw_params: BTreeMap::new(),
            decoded_params: BTreeMap::new(),
            asset_hints: Vec::new(),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Transform {
    pub translation: [f32; 3],
    pub rotation_degrees: [f32; 3],
    pub scale: [f32; 3],
}

impl Default for Transform {
    fn default() -> Self {
        Self {
            translation: [0.0, 0.0, 0.0],
            rotation_degrees: [0.0, 0.0, 0.0],
            scale: [1.0, 1.0, 1.0],
        }
    }
}

impl Transform {
    pub fn is_finite(&self) -> bool {
        self.translation
            .iter()
            .chain(self.rotation_degrees.iter())
            .chain(self.scale.iter())
            .all(|value| value.is_finite())
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", content = "value")]
pub enum ParamValue {
    Bool(bool),
    Int(i64),
    Float(f32),
    String(String),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AssetRef {
    pub path: String,
    pub role: AssetRole,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum AssetRole {
    PreviewModel,
    Collision,
    Texture,
    Animation,
    Script,
    Unknown,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ValidationIssue {
    pub severity: ValidationSeverity,
    pub code: String,
    pub message: String,
}

impl ValidationIssue {
    pub fn warning(code: impl Into<String>, message: impl Into<String>) -> Self {
        Self {
            severity: ValidationSeverity::Warning,
            code: code.into(),
            message: message.into(),
        }
    }

    pub fn error(code: impl Into<String>, message: impl Into<String>) -> Self {
        Self {
            severity: ValidationSeverity::Error,
            code: code.into(),
            message: message.into(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum ValidationSeverity {
    Info,
    Warning,
    Error,
}

fn hash_path_hint(path: &Path) -> String {
    let mut hasher = Sha1::new();
    hasher.update(path.to_string_lossy().as_bytes());
    format!("{:x}", hasher.finalize())
}

fn load_scene_objects_from_assets(assets: &[StageAsset]) -> Vec<SceneObject> {
    let mut objects = Vec::new();
    let model_index = stage_model_index(assets);

    for asset in assets
        .iter()
        .filter(|asset| asset.kind == StageAssetKind::Placement)
    {
        let path_text = asset.path.to_string_lossy().replace('\\', "/");
        if !path_text.to_ascii_lowercase().ends_with("/map/scene.bin") {
            continue;
        }

        let Ok(bytes) = read_stage_asset_bytes(&asset.path) else {
            continue;
        };
        let Ok(records) = parse_jdrama_object_records(&bytes) else {
            continue;
        };

        for record in records {
            if scene_record_is_editor_internal(&record.type_name) {
                continue;
            }
            let Some(transform) = record.transform else {
                continue;
            };

            let type_name = record.type_name.clone();
            let object_name = record
                .object_name
                .clone()
                .unwrap_or_else(|| type_name.clone());
            let mut object =
                SceneObject::new(format!("retail-{:08x}", record.offset), type_name.clone());
            object.source = Some(SourceLocation {
                path: asset.path.clone(),
                offset: Some(record.offset as u64),
                length: Some(record.size as u64),
            });
            object.class_name = Some(type_name);
            object.transform = Transform {
                translation: transform.translation,
                rotation_degrees: transform.rotation,
                scale: transform.scale,
            };
            object
                .raw_params
                .insert("name".to_string(), object_name.clone());
            for (index, value) in record.stream_strings.iter().enumerate() {
                object
                    .raw_params
                    .insert(format!("stream_string_{index}"), value.clone());
            }
            if let Some(params) = record.npc_params {
                object.raw_params.insert(
                    "npc_body_color_index".to_string(),
                    params.color_indices[0].to_string(),
                );
                object.raw_params.insert(
                    "npc_cloth_color_index".to_string(),
                    params.color_indices[1].to_string(),
                );
                object.raw_params.insert(
                    "npc_pollution_amount".to_string(),
                    params.pollution_amount.to_string(),
                );
                object
                    .raw_params
                    .insert("npc_parts_mask".to_string(), params.parts_mask.to_string());
                for (index, value) in params.parts_color_indices.into_iter().enumerate() {
                    object
                        .raw_params
                        .insert(format!("npc_parts_color_index_{index}"), value.to_string());
                }
                object.raw_params.insert(
                    "npc_action_flags".to_string(),
                    params.action_flags.to_string(),
                );
            }

            if !scene_object_is_preview_helper(&object) {
                if let Some(model_path) = infer_preview_model_path(&object, &model_index) {
                    object.asset_hints.push(AssetRef {
                        path: model_path,
                        role: AssetRole::PreviewModel,
                    });
                }
            }

            objects.push(object);
        }
    }

    objects
}

fn scene_object_is_preview_helper(object: &SceneObject) -> bool {
    let factory = object.factory_name.to_ascii_lowercase();
    let class_name = object
        .class_name
        .as_deref()
        .unwrap_or_default()
        .to_ascii_lowercase();
    let placement_name = object
        .raw_params
        .get("name")
        .map(String::as_str)
        .unwrap_or_default()
        .to_ascii_lowercase();

    factory == "palmleaf" || class_name == "palmleaf" || placement_name.starts_with("palmleaf")
}

fn scene_record_is_editor_internal(type_name: &str) -> bool {
    let lower = type_name.to_ascii_lowercase();
    lower.contains("manager")
        || lower.contains("group")
        || lower.contains("table")
        || lower.contains("camera")
        || lower.contains("light")
        || lower.contains("scenario")
        || lower.contains("stageevent")
}

fn stage_model_index(assets: &[StageAsset]) -> Vec<(String, String)> {
    let mut models: Vec<_> = assets
        .iter()
        .filter(|asset| asset.kind == StageAssetKind::Model)
        .filter_map(|asset| {
            let path = asset.path.to_string_lossy().replace('\\', "/");
            let normalized = normalize_model_key(&path);
            (!normalized.is_empty()).then_some((path, normalized))
        })
        .collect();
    models.sort_by(|a, b| a.0.cmp(&b.0));
    models
}

fn infer_preview_model_path(
    object: &SceneObject,
    model_index: &[(String, String)],
) -> Option<String> {
    if let Some((directory, model_name)) = npc_preview_model_identity(&object.factory_name) {
        let archive_directory = format!("!/{directory}/");
        if let Some((path, _)) = model_index.iter().find(|(path, _)| {
            let lower = path.to_ascii_lowercase();
            lower.contains(&archive_directory)
                && lower
                    .rsplit('/')
                    .next()
                    .is_some_and(|name| name.eq_ignore_ascii_case(model_name))
        }) {
            return Some(path.clone());
        }
    }

    let mut keys = Vec::new();
    keys.push(normalize_model_key(&object.factory_name));
    if let Some(class_name) = &object.class_name {
        keys.push(normalize_model_key(class_name));
        if let Some(short_name) = class_name.rsplit("::").next() {
            keys.push(normalize_model_key(short_name));
        }
    }
    if let Some(name) = object.raw_params.get("name") {
        keys.push(normalize_model_key(name));
    }
    keys.retain(|key| key.len() >= 3);
    keys.sort();
    keys.dedup();

    for key in &keys {
        if let Some(path) = exact_model_key_match(key, model_index) {
            return Some(path);
        }
    }

    for key in keys {
        if let Some(path) = fuzzy_model_key_match(&key, model_index) {
            return Some(path);
        }
    }

    None
}

fn npc_preview_model_identity(factory_name: &str) -> Option<(&'static str, &'static str)> {
    match factory_name.to_ascii_lowercase().as_str() {
        "npcmontem" => Some(("montem", "mom_model.bmd")),
        "npcmontema" => Some(("montema", "moma_model.bmd")),
        "npcmontemb" => Some(("montemb", "momb_model.bmd")),
        "npcmontemc" => Some(("montemc", "momc_model.bmd")),
        "npcmontemd" => Some(("montemd", "momd_model.bmd")),
        "npcmonteme" => Some(("monteme", "mome_model.bmd")),
        // These variants deliberately reuse another Monte model in the game.
        "npcmontemf" => Some(("montem", "mom_model.bmd")),
        "npcmontemg" => Some(("montemc", "momc_model.bmd")),
        "npcmontemh" => Some(("montema", "moma_model.bmd")),
        "npcmontew" => Some(("montew", "mow_model.bmd")),
        "npcmontewa" => Some(("montewa", "mowa_model.bmd")),
        "npcmontewb" => Some(("montewb", "mowb_model.bmd")),
        "npcmontewc" => Some(("montew", "mow_model.bmd")),
        "npcmarem" | "npcmarema" | "npcmaremb" | "npcmaremc" | "npcmaremd" => {
            Some(("marem", "marem.bmd"))
        }
        "npcmarew" | "npcmarewa" | "npcmarewb" => Some(("marew", "marew.bmd")),
        "npckinopio" => Some(("kinopio", "kinopio_body.bmd")),
        "npckinojii" => Some(("kinojii", "kinoji_body.bmd")),
        "npcpeach" => Some(("peach", "peach_model.bmd")),
        "npcraccoondog" => Some(("raccoondog", "tanuki.bmd")),
        "npcboard" => Some(("boardnpc", "boardnpc.bmd")),
        _ => None,
    }
}

fn exact_model_key_match(key: &str, model_index: &[(String, String)]) -> Option<String> {
    model_index
        .iter()
        .find(|(_, model_key)| model_key == key)
        .map(|(path, _)| path.clone())
}

fn fuzzy_model_key_match(key: &str, model_index: &[(String, String)]) -> Option<String> {
    let aliases = object_model_aliases(key);
    for alias in &aliases {
        if let Some((path, _)) = model_index.iter().find(|(path, model_key)| {
            let lower = path.to_ascii_lowercase();
            (lower.contains("!/mapobj/") || lower.contains("/scene/mapobj/")) && model_key == alias
        }) {
            return Some(path.clone());
        }
    }

    model_index
        .iter()
        .filter(|(path, _)| {
            let lower = path.to_ascii_lowercase();
            lower.contains("!/mapobj/") || lower.contains("/scene/mapobj/")
        })
        .find(|(_, model_key)| {
            model_key.contains(key)
                || key.contains(model_key.as_str())
                || aliases
                    .iter()
                    .any(|alias| model_key.contains(alias) || alias.contains(model_key.as_str()))
        })
        .map(|(path, _)| path.clone())
}

fn object_model_aliases(key: &str) -> Vec<&'static str> {
    let mut aliases = Vec::new();
    if key.contains("palm") {
        aliases.push("palmnormal");
    }
    if key.contains("manhole") {
        aliases.push("manhole");
    }
    if key.contains("kibako") || key.contains("crate") || key.contains("box") {
        aliases.push("kibako");
    }
    if key.contains("barrel") {
        aliases.push("barrelnormal");
    }
    if key.contains("coin") {
        aliases.push("coin");
    }
    aliases
}

fn normalize_model_key(value: &str) -> String {
    let value = value
        .rsplit("!/")
        .next()
        .unwrap_or(value)
        .rsplit('/')
        .next()
        .unwrap_or(value)
        .strip_suffix(".bmd")
        .or_else(|| value.strip_suffix(".bdl"))
        .unwrap_or(value);

    let mut key = String::new();
    for ch in value.chars() {
        if ch.is_ascii_alphanumeric() {
            key.push(ch.to_ascii_lowercase());
        }
    }

    for prefix in ["t", "m", "sm"] {
        if key.len() > prefix.len() + 3 && key.starts_with(prefix) {
            return key[prefix.len()..].to_string();
        }
    }
    key
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detects_invalid_transform() {
        let mut doc = StageDocument {
            stage_id: "dolpic".to_string(),
            base_root: PathBuf::from("."),
            assets: vec![],
            objects: vec![],
            changed_files: BTreeMap::new(),
            registry: None,
        };
        let mut object = SceneObject::new("obj-1", "coin");
        object.transform.translation[0] = f32::NAN;
        doc.add_object(object);

        let issues = doc.validate();
        assert!(issues.iter().any(|issue| issue.code == "invalid-transform"));
    }

    #[test]
    fn queues_editor_overlay_as_changed_file() {
        let mut doc = StageDocument {
            stage_id: "dolpic".to_string(),
            base_root: PathBuf::from("."),
            assets: vec![],
            objects: vec![SceneObject::new("obj-1", "coin")],
            changed_files: BTreeMap::new(),
            registry: None,
        };

        doc.queue_editor_overlay_change().unwrap();
        assert!(doc
            .changed_files
            .contains_key(&PathBuf::from("editor/stages/dolpic.scene.json")));
    }

    #[test]
    fn resolves_npc_models_from_decomp_manager_resource_names() {
        let models = vec![
            (
                "stage.szs!/montema/moma_model.bmd".to_string(),
                "omamodel".to_string(),
            ),
            (
                "stage.szs!/kinopio/kinopio_body.bmd".to_string(),
                "kinopiobody".to_string(),
            ),
        ];
        let monte = SceneObject::new("monte", "NPCMonteMA");
        let kinopio = SceneObject::new("kinopio", "NPCKinopio");

        assert_eq!(
            infer_preview_model_path(&monte, &models).as_deref(),
            Some("stage.szs!/montema/moma_model.bmd")
        );
        assert_eq!(
            infer_preview_model_path(&kinopio, &models).as_deref(),
            Some("stage.szs!/kinopio/kinopio_body.bmd")
        );
    }

    #[test]
    fn special_monte_variants_reuse_the_game_model_directory() {
        let models = vec![(
            "stage.szs!/montema/moma_model.bmd".to_string(),
            "omamodel".to_string(),
        )];
        let object = SceneObject::new("map-shop", "NPCMonteMH");

        assert_eq!(
            infer_preview_model_path(&object, &models).as_deref(),
            Some("stage.szs!/montema/moma_model.bmd")
        );
    }
}
