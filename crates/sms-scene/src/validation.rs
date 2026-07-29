use std::collections::{BTreeMap, BTreeSet};

use super::{
    validate_project_relative_path, validate_stage_id, StageDocument, StageResourceDocument,
    ValidationIssue, SHINE_QUICK_CAMERA_NAME,
};

const SHINE_CAMERA_RESOURCE_PATHS: &[&[u8]] = &[b"map/scene.bin", b"map/tables.bin"];

fn has_named_record(record: &sms_formats::JDramaRecord, type_name: &str, name: &str) -> bool {
    if record.type_name.rsplit("::").next() == Some(type_name) && record.name == name {
        return true;
    }
    let sms_formats::JDramaRecordPayload::Group { children, .. } = &record.payload else {
        return false;
    };
    children
        .iter()
        .any(|child| has_named_record(child, type_name, name))
}

fn validate_quick_shine_camera(document: &StageDocument) -> Option<ValidationIssue> {
    let mut inspected_placement_resource = false;
    for raw_path in SHINE_CAMERA_RESOURCE_PATHS {
        match document.effective_resource_clone(raw_path) {
            Ok(Some(StageResourceDocument::Placement(resource))) => {
                inspected_placement_resource = true;
                if has_named_record(&resource.root, "CameraMapInfo", SHINE_QUICK_CAMERA_NAME) {
                    return None;
                }
            }
            Ok(Some(_)) => {
                return Some(ValidationIssue::error(
                    "invalid-shine-quick-camera-resource",
                    format!(
                        "Quick-appearance Shines require {} to be placement data",
                        String::from_utf8_lossy(raw_path)
                    ),
                ));
            }
            Ok(None) => {}
            Err(error) => {
                return Some(ValidationIssue::error(
                    "invalid-shine-quick-camera-resource",
                    format!(
                        "Could not inspect {} for the quick-appearance Shine camera: {error}",
                        String::from_utf8_lossy(raw_path)
                    ),
                ));
            }
        }
    }
    Some(ValidationIssue::error(
        "missing-shine-quick-camera",
        if inspected_placement_resource {
            format!(
                "Quick-appearance Shines require retail CameraMapInfo {:?}; reopen the stage so the object catalog can repair its runtime dependencies",
                SHINE_QUICK_CAMERA_NAME
            )
        } else {
            "Quick-appearance Shines require a camera table, but neither map/scene.bin nor map/tables.bin is available"
                .to_string()
        },
    ))
}
fn validate_runtime_actor_links(document: &StageDocument, issues: &mut Vec<ValidationIssue>) {
    let by_id = document
        .objects
        .iter()
        .map(|object| (object.id.as_str(), object))
        .collect::<BTreeMap<_, _>>();
    let mut runtime_names = BTreeMap::<&str, &str>::new();
    let mut target_names = BTreeMap::<&str, &str>::new();

    for owner in &document.objects {
        for reference in &owner.runtime_references {
            let Some(target_id) = reference.target_object_id.as_deref() else {
                if reference.required {
                    issues.push(ValidationIssue::error(
                        "missing-runtime-actor-link",
                        format!(
                            "{} requires a {} actor for runtime lookup {:?}; place one and select it in Runtime Links",
                            owner.id, reference.required_factory_name, reference.runtime_name
                        ),
                    ));
                }
                continue;
            };
            let Some(target) = by_id.get(target_id) else {
                issues.push(ValidationIssue::error(
                    "missing-runtime-actor-target",
                    format!(
                        "{} runtime lookup {:?} references missing object {}",
                        owner.id, reference.runtime_name, target_id
                    ),
                ));
                continue;
            };
            if target.factory_name != reference.required_factory_name {
                issues.push(ValidationIssue::error(
                    "incompatible-runtime-actor-target",
                    format!(
                        "{} runtime lookup {:?} requires {}, but {} is {}",
                        owner.id,
                        reference.runtime_name,
                        reference.required_factory_name,
                        target.id,
                        target.factory_name
                    ),
                ));
            }
            if let Some(existing_name) =
                target_names.insert(target.id.as_str(), reference.runtime_name.as_str())
            {
                if existing_name != reference.runtime_name {
                    issues.push(ValidationIssue::error(
                        "conflicting-runtime-actor-name",
                        format!(
                            "{} is assigned incompatible runtime names {:?} and {:?}",
                            target.id, existing_name, reference.runtime_name
                        ),
                    ));
                }
            }
            if let Some(existing_target) =
                runtime_names.insert(reference.runtime_name.as_str(), target.id.as_str())
            {
                if existing_target != target.id {
                    issues.push(ValidationIssue::error(
                        "duplicate-runtime-actor-name",
                        format!(
                            "runtime lookup {:?} is assigned to both {} and {}",
                            reference.runtime_name, existing_target, target.id
                        ),
                    ));
                }
            }
        }
    }
}

