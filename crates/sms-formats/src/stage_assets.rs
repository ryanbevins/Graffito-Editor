use std::fs;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use walkdir::WalkDir;

use crate::{decode_yaz0, FormatError, RarcArchive, Result};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum StageAssetKind {
    Archive,
    Model,
    MaterialTable,
    Texture,
    Collision,
    Message,
    Particle,
    Animation,
    Placement,
    Other,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct StageAsset {
    pub path: PathBuf,
    pub kind: StageAssetKind,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SceneArchiveInfo {
    pub stage_id: String,
    pub group: String,
    pub relative_path: PathBuf,
    pub path: PathBuf,
    pub size_bytes: u64,
}

pub fn scan_stage_assets(base_root: impl AsRef<Path>, stage_id: &str) -> Result<Vec<StageAsset>> {
    let base_root = base_root.as_ref();
    if !base_root.exists() {
        return Err(FormatError::Io(std::io::Error::new(
            std::io::ErrorKind::NotFound,
            format!("base root does not exist: {}", base_root.display()),
        )));
    }

    let needle = stage_id.to_ascii_lowercase();
    let mut assets = Vec::new();
    let scene_archives = discover_scene_archives(base_root)?;
    let selected_archives = select_scene_archives(&scene_archives, &needle)?;

    for entry in WalkDir::new(base_root)
        .follow_links(false)
        .into_iter()
        .filter_map(std::result::Result::ok)
        .filter(|entry| entry.file_type().is_file())
    {
        let path = entry.path();
        if is_repo_workspace_file(base_root, path) {
            continue;
        }

        let lower = path.to_string_lossy().to_ascii_lowercase();
        let is_selected_archive = selected_archives
            .iter()
            .any(|archive| archive.path.as_path() == path);
        let is_stage_match = needle.is_empty()
            || lower.contains(&format!("/scene/{needle}"))
            || lower.contains(&format!("\\scene\\{needle}"))
            || is_selected_archive;
        let is_common = lower.contains("/common/") || lower.contains("\\common\\");
        if !(is_stage_match || is_common) {
            continue;
        }

        assets.push(StageAsset {
            path: path.to_path_buf(),
            kind: classify_asset(path),
        });
    }

    for archive in selected_archives {
        let mounted_assets = mount_scene_archive(&archive.path)?;
        assets.extend(mounted_assets);
    }

    assets.sort_by(|a, b| a.path.cmp(&b.path));
    assets.dedup_by(|a, b| a.path == b.path);
    Ok(assets)
}

pub fn discover_scene_archives(base_root: impl AsRef<Path>) -> Result<Vec<SceneArchiveInfo>> {
    let base_root = base_root.as_ref();
    if !base_root.exists() {
        return Err(FormatError::Io(std::io::Error::new(
            std::io::ErrorKind::NotFound,
            format!("base root does not exist: {}", base_root.display()),
        )));
    }

    let mut archives = Vec::new();
    for entry in WalkDir::new(base_root)
        .follow_links(false)
        .into_iter()
        .filter_map(std::result::Result::ok)
        .filter(|entry| entry.file_type().is_file())
    {
        let path = entry.path();
        if is_repo_workspace_file(base_root, path) || !is_archive_path(path) {
            continue;
        }

        let lower = path.to_string_lossy().to_ascii_lowercase();
        let in_scene_dir = lower.contains("/data/scene/") || lower.contains("\\data\\scene\\");
        if !in_scene_dir {
            continue;
        }

        let stage_id = path
            .file_stem()
            .and_then(|stem| stem.to_str())
            .unwrap_or_default()
            .to_string();
        if stage_id.is_empty() {
            continue;
        }

        archives.push(SceneArchiveInfo {
            group: scene_group(&stage_id),
            stage_id,
            relative_path: path
                .strip_prefix(base_root)
                .map(Path::to_path_buf)
                .unwrap_or_else(|_| path.to_path_buf()),
            path: path.to_path_buf(),
            size_bytes: entry.metadata().map(|metadata| metadata.len()).unwrap_or(0),
        });
    }

    archives.sort_by(|a, b| a.stage_id.cmp(&b.stage_id));
    Ok(archives)
}

pub fn mount_scene_archive(path: impl AsRef<Path>) -> Result<Vec<StageAsset>> {
    let path = path.as_ref();
    let source = fs::read(path)?;
    let archive_bytes = if source.starts_with(b"Yaz0") {
        decode_yaz0(&source)?
    } else {
        source
    };

    let archive = RarcArchive::parse(&archive_bytes)?;
    let mut assets = Vec::new();
    for entry in archive.files()? {
        let virtual_path = format!("{}!/{}", path.display(), entry.path);
        let virtual_path = PathBuf::from(virtual_path);
        assets.push(StageAsset {
            kind: classify_asset(&virtual_path),
            path: virtual_path,
        });
    }

    Ok(assets)
}

pub fn read_stage_asset_bytes(path: impl AsRef<Path>) -> Result<Vec<u8>> {
    let path = path.as_ref();
    let path_text = path.to_string_lossy();
    if let Some((archive_path, internal_path)) = path_text.split_once("!/") {
        return extract_archive_file(archive_path, internal_path);
    }

    Ok(fs::read(path)?)
}

pub fn extract_archive_file(
    archive_path: impl AsRef<Path>,
    internal_path: impl AsRef<str>,
) -> Result<Vec<u8>> {
    let source = fs::read(archive_path)?;
    let archive_bytes = if source.starts_with(b"Yaz0") {
        decode_yaz0(&source)?
    } else {
        source
    };

    let archive = RarcArchive::parse(&archive_bytes)?;
    archive.file_bytes(internal_path.as_ref())
}

fn select_scene_archives<'a>(
    scene_archives: &'a [SceneArchiveInfo],
    needle: &str,
) -> Result<Vec<&'a SceneArchiveInfo>> {
    if needle.is_empty() {
        return Err(FormatError::Unsupported {
            format: "stage selection",
            message: "stage id cannot be empty".to_string(),
        });
    }

    let exact: Vec<&SceneArchiveInfo> = scene_archives
        .iter()
        .filter(|archive| archive.stage_id.eq_ignore_ascii_case(needle))
        .collect();
    if exact.len() == 1 {
        return Ok(exact);
    }
    if exact.len() > 1 {
        return Err(FormatError::Unsupported {
            format: "stage selection",
            message: format!("stage id '{needle}' matches multiple archives"),
        });
    }

    let fuzzy: Vec<_> = scene_archives
        .iter()
        .filter(|archive| archive.stage_id.to_ascii_lowercase().contains(needle))
        .collect();
    match fuzzy.len() {
        0 | 1 => Ok(fuzzy),
        count => Err(FormatError::Unsupported {
            format: "stage selection",
            message: format!("stage id '{needle}' is ambiguous across {count} archives"),
        }),
    }
}

