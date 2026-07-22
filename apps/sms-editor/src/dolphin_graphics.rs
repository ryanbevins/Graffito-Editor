use std::fs;
use std::path::{Path, PathBuf};

const SMS_GRAPHICS_MOD_PATH: &str = "Super Mario Sunshine/metadata.json";

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct PreparedGraphicsProfile {
    pub(super) profile_path: PathBuf,
    pub(super) changed: bool,
}

pub(super) fn prepare_native_resolution_goop_profile(
    dolphin_executable: &Path,
    configured_user_directory: Option<&Path>,
    game_root: &Path,
) -> Result<PreparedGraphicsProfile, String> {
    let executable_directory = dolphin_executable.parent().ok_or_else(|| {
        format!(
            "Dolphin executable has no parent directory: {}",
            dolphin_executable.display()
        )
    })?;
    let bundled_mod = executable_directory
        .join("Sys")
        .join("Load")
        .join("GraphicMods")
        .join("Super Mario Sunshine")
        .join("metadata.json");
    if !bundled_mod.is_file() {
        return Err(format!(
            "this Dolphin installation does not include the Native Resolution Goop graphics mod at '{}'",
            bundled_mod.display()
        ));
    }

    let game_id = read_extracted_game_id(game_root)?;
    if !game_id.starts_with("GMS") {
        return Err(format!(
            "managed game '{}' has ID '{game_id}', not a Super Mario Sunshine ID",
            game_root.display()
        ));
    }
    let user_directory =
        resolve_dolphin_user_directory(dolphin_executable, configured_user_directory)?;
    let profile_path = user_directory
        .join("Config")
        .join("GraphicMods")
        .join(format!("{game_id}.json"));
    let changed = enable_bundled_mod_in_profile(&profile_path)?;
    Ok(PreparedGraphicsProfile {
        profile_path,
        changed,
    })
}

fn read_extracted_game_id(game_root: &Path) -> Result<String, String> {
    let boot_path = game_root.join("sys").join("boot.bin");
    let boot = fs::read(&boot_path).map_err(|error| {
        format!(
            "could not read managed game header '{}': {error}",
            boot_path.display()
        )
    })?;
    let id = boot.get(..6).ok_or_else(|| {
        format!(
            "managed game header '{}' is shorter than its six-byte game ID",
            boot_path.display()
        )
    })?;
    if !id.iter().all(u8::is_ascii_alphanumeric) {
        return Err(format!(
            "managed game header '{}' contains a non-ASCII game ID",
            boot_path.display()
        ));
    }
    Ok(String::from_utf8_lossy(id).into_owned())
}

fn resolve_dolphin_user_directory(
    dolphin_executable: &Path,
    configured: Option<&Path>,
) -> Result<PathBuf, String> {
    if let Some(configured) = configured {
        return Ok(configured.to_path_buf());
    }
    let executable_directory = dolphin_executable.parent().ok_or_else(|| {
        format!(
            "Dolphin executable has no parent directory: {}",
            dolphin_executable.display()
        )
    })?;
    if executable_directory.join("portable.txt").is_file() {
        return Ok(executable_directory.join("User"));
    }

    #[cfg(target_os = "windows")]
    {
        std::env::var_os("APPDATA")
            .map(PathBuf::from)
            .map(|root| root.join("Dolphin Emulator"))
            .ok_or_else(|| "APPDATA is unavailable for Dolphin's normal user profile".to_string())
    }
    #[cfg(target_os = "macos")]
    {
        std::env::var_os("HOME")
            .map(PathBuf::from)
            .map(|root| {
                root.join("Library")
                    .join("Application Support")
                    .join("Dolphin")
            })
            .ok_or_else(|| "HOME is unavailable for Dolphin's normal user profile".to_string())
    }
    #[cfg(all(not(target_os = "windows"), not(target_os = "macos")))]
    {
        std::env::var_os("XDG_DATA_HOME")
            .map(PathBuf::from)
            .or_else(|| {
                std::env::var_os("HOME")
                    .map(PathBuf::from)
                    .map(|root| root.join(".local").join("share"))
            })
            .map(|root| root.join("dolphin-emu"))
            .ok_or_else(|| {
                "XDG_DATA_HOME and HOME are unavailable for Dolphin's normal user profile"
                    .to_string()
            })
    }
}