fn source_route_graph_name(
    document: &StageDocument,
    object: &super::SceneObject,
) -> Option<String> {
    let address = object.placement.as_ref()?.source_address()?;
    let StageResourceDocument::Placement(placements) = document
        .stage_archive
        .as_ref()?
        .resource(&address.raw_resource_path)?
    else {
        return None;
    };
    let record = super::jdrama_record_at(&placements.root, &address.record_path)?;
    super::editable_object_parameters(record)
        .ok()?
        .into_iter()
        .find(|parameter| parameter.key == "graph_name")
        .map(|parameter| parameter.raw_value)
}

fn route_reference_requires_named_graph(
    object: &super::SceneObject,
    source_graph_name: Option<&str>,
) -> bool {
    // Sunshine deliberately maps unknown graph names to TGraphGroup's
    // <nullrail> dummy. Retail placements use that behavior for stationary
    // actors (for example dolpic10's NPCMonteMA named monte3), so a pristine
    // source value is not a dangling reference. An editor-authored assignment
    // is expected to name a real graph and remains an export-blocking error.
    let current = object.raw_param("graph_name");
    object
        .raw_params
        .get("graph_name")
        .is_some_and(super::SceneParameter::is_dirty)
        || matches!(object.placement, Some(super::PlacementBinding::Authored(_)))
        || source_graph_name.is_none_or(|source| current != Some(source))
}

