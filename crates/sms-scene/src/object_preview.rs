use std::collections::{BTreeMap, BTreeSet};
use std::path::{Component, Path, PathBuf};

use sms_formats::{mount_scene_archive, StageAsset, StageAssetKind};
use sms_schema::{ObjectPreviewDefinition, ObjectRegistry};

use super::{actor_preview_factory_key, ActorPreview, ResolvedObjectPreview, ValidationIssue};

pub(super) fn discover_object_preview_assets(
    base_root: &Path,
    registry: &ObjectRegistry,
) -> (Vec<StageAsset>, Vec<ValidationIssue>) {
    let mut definitions_by_archive = BTreeMap::<PathBuf, Vec<&ObjectPreviewDefinition>>::new();
    let mut issues = Vec::new();

    for definition in &registry.object_previews {
        let Some(archive_path) =
            locate_runtime_archive(base_root, &definition.runtime_archive_path)
        else {
            let candidates =
                runtime_archive_candidates(base_root, &definition.runtime_archive_path);
            let searched = candidates
                .iter()
                .map(|path| path.display().to_string())
                .collect::<Vec<_>>()
                .join(", ");
            issues.push(ValidationIssue::warning(
                "object-preview-runtime-archive-unavailable",
                format!(
                    "Could not find runtime archive '{}' for {} object preview from {}; searched: {}",
                    definition.runtime_archive_path,
                    definition.factory_name,
                    preview_source_label(definition),
                    if searched.is_empty() {
                        "no safe candidate paths".to_string()
                    } else {
                        searched
                    }
                ),
            ));
            continue;
        };
        definitions_by_archive
            .entry(archive_path)
            .or_default()
            .push(definition);
    }

    let mut selected_assets = Vec::new();
    for (archive_path, definitions) in definitions_by_archive {
        let mounted_assets = match mount_scene_archive(&archive_path) {
            Ok(assets) => assets,
            Err(error) => {
                issues.push(ValidationIssue::warning(
                    "object-preview-runtime-archive-read-failed",
                    format!(
                        "Could not mount {} for registry-backed object previews: {error}",
                        archive_path.display()
                    ),
                ));
                continue;
            }
        };

        for definition in definitions {
            for (resource_role, runtime_path, expected_kind) in
                declared_object_preview_resources(definition)
            {
                let mut matches = mounted_assets
                    .iter()
                    .filter(|asset| {
                        object_preview_asset_matches_resource(
                            asset.path.as_path(),
                            definition,
                            runtime_path,
                        )
                    })
                    .collect::<Vec<_>>();
                matches.sort_by(|left, right| left.path.cmp(&right.path));
                matches.dedup_by(|left, right| left.path == right.path);
                match matches.as_slice() {
                    [asset] if asset.kind == expected_kind => selected_assets.push((*asset).clone()),
                    [asset] => issues.push(ValidationIssue::warning(
                        "object-preview-resource-kind-mismatch",
                        format!(
                            "Runtime {} '{}' for {} resolved to {} with kind {:?}, expected {:?} from {}",
                            resource_role,
                            runtime_path,
                            definition.factory_name,
                            asset.path.display(),
                            asset.kind,
                            expected_kind,
                            preview_source_label(definition)
                        ),
                    )),
                    [] => issues.push(ValidationIssue::warning(
                        "object-preview-resource-unresolved",
                        format!(
                            "Could not resolve runtime {} '{}' for {} inside {} from {}",
                            resource_role,
                            runtime_path,
                            definition.factory_name,
                            archive_path.display(),
                            preview_source_label(definition)
                        ),
                    )),
                    matches => issues.push(ValidationIssue::warning(
                        "object-preview-resource-ambiguous",
                        format!(
                            "Runtime {} '{}' for {} matched multiple members of {}: {}",
                            resource_role,
                            runtime_path,
                            definition.factory_name,
                            archive_path.display(),
                            matches
                                .iter()
                                .map(|asset| asset.path.display().to_string())
                                .collect::<Vec<_>>()
                                .join(", ")
                        ),
                    )),
                }
            }
        }
    }

    selected_assets.sort_by(|left, right| left.path.cmp(&right.path));
    selected_assets.dedup_by(|left, right| left.path == right.path);
    (selected_assets, issues)
}