fn classify_asset(path: &Path) -> StageAssetKind {
    match path
        .extension()
        .and_then(|ext| ext.to_str())
        .unwrap_or_default()
        .to_ascii_lowercase()
        .as_str()
    {
        "arc" | "szs" => StageAssetKind::Archive,
        "bmd" | "bdl" => StageAssetKind::Model,
        "bmt" => StageAssetKind::MaterialTable,
        "bti" | "bmp" => StageAssetKind::Texture,
        "col" => StageAssetKind::Collision,
        "bmg" => StageAssetKind::Message,
        "jpa" | "jpc" => StageAssetKind::Particle,
        "bck" | "btp" | "btk" | "brk" | "bas" => StageAssetKind::Animation,
        "bin" | "prm" | "map" => StageAssetKind::Placement,
        _ => StageAssetKind::Other,
    }
}

fn is_archive_path(path: &Path) -> bool {
    matches!(
        path.extension()
            .and_then(|ext| ext.to_str())
            .map(|ext| ext.to_ascii_lowercase())
            .as_deref(),
        Some("arc" | "szs")
    )
}

fn scene_group(stage_id: &str) -> String {
    let mut group = String::new();
    for ch in stage_id.chars() {
        if ch.is_ascii_digit() {
            break;
        }
        group.push(ch);
    }

    group.trim_end_matches('_').to_string()
}

fn is_repo_workspace_file(base_root: &Path, path: &Path) -> bool {
    let Ok(relative) = path.strip_prefix(base_root) else {
        return false;
    };

    let first = relative
        .components()
        .next()
        .map(|component| component.as_os_str().to_string_lossy().to_ascii_lowercase());

    matches!(
        first.as_deref(),
        Some(
            ".git"
                | ".github"
                | ".codex"
                | ".claude"
                | "build"
                | "config"
                | "docs"
                | "editor"
                | "include"
                | "src"
                | "tools"
        )
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn groups_scene_names_by_prefix() {
        assert_eq!(scene_group("dolpic0"), "dolpic");
        assert_eq!(scene_group("dolpic_ex4"), "dolpic_ex");
        assert_eq!(scene_group("biancoBoss"), "biancoBoss");
    }

    #[test]
    fn exact_scene_archive_match_wins() {
        let archives = vec![scene("dolpic0"), scene("dolpic1"), scene("dolpic_ex0")];
        let selected = select_scene_archives(&archives, "dolpic0").unwrap();
        assert_eq!(selected.len(), 1);
        assert_eq!(selected[0].stage_id, "dolpic0");
    }

    #[test]
    fn fuzzy_scene_archive_match_rejects_ambiguous_stage_ids() {
        let archives = vec![scene("dolpic0"), scene("dolpic1"), scene("bianco0")];
        assert!(select_scene_archives(&archives, "dolpic").is_err());
    }

    fn scene(stage_id: &str) -> SceneArchiveInfo {
        SceneArchiveInfo {
            stage_id: stage_id.to_string(),
            group: scene_group(stage_id),
            relative_path: PathBuf::from(format!("{stage_id}.szs")),
            path: PathBuf::from(format!("{stage_id}.szs")),
            size_bytes: 0,
        }
    }
}