fn validate_routes(document: &StageDocument, issues: &mut Vec<ValidationIssue>) {
    let names = if let Some(routes) = document.route_authoring.as_ref() {
        if let Err(error) = routes.compile() {
            issues.push(ValidationIssue::error(
                "route-compile-failed",
                format!("Route export is blocked: {error}"),
            ));
        }
        let mut names = BTreeSet::new();
        for graph in &routes.graphs {
            if !names.insert(graph.name.as_str()) {
                issues.push(ValidationIssue::error(
                    "duplicate-route-name",
                    format!("Route name {:?} is duplicated", graph.name),
                ));
            }
            if graph.controls.is_empty() {
                continue;
            }
            let mut adjacency = BTreeMap::<&str, Vec<&str>>::new();
            for control in &graph.controls {
                adjacency.entry(control.id.as_str()).or_default();
            }
            for link in &graph.links {
                adjacency
                    .entry(link.from.as_str())
                    .or_default()
                    .push(link.to.as_str());
                adjacency
                    .entry(link.to.as_str())
                    .or_default()
                    .push(link.from.as_str());
            }
            let mut visited = BTreeSet::new();
            let mut pending = vec![graph.controls[0].id.as_str()];
            while let Some(id) = pending.pop() {
                if visited.insert(id) {
                    pending.extend(adjacency.get(id).into_iter().flatten().copied());
                }
            }
            if visited.len() != graph.controls.len() {
                issues.push(ValidationIssue::warning(
                    "disconnected-route",
                    format!(
                        "Route {:?} has {} disconnected control point(s)",
                        graph.name,
                        graph.controls.len() - visited.len()
                    ),
                ));
            }
            if graph.name.starts_with("S_")
                && adjacency.values().any(|neighbors| neighbors.len() > 2)
            {
                issues.push(ValidationIssue::warning(
                    "invalid-automatic-spline-topology",
                    format!(
                        "Route {:?} is interpreted by Sunshine as an ordered automatic spline but contains a branch",
                        graph.name
                    ),
                ));
            }
        }
        Some(
            names
                .into_iter()
                .map(str::to_string)
                .collect::<BTreeSet<_>>(),
        )
    } else {
        match document.effective_resource_clone(super::ROUTE_RESOURCE_PATH) {
            Ok(Some(StageResourceDocument::Rail(routes))) => Some(
                routes
                    .graphs
                    .into_iter()
                    .map(|graph| graph.name)
                    .collect::<BTreeSet<_>>(),
            ),
            Ok(Some(_)) => {
                issues.push(ValidationIssue::error(
                    "invalid-route-resource",
                    "map/scene.ral is not a typed RAL resource",
                ));
                None
            }
            Ok(None) => Some(BTreeSet::new()),
            Err(error) => {
                issues.push(ValidationIssue::error(
                    "route-resource-check-failed",
                    format!("Could not inspect the effective route resource: {error}"),
                ));
                None
            }
        }
    };
    let Some(names) = names else {
        return;
    };
    for object in &document.objects {
        let Some(graph_name) = object.raw_param("graph_name") else {
            continue;
        };
        if graph_name == "(null)" || graph_name.is_empty() {
            continue;
        }
        if !names.contains(graph_name) {
            let source_graph_name = source_route_graph_name(document, object);
            if !route_reference_requires_named_graph(object, source_graph_name.as_deref()) {
                continue;
            }
            issues.push(ValidationIssue::error(
                "missing-route-reference",
                format!(
                    "Object {} references missing route {:?}",
                    object.id, graph_name
                ),
            ));
            continue;
        }
        if let Some(graph) = document
            .route_authoring
            .as_ref()
            .and_then(|routes| routes.graph_by_name(graph_name))
        {
            if let Some(distance) = graph.nearest_control_distance(object.transform.translation) {
                if distance > 5000.0 {
                    issues.push(ValidationIssue::warning(
                        "distant-route-start",
                        format!(
                            "Object {} is {:.0} units from its nearest starting node on {:?}",
                            object.id, distance, graph_name
                        ),
                    ));
                }
            }
        }
    }
}

fn goop_runtime_payload_issue(
    estimate: crate::GoopRuntimePayloadEstimate,
) -> Option<ValidationIssue> {
    let total = estimate.total_bytes();
    let message = || {
        format!(
            "{} generated goop regions use an estimated {:.2} MiB of runtime pollution resources ({:.2} MiB models, {:.2} MiB masks). Sunshine has a fixed stage heap; reduce or merge regions before Dolphin testing.",
            estimate.generated_layer_count,
            total as f64 / (1024.0 * 1024.0),
            estimate.model_bytes as f64 / (1024.0 * 1024.0),
            estimate.bitmap_bytes as f64 / (1024.0 * 1024.0),
        )
    };
    if total >= crate::GOOP_RUNTIME_PAYLOAD_ERROR_BYTES {
        Some(ValidationIssue::error(
            "unsafe-goop-runtime-payload",
            format!(
                "{} Build is blocked because this exceeds Graffito's conservative {:.0} MiB generated-goop safety limit and can prevent the stage from booting.",
                message(),
                crate::GOOP_RUNTIME_PAYLOAD_ERROR_BYTES as f64 / (1024.0 * 1024.0),
            ),
        ))
    } else if total >= crate::GOOP_RUNTIME_PAYLOAD_WARNING_BYTES {
        Some(ValidationIssue::warning(
            "large-goop-runtime-payload",
            message(),
        ))
    } else {
        None
    }
}