pub(super) fn remove_registry_object_preview_assets(
    assets: &mut Vec<StageAsset>,
    registry: &ObjectRegistry,
) {
    assets.retain(|asset| {
        !registry
            .object_previews
            .iter()
            .any(|definition| object_preview_asset_matches_definition(&asset.path, definition))
    });
}

pub(super) fn install_registry_actor_previews(
    catalog: &mut BTreeMap<String, ActorPreview>,
    assets: &[StageAsset],
    registry: &ObjectRegistry,
) {
    for definition in &registry.object_previews {
        let Some(model_path) =
            resolve_object_preview_resource_path(definition, &definition.model_path, assets)
        else {
            continue;
        };
        catalog.insert(
            actor_preview_factory_key(&definition.factory_name),
            ActorPreview {
                model_path,
                load_flags: definition.load_flags,
                manager_factory: format!("{} runtime preview", definition.factory_name),
                runtime_uniform_scale: None,
            },
        );
    }
}

pub(super) fn resolve_object_preview_definition(
    definition: &ObjectPreviewDefinition,
    assets: &[StageAsset],
) -> Option<ResolvedObjectPreview> {
    let model_path =
        resolve_object_preview_resource_path(definition, &definition.model_path, assets)?;
    let idle_bck_path =
        resolve_object_preview_resource_path(definition, &definition.idle_bck_path, assets)?;
    let idle_btp_path = definition
        .idle_btp_path
        .as_deref()
        .and_then(|runtime_path| {
            resolve_object_preview_resource_path(definition, runtime_path, assets)
        });
    Some(ResolvedObjectPreview {
        factory_name: definition.factory_name.clone(),
        model_path,
        load_flags: definition.load_flags,
        idle_bck_path,
        idle_btp_path,
        idle_playback_rate_numerator: definition.idle_playback_rate_numerator,
        idle_playback_rate_denominator: definition.idle_playback_rate_denominator,
        hidden_shape_indices: definition.hidden_shape_indices.clone(),
        tev_k_color_alpha_overrides: definition.tev_k_color_alpha_overrides.clone(),
    })
}

pub(super) fn resolve_object_preview_resource_path(
    definition: &ObjectPreviewDefinition,
    runtime_resource_path: &str,
    assets: &[StageAsset],
) -> Option<String> {
    let mut matches = assets
        .iter()
        .filter(|asset| {
            object_preview_asset_matches_resource(
                asset.path.as_path(),
                definition,
                runtime_resource_path,
            )
        })
        .map(|asset| asset.path.to_string_lossy().replace('\\', "/"))
        .collect::<Vec<_>>();
    matches.sort();
    matches.dedup();
    (matches.len() == 1).then(|| matches.remove(0))
}

pub(super) fn is_registry_object_preview_model_asset(
    path: &Path,
    registry: &ObjectRegistry,
) -> bool {
    registry.object_previews.iter().any(|definition| {
        object_preview_asset_matches_resource(path, definition, &definition.model_path)
    })
}

pub(super) fn object_preview_asset_matches_definition(
    path: &Path,
    definition: &ObjectPreviewDefinition,
) -> bool {
    declared_object_preview_resources(definition)
        .into_iter()
        .any(|(_, runtime_path, _)| {
            object_preview_asset_matches_resource(path, definition, runtime_path)
        })
}