fn enable_bundled_mod_in_profile(profile_path: &Path) -> Result<bool, String> {
    let mut root = if profile_path.is_file() {
        let bytes = fs::read(profile_path).map_err(|error| {
            format!(
                "could not read Dolphin graphics-mod profile '{}': {error}",
                profile_path.display()
            )
        })?;
        serde_json::from_slice::<serde_json::Value>(&bytes).map_err(|error| {
            format!(
                "could not parse Dolphin graphics-mod profile '{}': {error}",
                profile_path.display()
            )
        })?
    } else {
        serde_json::json!({"mods": []})
    };
    let object = root.as_object_mut().ok_or_else(|| {
        format!(
            "Dolphin graphics-mod profile '{}' is not a JSON object",
            profile_path.display()
        )
    })?;
    let mods = object
        .entry("mods")
        .or_insert_with(|| serde_json::Value::Array(Vec::new()))
        .as_array_mut()
        .ok_or_else(|| {
            format!(
                "Dolphin graphics-mod profile '{}' has a non-array mods field",
                profile_path.display()
            )
        })?;

    let mut changed = false;
    if let Some(mod_entry) = mods.iter_mut().find(|entry| {
        entry.get("source").and_then(serde_json::Value::as_str) == Some("system")
            && entry
                .get("path")
                .and_then(serde_json::Value::as_str)
                .is_some_and(|path| path.replace('\\', "/") == SMS_GRAPHICS_MOD_PATH)
    }) {
        if mod_entry
            .get("enabled")
            .and_then(serde_json::Value::as_bool)
            != Some(true)
        {
            let mod_object = mod_entry.as_object_mut().ok_or_else(|| {
                format!(
                    "Dolphin graphics-mod entry in '{}' is not a JSON object",
                    profile_path.display()
                )
            })?;
            mod_object.insert("enabled".to_string(), serde_json::Value::Bool(true));
            changed = true;
        }
    } else {
        mods.push(serde_json::json!({
            "source": "system",
            "path": SMS_GRAPHICS_MOD_PATH,
            "enabled": true,
            "weight": 0
        }));
        changed = true;
    }

    if !changed {
        return Ok(false);
    }
    let parent = profile_path.parent().ok_or_else(|| {
        format!(
            "Dolphin graphics-mod profile has no parent directory: {}",
            profile_path.display()
        )
    })?;
    fs::create_dir_all(parent).map_err(|error| {
        format!(
            "could not create Dolphin graphics-mod directory '{}': {error}",
            parent.display()
        )
    })?;
    let mut bytes = serde_json::to_vec_pretty(&root)
        .map_err(|error| format!("could not serialize Dolphin graphics-mod profile: {error}"))?;
    bytes.push(b'\n');
    fs::write(profile_path, bytes).map_err(|error| {
        format!(
            "could not update Dolphin graphics-mod profile '{}': {error}",
            profile_path.display()
        )
    })?;
    Ok(true)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn enables_native_goop_mod_without_replacing_other_profile_entries() {
        let root = tempfile::tempdir().unwrap();
        let dolphin = root.path().join("Dolphin-x64");
        let executable = dolphin.join("Dolphin.exe");
        let metadata = dolphin.join("Sys/Load/GraphicMods/Super Mario Sunshine/metadata.json");
        fs::create_dir_all(metadata.parent().unwrap()).unwrap();
        fs::write(&metadata, b"{}").unwrap();
        let game = root.path().join("game");
        fs::create_dir_all(game.join("sys")).unwrap();
        fs::write(game.join("sys/boot.bin"), b"GMSE01retail header").unwrap();
        let user = root.path().join("user");
        let profile = user.join("Config/GraphicMods/GMSE01.json");
        fs::create_dir_all(profile.parent().unwrap()).unwrap();
        fs::write(
            &profile,
            br#"{"mods":[{"source":"system","path":"Super Mario Sunshine/metadata.json","enabled":false,"groups":[{}]},{"source":"user","path":"My Mod/metadata.json","enabled":true}],"custom":7}"#,
        )
        .unwrap();

        let first =
            prepare_native_resolution_goop_profile(&executable, Some(&user), &game).unwrap();
        let second =
            prepare_native_resolution_goop_profile(&executable, Some(&user), &game).unwrap();
        let saved: serde_json::Value =
            serde_json::from_slice(&fs::read(&profile).unwrap()).unwrap();

        assert_eq!(first.profile_path, profile);
        assert!(first.changed);
        assert!(!second.changed);
        assert_eq!(saved["custom"], 7);
        assert_eq!(saved["mods"][0]["enabled"], true);
        assert_eq!(saved["mods"][0]["groups"], serde_json::json!([{}]));
        assert_eq!(saved["mods"][1]["path"], "My Mod/metadata.json");
    }

    #[test]
    fn rejects_a_non_sunshine_managed_game() {
        let root = tempfile::tempdir().unwrap();
        let dolphin = root.path().join("Dolphin-x64");
        let metadata = dolphin.join("Sys/Load/GraphicMods/Super Mario Sunshine/metadata.json");
        fs::create_dir_all(metadata.parent().unwrap()).unwrap();
        fs::write(&metadata, b"{}").unwrap();
        let game = root.path().join("game");
        fs::create_dir_all(game.join("sys")).unwrap();
        fs::write(game.join("sys/boot.bin"), b"GZLE01retail header").unwrap();

        let error = prepare_native_resolution_goop_profile(
            &dolphin.join("Dolphin.exe"),
            Some(&root.path().join("user")),
            &game,
        )
        .unwrap_err();

        assert!(error.contains("not a Super Mario Sunshine ID"));
    }
}