fn goop_stage_heap_issue(estimate: crate::GoopStageHeapEstimate) -> Option<ValidationIssue> {
    let total = estimate.total_bytes();
    let message = || {
        format!(
            "Estimated Sunshine stage-heap pressure is {:.2} MiB: {:.2} MiB decompressed base archive, {:.2} MiB generated archive resources, and {:.2} MiB generated J3D runtime allowance.",
            total as f64 / (1024.0 * 1024.0),
            estimate.base_archive_bytes as f64 / (1024.0 * 1024.0),
            estimate.generated_archive_bytes as f64 / (1024.0 * 1024.0),
            estimate.j3d_runtime_bytes as f64 / (1024.0 * 1024.0),
        )
    };
    if total >= crate::GOOP_STAGE_HEAP_ERROR_BYTES {
        Some(ValidationIssue::warning(
            "high-goop-stage-heap-risk",
            format!(
                "{} This exceeds Graffito's conservative {:.2} MiB threshold and can trigger a JKRHeap abort or freeze the stage on a black screen.",
                message(),
                crate::GOOP_STAGE_HEAP_ERROR_BYTES as f64 / (1024.0 * 1024.0),
            ),
        ))
    } else if total >= crate::GOOP_STAGE_HEAP_WARNING_BYTES {
        Some(ValidationIssue::warning(
            "low-goop-stage-heap-headroom",
            format!(
                "{} Only {:.2} MiB remains before Graffito's conservative build limit.",
                message(),
                estimate.remaining_bytes() as f64 / (1024.0 * 1024.0),
            ),
        ))
    } else {
        None
    }
}