pub(super) fn object_preview_asset_matches_resource(
    path: &Path,
    definition: &ObjectPreviewDefinition,
    runtime_resource_path: &str,
) -> bool {
    let path = path.to_string_lossy().replace('\\', "/");
    let Some((archive_path, internal_path)) = path.split_once("!/") else {
        return false;
    };
    let Some(actual_archive_stem) = Path::new(archive_path)
        .file_stem()
        .and_then(|stem| stem.to_str())
    else {
        return false;
    };
    let Some(runtime_archive_stem) = runtime_archive_stem(&definition.runtime_archive_path) else {
        return false;
    };
    actual_archive_stem.eq_ignore_ascii_case(&runtime_archive_stem)
        && object_preview_internal_path_candidates(definition, runtime_resource_path)
            .iter()
            .any(|candidate| candidate == internal_path)
}

fn declared_object_preview_resources(
    definition: &ObjectPreviewDefinition,
) -> Vec<(&'static str, &str, StageAssetKind)> {
    let mut resources = vec![
        (
            "model",
            definition.model_path.as_str(),
            StageAssetKind::Model,
        ),
        (
            "idle joint animation",
            definition.idle_bck_path.as_str(),
            StageAssetKind::Animation,
        ),
    ];
    if let Some(path) = definition.idle_btp_path.as_deref() {
        resources.push((
            "idle texture-pattern animation",
            path,
            StageAssetKind::Animation,
        ));
    }
    resources
}

fn locate_runtime_archive(base_root: &Path, runtime_archive_path: &str) -> Option<PathBuf> {
    runtime_archive_candidates(base_root, runtime_archive_path)
        .into_iter()
        .find(|path| path.is_file())
}

fn runtime_archive_candidates(base_root: &Path, runtime_archive_path: &str) -> Vec<PathBuf> {
    let normalized = runtime_archive_path.replace('\\', "/");
    let relative = PathBuf::from(normalized.trim_start_matches('/'));
    if relative.as_os_str().is_empty()
        || !relative
            .components()
            .all(|component| matches!(component, Component::Normal(_)))
    {
        return Vec::new();
    }

    let mut relative_variants = Vec::new();
    if relative
        .extension()
        .and_then(|extension| extension.to_str())
        .is_some_and(|extension| extension.eq_ignore_ascii_case("arc"))
    {
        relative_variants.push(relative.with_extension("szs"));
    }
    relative_variants.push(relative);
    relative_variants.sort();
    relative_variants.dedup();

    let mut candidates = Vec::new();
    for relative in relative_variants {
        candidates.push(base_root.join("files").join(&relative));
        candidates.push(base_root.join(&relative));
        if let Some(file_name) = relative.file_name() {
            candidates.push(base_root.join("files/data").join(file_name));
            candidates.push(base_root.join("data").join(file_name));
            candidates.push(base_root.join(file_name));
        }
    }
    let mut seen = BTreeSet::new();
    candidates.retain(|path| seen.insert(path.clone()));
    candidates
}

fn object_preview_internal_path_candidates(
    definition: &ObjectPreviewDefinition,
    runtime_resource_path: &str,
) -> Vec<String> {
    let normalized = runtime_resource_path.replace('\\', "/");
    let normalized = normalized.trim_start_matches('/');
    let relative = Path::new(normalized);
    if normalized.is_empty()
        || !relative
            .components()
            .all(|component| matches!(component, Component::Normal(_)))
    {
        return Vec::new();
    }

    let mut candidates = Vec::new();
    if let Some(archive_stem) = runtime_archive_stem(&definition.runtime_archive_path) {
        let archive_prefix = format!("{archive_stem}/");
        if let Some(internal_path) = normalized.strip_prefix(&archive_prefix) {
            candidates.push(internal_path.to_string());
        }
    }
    candidates.push(normalized.to_string());
    candidates.sort();
    candidates.dedup();
    candidates
}

fn runtime_archive_stem(runtime_archive_path: &str) -> Option<String> {
    let normalized = runtime_archive_path.replace('\\', "/");
    Path::new(&normalized)
        .file_stem()
        .and_then(|stem| stem.to_str())
        .map(ToOwned::to_owned)
}

fn preview_source_label(definition: &ObjectPreviewDefinition) -> String {
    if definition.source_files.is_empty() {
        "generated object-preview metadata".to_string()
    } else {
        definition.source_files.join(", ")
    }
}