pub(super) fn validate_document(document: &StageDocument) -> Vec<ValidationIssue> {
    let mut issues = document.load_issues.clone();
    validate_runtime_actor_links(document, &mut issues);
    validate_routes(document, &mut issues);
    issues.extend(super::dialogue_authoring::validate_dialogue_document(
        document,
    ));
    if let Some(goop) = &document.goop_authoring {
        let capacity = crate::goop_runtime_layer_capacity(&document.stage_id);
        if goop.layers.len() > capacity {
            issues.push(ValidationIssue::error(
                "too-many-runtime-goop-layers",
                format!(
                    "{} supports at most {capacity} runtime goop layers, got {}",
                    document.stage_id,
                    goop.layers.len()
                ),
            ));
        }
        if let Err(error) = goop.validate() {
            issues.push(ValidationIssue::error(
                "invalid-goop-authoring",
                error.to_string(),
            ));
        }
        if let Some(issue) = goop_runtime_payload_issue(crate::estimate_goop_runtime_payload(goop))
        {
            issues.push(issue);
        }
        match document.estimate_goop_stage_heap() {
            Ok(Some(estimate)) => {
                if let Some(issue) = goop_stage_heap_issue(estimate) {
                    issues.push(issue);
                }
            }
            Ok(None) => {}
            Err(error) => issues.push(ValidationIssue::warning(
                "goop-stage-heap-estimate-failed",
                format!("Could not estimate Sunshine stage-heap pressure: {error}"),
            )),
        }
    }

    if !document.base_root.exists() {
        issues.push(ValidationIssue::error(
            "missing-base-root",
            format!("Base root does not exist: {}", document.base_root.display()),
        ));
    }

    if document.assets.is_empty() {
        issues.push(ValidationIssue::warning(
            "no-stage-assets",
            format!("No assets found for stage '{}'", document.stage_id),
        ));
    }

    if document.lighting.object_lighting_uses_ordinal_fallback() {
        issues.push(ValidationIssue::warning(
            "ordinal-object-lighting-fallback",
            "Object lighting was selected by retail table position because semantic runtime names were unavailable",
        ));
    }

    if validate_stage_id(&document.stage_id).is_err() {
        issues.push(ValidationIssue::error(
            "invalid-stage-id",
            format!(
                "Stage id '{}' is not safe for project output",
                document.stage_id
            ),
        ));
    }

    for path in document.changed_files.keys() {
        if validate_project_relative_path(path).is_err() {
            issues.push(ValidationIssue::error(
                "unsafe-project-path",
                format!("Changed file path is unsafe: {}", path.display()),
            ));
        }
    }

    let mut object_ids = BTreeSet::new();
    let mut authored_shines_by_flag = BTreeMap::<i32, Vec<String>>::new();
    let runtime_target_ids = document
        .objects
        .iter()
        .flat_map(|owner| owner.runtime_references.iter())
        .filter_map(|reference| reference.target_object_id.as_deref())
        .collect::<BTreeSet<_>>();

    let mut has_quick_authored_shine = false;
    for object in &document.objects {
        if object.id.trim().is_empty() {
            issues.push(ValidationIssue::error(
                "empty-object-id",
                "Scene objects must have a non-empty id",
            ));
        }
        if !object_ids.insert(object.id.as_str()) {
            issues.push(ValidationIssue::error(
                "duplicate-object-id",
                format!("Object id '{}' is duplicated", object.id),
            ));
        }
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
        if object
            .transform
            .scale
            .iter()
            .any(|value| value.abs() <= f32::EPSILON)
        {
            issues.push(ValidationIssue::warning(
                "zero-scale",
                format!("Object {} has a non-invertible scale", object.id),
            ));
        }

        if let Some(registry) = &document.registry {
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

        let is_authored_shine = matches!(
            &object.placement,
            Some(super::PlacementBinding::Authored(authored))
                if authored.prototype.type_name.rsplit("::").next() == Some("Shine")
        );
        if !is_authored_shine {
            continue;
        }

        match object.raw_param("collection_type") {
            Some("normal") => {}
            Some("quickly") => has_quick_authored_shine = true,
            Some(_) if runtime_target_ids.contains(object.id.as_str()) => {}
            Some(mode) => issues.push(ValidationIssue::warning(
                "shine-requires-external-trigger",
                format!(
                    "Authored Shine '{}' uses collection_type '{mode}', so Sunshine creates it dormant until an external event triggers it; use 'normal' for an immediately visible standalone Shine",
                    object.id
                ),
            )),
            None => issues.push(ValidationIssue::warning(
                "missing-shine-collection-type",
                format!(
                    "Authored Shine '{}' has no collection_type; use 'normal' for an immediately visible standalone Shine",
                    object.id
                ),
            )),
        }

        match object
            .raw_param("shine_id")
            .and_then(|value| value.parse::<i32>().ok())
        {
            Some(shine_id @ -1..=119) => {
                let effective_flag = if shine_id == -1 { 0 } else { shine_id };
                authored_shines_by_flag
                    .entry(effective_flag)
                    .or_default()
                    .push(object.id.clone());
            }
            Some(shine_id) => issues.push(ValidationIssue::warning(
                "invalid-shine-id",
                format!(
                    "Authored Shine '{}' has shine_id {shine_id}; use -1 or 0 through 119 (the runtime folds -1/120+ onto flag 0)",
                    object.id
                ),
            )),
            None => issues.push(ValidationIssue::warning(
                "invalid-shine-id",
                format!(
                    "Authored Shine '{}' has no valid integer shine_id; use -1 or 0 through 119",
                    object.id
                ),
            )),
        }

        match object
            .raw_param("in_stage")
            .and_then(|value| value.parse::<i32>().ok())
        {
            Some(-1 | 0) => {}
            Some(in_stage) => issues.push(ValidationIssue::warning(
                "invalid-shine-camera-mode",
                format!(
                    "Authored Shine '{}' has in_stage {in_stage}; use -1 for the outside collection camera or 0 for the inside camera",
                    object.id
                ),
            )),
            None => issues.push(ValidationIssue::warning(
                "invalid-shine-camera-mode",
                format!(
                    "Authored Shine '{}' has no valid integer in_stage; use -1 for outside or 0 for inside",
                    object.id
                ),
            )),
        }
    }

    if has_quick_authored_shine {
        if let Some(issue) = validate_quick_shine_camera(document) {
            issues.push(issue);
        }
    }

    for (shine_flag, object_ids) in authored_shines_by_flag {
        if object_ids.len() > 1 {
            issues.push(ValidationIssue::warning(
                "duplicate-authored-shine-id",
                format!(
                    "Authored Shines {} share persistent Shine flag {shine_flag}; collecting one will mark all of them collected",
                    object_ids.join(", ")
                ),
            ));
        }
    }

    issues
}

#[cfg(test)]
mod tests {
    use super::{
        goop_runtime_payload_issue, goop_stage_heap_issue, route_reference_requires_named_graph,
    };
    use crate::{
        AuthoredPlacement, GoopRuntimePayloadEstimate, GoopStageHeapEstimate, PlacementBinding,
        SceneObject, ValidationSeverity, GOOP_RUNTIME_PAYLOAD_ERROR_BYTES,
        GOOP_RUNTIME_PAYLOAD_WARNING_BYTES, GOOP_STAGE_HEAP_ERROR_BYTES,
        GOOP_STAGE_HEAP_WARNING_BYTES,
    };
    use sms_formats::{JDramaRecord, JDramaRecordPayload};

    #[test]
    fn pristine_retail_dummy_route_is_not_a_required_reference() {
        let mut object = SceneObject::new("retail", "NPCMonteMA");
        object.insert_source_raw_param("graph_name", "monte3");
        assert!(!route_reference_requires_named_graph(
            &object,
            Some("monte3")
        ));

        object.insert_source_raw_param("graph_name", "missing-authored-route");
        assert!(route_reference_requires_named_graph(
            &object,
            Some("monte3")
        ));

        object.set_raw_param("graph_name", "missing-authored-route");
        assert!(route_reference_requires_named_graph(
            &object,
            Some("monte3")
        ));
    }

    #[test]
    fn authored_placement_requires_its_named_route() {
        let mut object = SceneObject::new("authored", "NPCMonteMA");
        object.insert_source_raw_param("graph_name", "missing-authored-route");
        object.placement = Some(PlacementBinding::Authored(AuthoredPlacement {
            raw_resource_path: b"map/scene.bin".to_vec(),
            target_group_index: 0,
            prototype: JDramaRecord::new("Group", "Group", JDramaRecordPayload::Empty).unwrap(),
            dependencies: Vec::new(),
        }));
        assert!(route_reference_requires_named_graph(&object, None));
    }

    #[test]
    fn generated_goop_payload_warns_before_blocking_unsafe_builds() {
        let warning = goop_runtime_payload_issue(GoopRuntimePayloadEstimate {
            generated_layer_count: 4,
            model_bytes: GOOP_RUNTIME_PAYLOAD_WARNING_BYTES,
            bitmap_bytes: 0,
        })
        .unwrap();
        assert_eq!(warning.severity, ValidationSeverity::Warning);
        assert_eq!(warning.code, "large-goop-runtime-payload");
        assert!(warning.message.contains("fixed stage heap"));

        let error = goop_runtime_payload_issue(GoopRuntimePayloadEstimate {
            generated_layer_count: 7,
            model_bytes: GOOP_RUNTIME_PAYLOAD_ERROR_BYTES,
            bitmap_bytes: 0,
        })
        .unwrap();
        assert_eq!(error.severity, ValidationSeverity::Error);
        assert_eq!(error.code, "unsafe-goop-runtime-payload");
        assert!(error.message.contains("prevent the stage from booting"));
    }

    #[test]
    fn whole_stage_heap_estimate_warns_before_jkrheap_abort() {
        let warning = goop_stage_heap_issue(GoopStageHeapEstimate {
            generated_layer_count: 1,
            base_archive_bytes: GOOP_STAGE_HEAP_WARNING_BYTES,
            generated_archive_bytes: 0,
            j3d_runtime_bytes: 0,
        })
        .unwrap();
        assert_eq!(warning.severity, ValidationSeverity::Warning);
        assert_eq!(warning.code, "low-goop-stage-heap-headroom");
        assert!(warning.message.contains("decompressed base archive"));

        let error = goop_stage_heap_issue(GoopStageHeapEstimate {
            generated_layer_count: 1,
            base_archive_bytes: GOOP_STAGE_HEAP_ERROR_BYTES,
            generated_archive_bytes: 0,
            j3d_runtime_bytes: 0,
        })
        .unwrap();
        assert_eq!(error.severity, ValidationSeverity::Warning);
        assert_eq!(error.code, "high-goop-stage-heap-risk");
        assert!(error.message.contains("JKRHeap"));
        assert!(error.message.contains("black screen"));
    }
}
