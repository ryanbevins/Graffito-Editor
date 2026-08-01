//! Spawning Goobles from painted goop.
//!
//! The runtime mechanism is `TConductor::genEnemyFromPollution()`: every
//! `mGenerateTime` frames it picks a `StageEnemyInfo` entry whose flags carry
//! bit `0x1`, looks up that entry's manager *by object name*, rolls a point
//! near Mario, and spawns one of the manager's actors only if that point is
//! polluted. Goop is the condition being tested, not the owner of the setting:
//! the flag lives in `map/tables.bin`, and the actor pool is an ordinary
//! `NameKuriManager` record in `map/scene.bin`.
//!
//! The selector therefore edits two things and invents nothing. It flips bit
//! `0x1` on the selected manager's enemy-table entry and, when necessary,
//! copies an exact retail-backed manager/resource bundle discovered through
//! the decomp schema and object-authoring census. The backing actor is marked
//! pool-only, so only its manager reaches the exported world. Measured ground truth:
//! bianco0's only flagged entry is ナメクリマネージャー, flags 1, weight 100,
//! and its manager record is `character_name`/`capacity 20`/`manager_load_value
//! 3` — those exact values are reproduced here.

use super::*;

const TABLES_PATH: &[u8] = b"map/tables.bin";
const SCENE_PATH: &[u8] = b"map/scene.bin";

/// ナメクリマネージャー — the Gooble manager's JDrama object name, which is
/// what `getManagerByName` resolves. The factory name is `NameKuriManager`.
/// Production code no longer special-cases it — any decomp-identified
/// TEnemyManager in the scene can be flagged — so only the tests pin against it.
#[cfg(test)]
const GOOBLE_MANAGER_NAME: &str =
    "\u{30CA}\u{30E1}\u{30AF}\u{30EA}\u{30DE}\u{30CD}\u{30FC}\u{30B8}\u{30E3}\u{30FC}";
/// 敵情報 — every retail StageEnemyInfo entry carries this object name.
const ENEMY_INFO_NAME: &str = "\u{6575}\u{60C5}\u{5831}";
/// 敵出現テーブル — the StageEnemyInfoHeader's object name.
const ENEMY_TABLE_NAME: &str = "\u{6575}\u{51FA}\u{73FE}\u{30C6}\u{30FC}\u{30D6}\u{30EB}";
/// データテーブル群 — the tables.bin root NameRefGrp.
const TABLES_ROOT_NAME: &str = "\u{30C7}\u{30FC}\u{30BF}\u{30C6}\u{30FC}\u{30D6}\u{30EB}\u{7FA4}";

/// Bit 0x1 of `StageEnemyInfo::mFlags`: eligible for pollution generation.
const SPAWN_FROM_GOOP_FLAG: i32 = 0x1;
/// Retail weight on every bianco entry; getMatchedInfo weights entries
/// against each other, so with equal weights all flagged entries are equally
/// likely.
const SPAWN_WEIGHT: u32 = 100;

/// Suffix of the hidden pool that spawns from goop when placed actors exist.
///
/// The conductor recycles every enemy of a flagged manager into goop --
/// retail's "spawns" are its placed enemies being teleported -- so flagging a
/// manager that also has hand-placed actors steals them off their placements.
/// Spawning is instead given an unplaced clone pool under this suffix, and the
/// placed actors' own manager stays unflagged.
const GOOP_POOL_SUFFIX: &str = "_gp";

fn jdrama_field(name: &str, value: sms_formats::JDramaFieldValue) -> sms_formats::JDramaField {
    sms_formats::JDramaField {
        name: name.to_string(),
        value,
    }
}

fn enemy_info_record(manager: &str, flags: i32) -> sms_formats::JDramaRecord {
    sms_formats::JDramaRecord {
        type_name: "StageEnemyInfo".to_string(),
        name: ENEMY_INFO_NAME.to_string(),
        payload: sms_formats::JDramaRecordPayload::Fields {
            fields: vec![
                jdrama_field(
                    "manager_name",
                    sms_formats::JDramaFieldValue::String(manager.to_string()),
                ),
                jdrama_field("flags", sms_formats::JDramaFieldValue::I32(flags)),
                jdrama_field("weight", sms_formats::JDramaFieldValue::U32(SPAWN_WEIGHT)),
            ],
        },
    }
}

fn record_field<'a>(
    record: &'a sms_formats::JDramaRecord,
    name: &str,
) -> Option<&'a sms_formats::JDramaFieldValue> {
    let fields = match &record.payload {
        sms_formats::JDramaRecordPayload::Actor { fields, .. }
        | sms_formats::JDramaRecordPayload::Fields { fields }
        | sms_formats::JDramaRecordPayload::Group { fields, .. } => fields,
        sms_formats::JDramaRecordPayload::Empty => return None,
    };
    fields
        .iter()
        .find(|field| field.name == name)
        .map(|field| &field.value)
}

fn entry_for_manager(record: &sms_formats::JDramaRecord, manager: &str) -> bool {
    record.type_name == "StageEnemyInfo"
        && matches!(
            record_field(record, "manager_name"),
            Some(sms_formats::JDramaFieldValue::String(name)) if name == manager
        )
}

fn entry_spawns_from_goop(record: &sms_formats::JDramaRecord) -> bool {
    matches!(
        record_field(record, "flags"),
        Some(sms_formats::JDramaFieldValue::I32(flags)) if flags & SPAWN_FROM_GOOP_FLAG != 0
    )
}

fn set_entry_spawn_flag(record: &mut sms_formats::JDramaRecord, enabled: bool) {
    let sms_formats::JDramaRecordPayload::Fields { fields } = &mut record.payload else {
        return;
    };
    for field in fields {
        if field.name != "flags" {
            continue;
        }
        if let sms_formats::JDramaFieldValue::I32(flags) = &mut field.value {
            match enabled {
                true => *flags |= SPAWN_FROM_GOOP_FLAG,
                false => *flags &= !SPAWN_FROM_GOOP_FLAG,
            }
        }
    }
}

/// Depth-first search for the first record satisfying `predicate`, returning
/// its child-index path.
fn find_record_path(
    record: &sms_formats::JDramaRecord,
    path: &mut Vec<usize>,
    predicate: &impl Fn(&sms_formats::JDramaRecord) -> bool,
) -> bool {
    if predicate(record) {
        return true;
    }
    if let sms_formats::JDramaRecordPayload::Group { children, .. } = &record.payload {
        for (index, child) in children.iter().enumerate() {
            path.push(index);
            if find_record_path(child, path, predicate) {
                return true;
            }
            path.pop();
        }
    }
    false
}

fn any_record(
    record: &sms_formats::JDramaRecord,
    predicate: &impl Fn(&sms_formats::JDramaRecord) -> bool,
) -> bool {
    find_record_path(record, &mut Vec::new(), predicate)
}

fn record_at_mut<'a>(
    root: &'a mut sms_formats::JDramaRecord,
    path: &[usize],
) -> Option<&'a mut sms_formats::JDramaRecord> {
    let mut record = root;
    for index in path {
        let sms_formats::JDramaRecordPayload::Group { children, .. } = &mut record.payload else {
            return None;
        };
        record = children.get_mut(*index)?;
    }
    Some(record)
}

/// Sets or clears the Gooble spawn flag in a tables document, creating the
/// enemy table and entry when they are absent.
///
/// Pure so it can be tested without an archive. Returns false when nothing
/// changed.
fn apply_spawn_to_tables(
    tables: &mut sms_formats::JDramaDocument,
    manager: &str,
    enabled: bool,
) -> bool {
    let mut entry_path = Vec::new();
    if find_record_path(&tables.root, &mut entry_path, &|record| {
        entry_for_manager(record, manager)
    }) {
        let entry = record_at_mut(&mut tables.root, &entry_path).expect("path just found");
        if entry_spawns_from_goop(entry) == enabled {
            return false;
        }
        set_entry_spawn_flag(entry, enabled);
        return true;
    }
    if !enabled {
        return false;
    }
    // No entry yet. Find or create the header, then append one.
    let mut header_path = Vec::new();
    if !find_record_path(&tables.root, &mut header_path, &|record| {
        record.type_name.ends_with("StageEnemyInfoHeader")
    }) {
        let sms_formats::JDramaRecordPayload::Group { children, .. } = &mut tables.root.payload
        else {
            return false;
        };
        children.push(sms_formats::JDramaRecord {
            type_name: "StageEnemyInfoHeader".to_string(),
            name: ENEMY_TABLE_NAME.to_string(),
            payload: sms_formats::JDramaRecordPayload::Group {
                fields: Vec::new(),
                children: Vec::new(),
            },
        });
        header_path = vec![children.len() - 1];
    }
    let Some(header) = record_at_mut(&mut tables.root, &header_path) else {
        return false;
    };
    let sms_formats::JDramaRecordPayload::Group { children, .. } = &mut header.payload else {
        return false;
    };
    children.push(enemy_info_record(manager, SPAWN_FROM_GOOP_FLAG));
    true
}

/// A tables document for a stage that has none: the retail root with only the
/// enemy table in it. Every other retail table is optional at runtime — blank
/// stages boot with no tables.bin at all.
fn fresh_tables_document() -> sms_formats::JDramaDocument {
    sms_formats::JDramaDocument {
        root: sms_formats::JDramaRecord {
            type_name: "NameRefGrp".to_string(),
            name: TABLES_ROOT_NAME.to_string(),
            payload: sms_formats::JDramaRecordPayload::Group {
                fields: Vec::new(),
                children: Vec::new(),
            },
        },
    }
}

fn semantic_factory_name(type_name: &str) -> &str {
    type_name.rsplit("::").next().unwrap_or(type_name)
}

fn enemy_manager_definition<'a>(
    registry: &'a ObjectRegistry,
    factory_name: &str,
) -> Option<&'a sms_schema::EnemyManagerDefinition> {
    registry
        .find_enemy_manager(factory_name)
        .or_else(|| registry.find_enemy_manager(semantic_factory_name(factory_name)))
}

fn enemy_actor_definition<'a>(
    registry: &'a ObjectRegistry,
    factory_name: &str,
) -> Option<&'a sms_schema::EnemyActorDefinition> {
    registry
        .find_enemy_actor(factory_name)
        .or_else(|| registry.find_enemy_actor(semantic_factory_name(factory_name)))
}

fn manager_factory_matches_actor(
    actor: &sms_schema::EnemyActorDefinition,
    manager_factory: &str,
) -> bool {
    let manager_factory = semantic_factory_name(manager_factory);
    actor
        .manager_factories
        .iter()
        .any(|expected| semantic_factory_name(expected) == manager_factory)
}

fn manager_has_editor_pollution_support(
    registry: &ObjectRegistry,
    manager: &sms_schema::EnemyManagerDefinition,
    manager_name: &str,
) -> bool {
    if registry.enemy_manager_has_native_pollution_pool(&manager.factory_name, manager_name) {
        return true;
    }
    match semantic_factory_name(&manager.factory_name) {
        "PoiHanaManager" => registry
            .conditional_enemy_manager_factories
            .iter()
            .any(|factory| semantic_factory_name(factory) == "PoiHanaManager"),
        "HinoKuri2Manager" => registry
            .conductor_pool_excluded_manager_names
            .iter()
            .any(|excluded| excluded == manager_name),
        _ => false,
    }
}

fn actor_is_pollution_spawn_subject(
    registry: &ObjectRegistry,
    actor: &sms_schema::EnemyActorDefinition,
) -> bool {
    let factory = semantic_factory_name(&actor.factory_name);
    let class = semantic_factory_name(&actor.class_name);
    if factory.ends_with("LaunchPad") || class.ends_with("LaunchPad") {
        return false;
    }
    // Decomp-identified equipment actors (TRocket) are physically attached to
    // Mario's water-gun state rather than autonomous pollution subjects.
    if registry.runtime_name_references.iter().any(|reference| {
        reference.factory_name == actor.factory_name
            && matches!(
                reference.target,
                sms_schema::RuntimeNameReferenceTarget::MarioWaterGun
            )
    }) {
        return false;
    }
    let category = registry
        .find_object(&actor.factory_name)
        .map(|definition| definition.category.as_str());
    if category == Some("Boss")
        && !registry.runtime_name_references.iter().any(|reference| {
            reference.factory_name == actor.factory_name
                && matches!(
                    reference.target,
                    sms_schema::RuntimeNameReferenceTarget::Graph
                )
        })
    {
        return false;
    }
    true
}

fn manager_is_pollution_spawn_subject(
    registry: &ObjectRegistry,
    manager: &sms_schema::EnemyManagerDefinition,
) -> bool {
    if let Some(exact) = manager
        .spawned_actor_class
        .as_deref()
        .and_then(|class_name| {
            registry.enemy_actors.iter().find(|actor| {
                actor.class_name == class_name
                    && manager_factory_matches_actor(actor, &manager.factory_name)
            })
        })
    {
        return actor_is_pollution_spawn_subject(registry, exact);
    }
    registry.enemy_actors.iter().any(|actor| {
        manager_factory_matches_actor(actor, &manager.factory_name)
            && actor_is_pollution_spawn_subject(registry, actor)
    })
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct CatalogEnemyManagerChoice {
    actor_factory: String,
    manager_factory: String,
    manager_name: String,
    display_name: String,
    unavailable_reason: Option<&'static str>,
}

fn goop_enemy_unavailable_reason(
    actor_factory: Option<&str>,
    manager_factory: Option<&str>,
) -> Option<&'static str> {
    (actor_factory == Some("BossManta") || manager_factory == Some("BossMantaManager")).then_some(
        "BossManta currently soft-crashes when its goop pool initializes and does not move after spawning.",
    )
}

fn catalog_enemy_manager_choice(
    template: &sms_scene::ObjectAuthoringTemplate,
    registry: &ObjectRegistry,
) -> Option<CatalogEnemyManagerChoice> {
    if registry
        .find_object(&template.factory_name)
        .is_some_and(|definition| definition.unsafe_to_edit)
    {
        return None;
    }
    let actor = enemy_actor_definition(registry, &template.factory_name)?;
    if !actor_is_pollution_spawn_subject(registry, actor) {
        return None;
    }
    let sms_formats::JDramaFieldValue::String(manager_name) =
        record_field(&template.record, "manager_name")?
    else {
        return None;
    };
    let dependency = template.dependencies.iter().find(|dependency| {
        dependency.record.name == *manager_name
            && enemy_manager_definition(registry, &dependency.record.type_name)
                .is_some_and(|manager| manager_factory_matches_actor(actor, &manager.factory_name))
    })?;
    let manager = enemy_manager_definition(registry, &dependency.record.type_name)?;
    if !manager_has_editor_pollution_support(registry, manager, manager_name) {
        return None;
    }
    Some(CatalogEnemyManagerChoice {
        actor_factory: actor.factory_name.clone(),
        manager_factory: manager.factory_name.clone(),
        manager_name: manager_name.clone(),
        display_name: actor.factory_name.clone(),
        unavailable_reason: goop_enemy_unavailable_reason(
            Some(&actor.factory_name),
            Some(&manager.factory_name),
        ),
    })
}

fn catalog_enemy_manager_choices(
    catalog: &ObjectAuthoringCatalog,
    registry: &ObjectRegistry,
) -> BTreeMap<String, Vec<CatalogEnemyManagerChoice>> {
    let mut compatible = BTreeMap::<String, Vec<CatalogEnemyManagerChoice>>::new();
    for (_, template) in catalog.iter() {
        let Some(choice) = catalog_enemy_manager_choice(template, registry) else {
            continue;
        };
        compatible
            .entry(choice.manager_name.clone())
            .or_default()
            .push(choice);
    }
    let mut choices = BTreeMap::new();
    for (manager_name, mut candidates) in compatible {
        candidates.sort_by(|left, right| left.actor_factory.cmp(&right.actor_factory));
        let manager = candidates
            .first()
            .and_then(|choice| enemy_manager_definition(registry, &choice.manager_factory));
        let exact = manager
            .and_then(|manager| manager.spawned_actor_class.as_deref())
            .and_then(|spawned_class| {
                candidates
                    .iter()
                    .find(|choice| {
                        enemy_actor_definition(registry, &choice.actor_factory)
                            .is_some_and(|actor| actor.class_name == spawned_class)
                    })
                    .cloned()
            });
        let mut selected = exact.into_iter().collect::<Vec<_>>();
        if let Some(red) = candidates
            .iter()
            .find(|choice| choice.actor_factory == "PoiHanaRed")
            .cloned()
        {
            if !selected
                .iter()
                .any(|current| current.actor_factory == red.actor_factory)
            {
                selected.push(red);
            }
        }
        if selected.is_empty() {
            // Some runtime-only manager products have no public factory for
            // their exact base class (TTobiPuku is represented by PukuPuku).
            // Use the least-ambiguous retail carrier tied to that manager.
            candidates.sort_by_key(|choice| {
                enemy_actor_definition(registry, &choice.actor_factory)
                    .map_or(usize::MAX, |actor| actor.manager_factories.len())
            });
            if let Some(mut fallback) = candidates.into_iter().next() {
                if let Some(spawned_class) =
                    manager.and_then(|manager| manager.spawned_actor_class.as_deref())
                {
                    fallback.display_name = spawned_class
                        .strip_prefix('T')
                        .unwrap_or(spawned_class)
                        .to_string();
                }
                selected.push(fallback);
            }
        }
        if !selected.is_empty() {
            choices.insert(manager_name, selected);
        }
    }
    choices
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct EnemyManagerInstance {
    factory_name: String,
    display_name: String,
}

fn enemy_manager_display_name(
    registry: &ObjectRegistry,
    manager: &sms_schema::EnemyManagerDefinition,
) -> String {
    manager
        .spawned_actor_class
        .as_deref()
        .and_then(|class_name| {
            registry.enemy_actors.iter().find(|actor| {
                actor.class_name == class_name
                    && actor
                        .manager_factories
                        .iter()
                        .any(|factory| factory == &manager.factory_name)
            })
        })
        .map_or_else(
            || manager.factory_name.clone(),
            |actor| actor.factory_name.clone(),
        )
}

fn collect_enemy_manager_records(
    record: &sms_formats::JDramaRecord,
    registry: &ObjectRegistry,
    out: &mut BTreeMap<String, EnemyManagerInstance>,
) {
    if let Some(manager) = enemy_manager_definition(registry, &record.type_name) {
        if manager_is_pollution_spawn_subject(registry, manager)
            && manager_has_editor_pollution_support(registry, manager, &record.name)
        {
            out.entry(record.name.clone())
                .or_insert_with(|| EnemyManagerInstance {
                    factory_name: manager.factory_name.clone(),
                    display_name: enemy_manager_display_name(registry, manager),
                });
        }
    }
    if let sms_formats::JDramaRecordPayload::Group { children, .. } = &record.payload {
        for child in children {
            collect_enemy_manager_records(child, registry, out);
        }
    }
}

fn enemy_manager_instances(
    document: &StageDocument,
    registry: &ObjectRegistry,
) -> BTreeMap<String, EnemyManagerInstance> {
    let mut managers = BTreeMap::new();
    if let Ok(Some(StageResourceDocument::Placement(scene))) =
        document.effective_resource_clone(SCENE_PATH)
    {
        collect_enemy_manager_records(&scene.root, registry, &mut managers);
    }
    for object in &document.objects {
        let Some(sms_scene::PlacementBinding::Authored(placement)) = &object.placement else {
            continue;
        };
        for dependency in &placement.dependencies {
            let Some(manager) = enemy_manager_definition(registry, &dependency.record.type_name)
            else {
                continue;
            };
            if !manager_is_pollution_spawn_subject(registry, manager)
                || !manager_has_editor_pollution_support(registry, manager, &dependency.record.name)
            {
                continue;
            }
            managers
                .entry(dependency.record.name.clone())
                .or_insert_with(|| EnemyManagerInstance {
                    factory_name: manager.factory_name.clone(),
                    display_name: enemy_manager_display_name(registry, manager),
                });
        }
    }
    managers
}

/// What each goop layer does to a spawn that lands in it, for the build's
/// runtime patch.
///
/// A styled layer with its own pool routes spawns to that pool; a styled
/// layer without one spawns nothing, so a stage that binds any layer does not
/// silently keep spawning the wrong enemy elsewhere. Unstyled layers are left
/// alone. Returns nothing when no layer has a pool, which keeps the patch off
/// entirely.
pub(super) fn goop_layer_spawn_bindings(
    document: &StageDocument,
) -> BTreeMap<usize, crate::direct_boot::RuntimeGoopLayerBinding> {
    use crate::direct_boot::RuntimeGoopLayerBinding;

    let mut bindings = BTreeMap::new();
    let Some(authoring) = document.goop_authoring.as_ref() else {
        return bindings;
    };
    let mut any_pool = false;
    for (index, layer) in authoring.layers.iter().enumerate() {
        if layer.style_source.is_none() {
            continue;
        }
        let suffix = format!("_L{index:02}");
        let pool = document.objects.iter().find_map(|object| {
            let manager = object.raw_param("manager_name")?;
            manager.ends_with(&suffix).then(|| manager.to_string())
        });
        match pool {
            Some(manager) => {
                any_pool = true;
                bindings.insert(index, RuntimeGoopLayerBinding::Pool(manager));
            }
            None => {
                bindings.insert(index, RuntimeGoopLayerBinding::Empty);
            }
        }
    }
    if !any_pool {
        return BTreeMap::new();
    }
    bindings
}

pub(super) fn goop_flagged_managers(document: &StageDocument) -> BTreeSet<String> {
    let Ok(Some(StageResourceDocument::Placement(tables))) =
        document.effective_resource_clone(TABLES_PATH)
    else {
        return BTreeSet::new();
    };
    fn collect(record: &sms_formats::JDramaRecord, out: &mut BTreeSet<String>) {
        if record.type_name == "StageEnemyInfo" && entry_spawns_from_goop(record) {
            if let Some(sms_formats::JDramaFieldValue::String(manager)) =
                record_field(record, "manager_name")
            {
                out.insert(manager.clone());
            }
        }
        if let sms_formats::JDramaRecordPayload::Group { children, .. } = &record.payload {
            for child in children {
                collect(child, out);
            }
        }
    }
    let mut managers = BTreeSet::new();
    collect(&tables.root, &mut managers);
    managers
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct GoopSpawnEntity {
    display_name: String,
    manager_name: String,
    manager_factory: Option<String>,
    manager_present: bool,
    manager_survives_pool_removal: bool,
    catalog_actor_factory: Option<String>,
    variant_active: bool,
    unavailable_reason: Option<&'static str>,
}

fn active_pool_actor_factory(document: &StageDocument, manager_name: &str) -> Option<String> {
    document.objects.iter().find_map(|object| {
        let sms_scene::PlacementBinding::Authored(placement) = object.placement.as_ref()? else {
            return None;
        };
        (placement.pool_only && object.raw_param("manager_name") == Some(manager_name))
            .then(|| object.factory_name.clone())
    })
}

fn manager_survives_pool_removal(document: &StageDocument, manager_name: &str) -> bool {
    let in_scene = document
        .effective_resource_clone(SCENE_PATH)
        .ok()
        .flatten()
        .and_then(|resource| match resource {
            StageResourceDocument::Placement(scene) => Some(scene),
            _ => None,
        })
        .is_some_and(|scene| any_record(&scene.root, &|record| record.name == manager_name));
    in_scene
        || document.objects.iter().any(|object| {
            let Some(sms_scene::PlacementBinding::Authored(placement)) = &object.placement else {
                return false;
            };
            !placement.pool_only
                && placement
                    .dependencies
                    .iter()
                    .any(|dependency| dependency.record.name == manager_name)
        })
}

impl SmsEditorApp {
    /// Whether the effective enemy table flags Goobles to generate from goop.
    /// Enemy managers this stage could spawn from goop. Existing scene
    /// managers and complete retail-backed manager bundles are both included.
    ///
    /// Catalog candidates are accepted only when the decomp-derived schema
    /// identifies the actor/manager relationship and the retail census has an
    /// exact dependency plus resource closure. Enabling an absent candidate
    /// creates a pool-only authoring handle, so no actor is placed in the
    /// exported world.
    fn goop_spawnable_entities(&self) -> Vec<GoopSpawnEntity> {
        let (Some(document), Some(registry)) = (self.document.as_ref(), self.registry.as_ref())
        else {
            return Vec::new();
        };
        let managers = enemy_manager_instances(document, registry);
        let catalog_choices =
            catalog_enemy_manager_choices(&self.object_authoring_catalog, registry);
        let mut entities = Vec::new();
        let manager_names = managers
            .keys()
            .chain(catalog_choices.keys())
            .cloned()
            .collect::<BTreeSet<_>>();
        for manager_name in manager_names {
            let manager = managers.get(&manager_name);
            let choices = catalog_choices.get(&manager_name);
            let active_factory = active_pool_actor_factory(document, &manager_name);
            if let Some(choices) = choices {
                for (index, choice) in choices.iter().enumerate() {
                    entities.push(GoopSpawnEntity {
                        display_name: choice.display_name.clone(),
                        manager_name: manager_name.clone(),
                        manager_factory: Some(choice.manager_factory.clone()),
                        manager_present: manager.is_some(),
                        manager_survives_pool_removal: manager_survives_pool_removal(
                            document,
                            &manager_name,
                        ),
                        catalog_actor_factory: Some(choice.actor_factory.clone()),
                        variant_active: active_factory
                            .as_deref()
                            .map_or(index == 0, |active| active == choice.actor_factory),
                        unavailable_reason: choice.unavailable_reason,
                    });
                }
            } else if let Some(manager) = manager {
                entities.push(GoopSpawnEntity {
                    display_name: manager.display_name.clone(),
                    manager_name: manager_name.clone(),
                    manager_factory: Some(manager.factory_name.clone()),
                    manager_present: true,
                    manager_survives_pool_removal: true,
                    catalog_actor_factory: None,
                    variant_active: true,
                    unavailable_reason: goop_enemy_unavailable_reason(
                        None,
                        Some(&manager.factory_name),
                    ),
                });
            }
        }
        for manager_name in goop_flagged_managers(document) {
            if !entities
                .iter()
                .any(|entity| entity.manager_name == manager_name)
            {
                entities.push(GoopSpawnEntity {
                    display_name: "Missing enemy manager".to_string(),
                    manager_name,
                    manager_factory: None,
                    manager_present: false,
                    manager_survives_pool_removal: false,
                    catalog_actor_factory: None,
                    variant_active: true,
                    unavailable_reason: None,
                });
            }
        }
        entities.sort_by(|left, right| {
            left.display_name
                .cmp(&right.display_name)
                .then_with(|| left.manager_name.cmp(&right.manager_name))
        });
        entities
    }

    /// Whether the effective enemy table flags `manager` for goop spawning.
    pub(super) fn manager_spawns_from_goop(&self, manager: &str) -> bool {
        let Some(document) = self.document.as_ref() else {
            return false;
        };
        let Ok(Some(StageResourceDocument::Placement(tables))) =
            document.effective_resource_clone(TABLES_PATH)
        else {
            return false;
        };
        let spawn_pool = format!("{manager}{GOOP_POOL_SUFFIX}");
        any_record(&tables.root, &|record| {
            (entry_for_manager(record, manager) || entry_for_manager(record, &spawn_pool))
                && entry_spawns_from_goop(record)
        })
    }

    /// Whether any hand-placed (non-carrier) actor is bound to `manager`.
    fn placed_actors_use_manager(&self, manager: &str) -> bool {
        self.document.as_ref().is_some_and(|document| {
            document.objects.iter().any(|object| {
                !object.is_pool_only() && object.raw_param("manager_name") == Some(manager)
            })
        })
    }

    /// Flips a manager's spawn flag in the enemy table. If the manager is a
    /// catalog-backed candidate rather than an existing stage record, its
    /// complete pool-only authoring bundle is added in the same undo step.
    fn set_manager_spawns_from_goop(&mut self, entity: &GoopSpawnEntity, enabled: bool) {
        let Some(document) = self.document.as_ref() else {
            return;
        };
        let before_objects = document.objects.clone();
        let before_archive_edits = document.archive_edits.clone();
        let before_object_serial = self.next_object_serial;
        let mut pool_log = Vec::new();

        let spawn_pool = format!("{}{GOOP_POOL_SUFFIX}", entity.manager_name);
        let result: Result<(), String> = (|| {
            if enabled && self.placed_actors_use_manager(&entity.manager_name) {
                // Placed actors must keep standing where they were put, so the
                // goop flag goes onto an unplaced clone pool instead of the
                // manager that owns the placements.
                let actor_factory = entity.catalog_actor_factory.as_deref().ok_or_else(|| {
                    format!(
                        "manager {:?} has hand-placed actors but no retail-backed bundle to                          clone for goop spawning",
                        entity.manager_name
                    )
                })?;
                let manager_factory = entity.manager_factory.as_deref().ok_or_else(|| {
                    format!(
                        "manager {:?} has no decomp-derived factory",
                        entity.manager_name
                    )
                })?;
                let clone_exists = self.document.as_ref().is_some_and(|document| {
                    document
                        .objects
                        .iter()
                        .any(|object| object.raw_param("manager_name") == Some(spawn_pool.as_str()))
                });
                if clone_exists {
                    let repaired = self.ensure_cloned_manager_pool_resources(
                        actor_factory,
                        &entity.manager_name,
                        GOOP_POOL_SUFFIX,
                    )?;
                    if repaired > 0 {
                        pool_log.push(format!(
                            "Restored {repaired} missing model resource(s) for the goop spawn                              pool."
                        ));
                    }
                } else {
                    pool_log.push(self.ensure_cloned_enemy_manager_pool(
                        actor_factory,
                        manager_factory,
                        &entity.manager_name,
                        GOOP_POOL_SUFFIX,
                    )?);
                }
                self.apply_manager_spawn(&spawn_pool, true)?;
                // A project flagged before this arrangement existed migrates:
                // the placed actors' manager gives the flag up to the clone.
                if goop_flagged_managers(self.document.as_ref().expect("document checked"))
                    .contains(&entity.manager_name)
                {
                    self.apply_manager_spawn(&entity.manager_name, false)?;
                    pool_log.push(format!(
                        "Moved the goop flag off {:?} so its placed actors keep their                          placements; spawning now comes from {spawn_pool:?}.",
                        entity.manager_name
                    ));
                }
                return Ok(());
            }
            if enabled {
                if let Some(actor_factory) = entity.catalog_actor_factory.as_deref() {
                    let manager_factory = entity.manager_factory.as_deref().ok_or_else(|| {
                        format!(
                            "manager {:?} has no decomp-derived factory",
                            entity.manager_name
                        )
                    })?;
                    let matching_pool_exists = self.document.as_ref().is_some_and(|document| {
                        document.objects.iter().any(|object| {
                            matches!(
                                &object.placement,
                                Some(sms_scene::PlacementBinding::Authored(placement))
                                    if placement.pool_only
                            ) && object.raw_param("manager_name") == Some(&entity.manager_name)
                                && object.factory_name == actor_factory
                        })
                    });
                    let needs_variant_carrier = actor_factory == "PoiHanaRed";
                    if !matching_pool_exists {
                        if let Some(document) = self.document.as_mut() {
                            document.objects.retain(|object| {
                                !matches!(
                                    &object.placement,
                                    Some(sms_scene::PlacementBinding::Authored(placement))
                                        if placement.pool_only
                                            && object.raw_param("manager_name") == Some(&entity.manager_name)
                                )
                            });
                        }
                        if !entity.manager_survives_pool_removal || needs_variant_carrier {
                            pool_log.push(self.ensure_catalog_enemy_manager_pool(
                                actor_factory,
                                manager_factory,
                                &entity.manager_name,
                            )?);
                        }
                    }
                } else if !entity.manager_present {
                    return Err(format!(
                        "manager {:?} has no safe retail-backed pool bundle",
                        entity.manager_name
                    ));
                }
            }
            self.apply_manager_spawn(&entity.manager_name, enabled)?;
            if !enabled {
                // Clearing a spawn-pool entry that never existed is a no-op.
                self.apply_manager_spawn(&spawn_pool, false)?;
                pool_log.extend(self.cleanup_unused_goop_manager_pools());
            }
            Ok(())
        })();

        if let Err(error) = result {
            if let Some(document) = self.document.as_mut() {
                document.objects = before_objects;
                document.archive_edits = before_archive_edits;
            }
            self.next_object_serial = before_object_serial;
            self.log
                .push(format!("Could not update goop spawning: {error}"));
            return;
        }

        let (record, dirty, added_pool) = {
            let Some(document) = self.document.as_ref() else {
                return;
            };
            (
                ObjectUndoRecord::between(
                    &before_objects,
                    &document.objects,
                    &before_archive_edits,
                    &document.archive_edits,
                ),
                stage_document_differs_from_saved(
                    document,
                    &self.saved_objects,
                    &self.saved_lighting,
                    &self.saved_death_barrier,
                    &self.saved_archive_edits,
                    &self.saved_dialogue_authoring,
                    &self.saved_dialogue_library,
                ),
                document.objects.len() != before_objects.len(),
            )
        };
        if !record.is_empty() {
            self.push_undo_record(record);
        }
        self.document_dirty = dirty;
        self.flush_document_change();
        if added_pool {
            self.rebuild_model_preview_from_document_async();
        }
        for message in pool_log {
            self.log.push(message);
        }
        self.log.push(match enabled {
            true => format!(
                "{} now spawns its enemies from painted goop.",
                entity.manager_name
            ),
            false => format!("{} no longer spawns from goop.", entity.manager_name),
        });
    }

    fn apply_manager_spawn(&mut self, manager: &str, enabled: bool) -> Result<(), String> {
        if enabled {
            let registry = self
                .registry
                .as_ref()
                .ok_or_else(|| "enemy schema is unavailable".to_string())?;
            let document = self
                .document
                .as_ref()
                .ok_or_else(|| "no stage is open".to_string())?;
            if !enemy_manager_instances(document, registry).contains_key(manager) {
                return Err(format!(
                    "{manager:?} is not a decomp-identified TEnemyManager in the effective scene"
                ));
            }
        }
        let document = self
            .document
            .as_mut()
            .ok_or_else(|| "no stage is open".to_string())?;

        // Editing the effective document keeps any audio-cube edits already
        // upserted into the same file.
        let mut tables = match document
            .effective_resource_clone(TABLES_PATH)
            .map_err(|error| error.to_string())?
        {
            Some(StageResourceDocument::Placement(tables)) => tables,
            Some(_) => return Err("map/tables.bin is not typed placement data".to_string()),
            None => fresh_tables_document(),
        };
        if apply_spawn_to_tables(&mut tables, manager, enabled) {
            document.archive_edits.upsert_resource(
                TABLES_PATH.to_vec(),
                StageResourceDocument::Placement(tables),
            );
        }

        // An earlier revision inserted a bare NameKuriManager record here. A
        // manager is not just a record: TObjManager::load resolves its
        // character archive by name and dereferences the result, so a manager
        // without its resources crashes the stage on load. Strip any bare
        // insert that revision left behind, so an affected stage heals.
        document.archive_edits.placement_inserts.retain(|insert| {
            !(insert.raw_resource_path == SCENE_PATH
                && insert.record.type_name == "NameKuriManager"
                && matches!(
                    &insert.record.payload,
                    sms_formats::JDramaRecordPayload::Fields { .. }
                ))
        });
        Ok(())
    }

    /// Bakes the goop stain into every copied model that carries the stain's
    /// dummy texture slot.
    ///
    /// The runtime decides the stain per frame -- the JP source keys it on a
    /// texture lookup, and the US build observably applies a further condition
    /// the decomp does not show -- so a custom stage cannot rely on it. Baking
    /// writes the stain texture into the model's `H_ma_rak_dummy` slot and
    /// pins the blend to a constant one-half, which is the same 0x80 the
    /// runtime uses when it shows the effect. After that there is nothing left
    /// for the runtime to decide. Undo reverses it.
    /// Whether a top-level archive folder belongs to a per-layer pool clone.
    ///
    /// Pool folders are the original folder plus the lowercased layer suffix
    /// ("hamukuri" -> "hamukuri_l00"). Stage-wide stain actions skip them:
    /// each pool's look belongs to its layer, not to the stage toggle.
    fn is_layer_pool_folder(segment: &str) -> bool {
        let bytes = segment.as_bytes();
        bytes.len() > 4
            && bytes[bytes.len() - 4..bytes.len() - 2] == *b"_l"
            && bytes[bytes.len() - 2..]
                .iter()
                .all(|byte| byte.is_ascii_digit())
    }

    fn outside_layer_pool_folders(path: &str) -> bool {
        match path.split('/').next() {
            Some(segment) => !Self::is_layer_pool_folder(segment),
            None => true,
        }
    }

    /// Bakes `stain` into every copied model whose archive path satisfies
    /// `included`, returning how many models changed.
    ///
    /// Models whose blend is already pinned are re-baked rather than skipped,
    /// so a pool cloned from an already-stained stage still takes its own
    /// layer's look. Callers own the undo record, letting the bake ride a
    /// larger transaction.
    fn bake_stain_into_models(
        &mut self,
        stain: &sms_formats::BtiFile,
        included: &dyn Fn(&str) -> bool,
        protect_from_runtime: bool,
        require_pinned_blend: bool,
    ) -> usize {
        const DUMMY_TEXTURE: &str = "H_ma_rak_dummy";
        const STAIN_MATERIAL: &str = "_mat_body_top1";
        // `THamuKuri::setMActorAndKeeper` looks the stain slot up by this name
        // and overwrites it from the one stage-wide texture on every spawn.
        // A pool that must keep its own look renames the slot out of reach;
        // materials address textures by index, so only the lookup changes.
        const PROTECTED_TEXTURE: &str = "H_ma_rak_layer";
        let Some(document) = self.document.as_mut() else {
            return 0;
        };
        let mut baked_models = 0usize;
        for edit in &mut document.archive_edits.resources {
            let path = String::from_utf8_lossy(&edit.raw_resource_path).replace('\\', "/");
            if !included(path.trim_matches('/')) {
                continue;
            }
            let StageResourceDocument::Model(model) = &mut edit.document else {
                continue;
            };
            // A pool baked once already carries the renamed slot, so re-baking
            // has to target whichever name this model actually has.
            let slot = if model.has_named_texture(DUMMY_TEXTURE) {
                DUMMY_TEXTURE
            } else if protect_from_runtime && model.has_named_texture(PROTECTED_TEXTURE) {
                PROTECTED_TEXTURE
            } else {
                continue;
            };
            let already_pinned = model.material_konst_alpha_half_is_pinned(STAIN_MATERIAL);
            let can_pin = model.can_pin_material_konst_alpha_half(STAIN_MATERIAL);
            // A Stu cap needs the blend pinned on or the stain never shows; an
            // actor whose goop is a runtime-animated mask (Pakkun fades its
            // coating as Mario sprays it) has no such material and must keep
            // driving its own alpha, so only the texture is swapped there.
            if require_pinned_blend && !already_pinned && !can_pin {
                continue;
            }
            let mut baked = model.clone();
            if !already_pinned && can_pin {
                match baked.pin_material_konst_alpha_half(STAIN_MATERIAL) {
                    Ok(count) if count > 0 => {}
                    Ok(_) if require_pinned_blend => continue,
                    Ok(_) => {}
                    Err(error) => {
                        self.log.push(format!(
                            "Could not pin the stain blend in {}: {error}",
                            String::from_utf8_lossy(&edit.raw_resource_path)
                        ));
                        continue;
                    }
                }
            }
            let replaced = match baked.replace_named_texture_from_bti(slot, stain) {
                Ok(count) => count,
                Err(error) => {
                    self.log.push(format!(
                        "Could not bake the stain into {}: {error}",
                        String::from_utf8_lossy(&edit.raw_resource_path)
                    ));
                    continue;
                }
            };
            if replaced == 0 {
                continue;
            }
            if protect_from_runtime && slot == DUMMY_TEXTURE {
                if let Err(error) = baked.rename_texture(DUMMY_TEXTURE, PROTECTED_TEXTURE) {
                    self.log.push(format!(
                        "Could not shield the baked stain in {} from the runtime: {error}",
                        String::from_utf8_lossy(&edit.raw_resource_path)
                    ));
                    continue;
                }
            }
            *model = baked;
            baked_models += 1;
        }
        baked_models
    }

    /// The stain texture for a goop layer's style, from the style's retail
    /// source archive.
    /// The surface texture a goop layer paints with, lifted from the layer's
    /// pollution model.
    ///
    /// A stage's per-layer looks live inside `pollutionNN.bmd`, not in the
    /// stage-wide `H_ma_rak.bti` (which is one shared pink across nearly every
    /// retail stage). A pollution model carries three textures in a consistent
    /// order: the stage's painted coverage mask, then the goop material, then
    /// a shared edge map. The material is the one that reads as the goop --
    /// `B_RAKenogu_pink` for graffiti pink, `B_ricoDrDr` for Ricco's sludge,
    /// `TestChoco2` for bianco's brown -- so the layer's look is index 1.
    /// Index 0 is a stage-shaped mask and renders as a meaningless blob on a
    /// cap.
    fn stain_for_layer(&self, layer_index: usize) -> Option<sms_formats::BtiFile> {
        let document = self.document.as_ref()?;
        let style = document
            .goop_authoring
            .as_ref()?
            .layers
            .get(layer_index)?
            .style_source
            .as_ref()?;
        let template = self.retail_goop_templates.iter().find(|template| {
            template.stage_id == style.stage_id && template.layer_index == style.layer_index
        })?;
        let model_path = format!("map/pollution/pollution{:02}.bmd", style.layer_index);
        let bytes = std::fs::read(&template.archive_path).ok()?;
        let archive = sms_scene::SourceFreeStageArchive::parse(&bytes).ok()?;
        let resource = archive.resources().iter().find(|resource| {
            String::from_utf8_lossy(&resource.raw_path)
                .replace('\\', "/")
                .eq_ignore_ascii_case(&model_path)
        })?;
        let StageResourceDocument::Model(model) = &resource.document else {
            return None;
        };
        let names = model.texture_names();
        // Fall back to whatever exists if a model does not follow the retail
        // three-texture layout, so an unusual style still yields something.
        let texture_name = names.get(1).or_else(|| names.first())?;
        model.named_texture_as_bti(texture_name).ok()
    }

    pub(super) fn bake_stu_stain(&mut self) {
        const STAIN_PATH: &[u8] = b"map/pollution/H_ma_rak.bti";

        let Some(document) = self.document.as_ref() else {
            return;
        };
        let before = document.archive_edits.clone();
        let stain = match document.effective_resource_clone(STAIN_PATH) {
            Ok(Some(StageResourceDocument::Texture(stain))) => stain,
            Ok(_) => {
                self.log.push(
                    "The stage has no map/pollution/H_ma_rak.bti to bake; place a Stu first so \
                     its resources are copied."
                        .to_string(),
                );
                return;
            }
            Err(error) => {
                self.log
                    .push(format!("Could not read the stain texture: {error}"));
                return;
            }
        };

        let baked_models =
            self.bake_stain_into_models(&stain, &Self::outside_layer_pool_folders, false, true);

        if baked_models == 0 {
            self.log.push(
                "No copied model carries the stain slot; place a Stu from the content browser \
                 first."
                    .to_string(),
            );
            return;
        }

        let (record, dirty) = {
            let Some(document) = self.document.as_ref() else {
                return;
            };
            (
                ObjectUndoRecord::between(
                    &document.objects,
                    &document.objects,
                    &before,
                    &document.archive_edits,
                ),
                stage_document_differs_from_saved(
                    document,
                    &self.saved_objects,
                    &self.saved_lighting,
                    &self.saved_death_barrier,
                    &self.saved_archive_edits,
                    &self.saved_dialogue_authoring,
                    &self.saved_dialogue_library,
                ),
            )
        };
        if !record.is_empty() {
            self.push_undo_record(record);
        }
        self.document_dirty = dirty;
        self.flush_document_change();
        self.log.push(format!(
            "Baked the goop stain into {baked_models} model(s); Ctrl+Z reverses it."
        ));
    }

    pub(super) fn stu_stain_available(&self) -> bool {
        const DUMMY_TEXTURE: &str = "H_ma_rak_dummy";
        const STAIN_MATERIAL: &str = "_mat_body_top1";
        let Some(document) = self.document.as_ref() else {
            return false;
        };
        document.archive_edits.resources.iter().any(|edit| {
            Self::edit_outside_layer_pool_folders(&edit.raw_resource_path)
                && matches!(
                    &edit.document,
                    StageResourceDocument::Model(model)
                        if model.has_named_texture(DUMMY_TEXTURE)
                            && model.can_pin_material_konst_alpha_half(STAIN_MATERIAL)
                )
        })
    }

    /// Whether a resource path belongs outside every per-layer pool folder.
    ///
    /// The stage-wide stain reads and writes only the base pool: a per-layer
    /// pool owns its look. Reporting on pool folders too made the toggle show
    /// as baked whenever any layer was, so clicking it always unbaked and the
    /// base Stus could never get their stain back.
    fn edit_outside_layer_pool_folders(raw_resource_path: &[u8]) -> bool {
        let path = String::from_utf8_lossy(raw_resource_path).replace('\\', "/");
        Self::outside_layer_pool_folders(path.trim_matches('/'))
    }

    /// Whether a base-pool Stu model carries the baked stain.
    pub(super) fn stu_stain_baked(&self) -> bool {
        const STAIN_MATERIAL: &str = "_mat_body_top1";
        let Some(document) = self.document.as_ref() else {
            return false;
        };
        document.archive_edits.resources.iter().any(|edit| {
            Self::edit_outside_layer_pool_folders(&edit.raw_resource_path)
                && matches!(
                    &edit.document,
                    StageResourceDocument::Model(model)
                        if model.material_konst_alpha_half_is_pinned(STAIN_MATERIAL)
                )
        })
    }

    /// Removes the baked stain, returning the blend to the runtime register.
    pub(super) fn unbake_stu_stain(&mut self) {
        const STAIN_MATERIAL: &str = "_mat_body_top1";
        let Some(document) = self.document.as_ref() else {
            return;
        };
        let before = document.archive_edits.clone();
        let Some(document) = self.document.as_mut() else {
            return;
        };
        let mut unbaked = 0usize;
        for edit in &mut document.archive_edits.resources {
            let path = String::from_utf8_lossy(&edit.raw_resource_path).replace('\\', "/");
            if !Self::outside_layer_pool_folders(path.trim_matches('/')) {
                continue;
            }
            let StageResourceDocument::Model(model) = &mut edit.document else {
                continue;
            };
            match model.unpin_material_konst_alpha_half(STAIN_MATERIAL) {
                Ok(count) if count > 0 => unbaked += 1,
                Ok(_) => {}
                Err(error) => self.log.push(format!(
                    "Could not remove the stain from {}: {error}",
                    String::from_utf8_lossy(&edit.raw_resource_path)
                )),
            }
        }
        if unbaked == 0 {
            return;
        }
        let (record, dirty) = {
            let Some(document) = self.document.as_ref() else {
                return;
            };
            (
                ObjectUndoRecord::between(
                    &document.objects,
                    &document.objects,
                    &before,
                    &document.archive_edits,
                ),
                stage_document_differs_from_saved(
                    document,
                    &self.saved_objects,
                    &self.saved_lighting,
                    &self.saved_death_barrier,
                    &self.saved_archive_edits,
                    &self.saved_dialogue_authoring,
                    &self.saved_dialogue_library,
                ),
            )
        };
        if !record.is_empty() {
            self.push_undo_record(record);
        }
        self.document_dirty = dirty;
        self.flush_document_change();
        self.log
            .push(format!("Removed the baked stain from {unbaked} model(s)."));
    }

    /// Suffix that marks a per-layer clone of a manager bundle.
    fn layer_pool_suffix(layer_index: usize) -> String {
        format!("_L{layer_index:02}")
    }

    /// Goop layers a per-layer pool can be bound to: index and style name.
    pub(super) fn goop_layer_choices(&self) -> Vec<(usize, String)> {
        let Some(document) = self.document.as_ref() else {
            return Vec::new();
        };
        let Some(authoring) = document.goop_authoring.as_ref() else {
            return Vec::new();
        };
        authoring
            .layers
            .iter()
            .enumerate()
            .filter_map(|(index, layer)| {
                layer
                    .style_source
                    .as_ref()
                    .map(|style| (index, style.display_name.clone()))
            })
            .collect()
    }

    /// Whether a per-layer pool exists for this manager and layer.
    pub(super) fn layer_pool_exists(&self, manager_name: &str, layer_index: usize) -> bool {
        let cloned = format!("{manager_name}{}", Self::layer_pool_suffix(layer_index));
        self.document.as_ref().is_some_and(|document| {
            document
                .objects
                .iter()
                .any(|object| object.raw_param("manager_name") == Some(cloned.as_str()))
        })
    }

    /// Every per-layer pool this stage carries, as layer index -> manager name.
    ///
    /// This is the mapping the runtime patch consumes: the conductor picks a
    /// pool before it rolls a position, so the layer a spawn lands in has to
    /// select the pool after the fact.
    pub(super) fn layer_pool_bindings(&self) -> BTreeMap<usize, String> {
        let mut bindings = BTreeMap::new();
        let Some(document) = self.document.as_ref() else {
            return bindings;
        };
        for (index, _) in self.goop_layer_choices() {
            let suffix = Self::layer_pool_suffix(index);
            for object in &document.objects {
                let Some(name) = object.raw_param("manager_name") else {
                    continue;
                };
                if name.ends_with(&suffix) {
                    bindings.insert(index, name.to_string());
                    break;
                }
            }
        }
        bindings
    }

    /// Adds or removes the pool that spawns in one goop layer.
    ///
    /// Adding imports a renamed copy of the manager's retail bundle, so the
    /// clone loads its own models, then bakes that layer's stain into the
    /// clone's own model copy and flags it in the enemy table. All of it is
    /// one undo step.
    fn set_layer_pool(&mut self, entity: &GoopSpawnEntity, layer_index: usize, enabled: bool) {
        let Some(document) = self.document.as_ref() else {
            return;
        };
        let before_objects = document.objects.clone();
        let before_archive_edits = document.archive_edits.clone();
        let before_object_serial = self.next_object_serial;
        let suffix = Self::layer_pool_suffix(layer_index);
        let cloned_manager = format!("{}{suffix}", entity.manager_name);
        let mut log = Vec::new();

        let result: Result<(), String> = (|| {
            if enabled {
                let actor_factory = entity.catalog_actor_factory.as_deref().ok_or_else(|| {
                    format!(
                        "manager {:?} has no retail-backed bundle to clone",
                        entity.manager_name
                    )
                })?;
                let manager_factory = entity.manager_factory.as_deref().ok_or_else(|| {
                    format!(
                        "manager {:?} has no decomp-derived factory",
                        entity.manager_name
                    )
                })?;
                if self.layer_pool_exists(&entity.manager_name, layer_index) {
                    // An existing pool may predate the folder materialization,
                    // in which case it resolves to no models and kills the
                    // stage on load. Repair rather than trust it.
                    let repaired = self.ensure_cloned_manager_pool_resources(
                        actor_factory,
                        &entity.manager_name,
                        &suffix,
                    )?;
                    if repaired > 0 {
                        log.push(format!(
                            "Restored {repaired} missing model resource(s) for the existing                              layer {layer_index:02} pool."
                        ));
                    }
                } else {
                    log.push(self.ensure_cloned_enemy_manager_pool(
                        actor_factory,
                        manager_factory,
                        &entity.manager_name,
                        &suffix,
                    )?);
                }
                // The pool's own model copy takes its layer's stain, so what
                // climbs out of a layer wears that layer's look. Baking is
                // idempotent and re-runs on repair, keeping the look current.
                let folders =
                    self.cloned_manager_folders(actor_factory, &entity.manager_name, &suffix)?;
                match self.stain_for_layer(layer_index) {
                    Some(stain) => {
                        let baked = self.bake_stain_into_models(
                            &stain,
                            &|path: &str| {
                                let path = path.to_ascii_lowercase();
                                folders.iter().any(|folder| {
                                    let folder = folder.to_ascii_lowercase();
                                    path == folder || path.starts_with(&format!("{folder}/"))
                                })
                            },
                            true,
                            true,
                        );
                        log.push(format!(
                            "Baked layer {layer_index:02}'s own look into {baked} pool model(s), \
                             shielded from the runtime's stain replacement."
                        ));
                    }
                    None => log.push(format!(
                        "Layer {layer_index:02} has no retail stain texture available, so its \
                         pool keeps the current look."
                    )),
                }
                self.apply_manager_spawn(&cloned_manager, true)?;
            } else {
                self.apply_manager_spawn(&cloned_manager, false)?;
                if let Some(document) = self.document.as_mut() {
                    document.objects.retain(|object| {
                        object.raw_param("manager_name") != Some(cloned_manager.as_str())
                    });
                }
                log.extend(self.cleanup_unused_goop_manager_pools());
            }
            Ok(())
        })();

        if let Err(error) = result {
            if let Some(document) = self.document.as_mut() {
                document.objects = before_objects;
                document.archive_edits = before_archive_edits;
            }
            self.next_object_serial = before_object_serial;
            self.log.push(format!(
                "Could not update the layer {layer_index:02} pool: {error}"
            ));
            return;
        }

        let (record, dirty) = {
            let Some(document) = self.document.as_ref() else {
                return;
            };
            (
                ObjectUndoRecord::between(
                    &before_objects,
                    &document.objects,
                    &before_archive_edits,
                    &document.archive_edits,
                ),
                stage_document_differs_from_saved(
                    document,
                    &self.saved_objects,
                    &self.saved_lighting,
                    &self.saved_death_barrier,
                    &self.saved_archive_edits,
                    &self.saved_dialogue_authoring,
                    &self.saved_dialogue_library,
                ),
            )
        };
        if !record.is_empty() {
            self.push_undo_record(record);
        }
        self.document_dirty = dirty;
        self.flush_document_change();
        self.log.extend(log);
        self.log.push(match enabled {
            true => format!(
                "Layer {layer_index:02} spawns from its own pool {cloned_manager:?}, wearing \
                 that layer's stain."
            ),
            false => format!("Removed the layer {layer_index:02} pool {cloned_manager:?}."),
        });
    }

    /// Writes the layer -> pool mapping the runtime layer patch consumes.
    ///
    /// The conductor chooses a pool before it rolls a spawn position, so the
    /// layer a spawn lands in can only select the pool afterwards. The patch
    /// does that swap; this file tells it which pool belongs to which layer.
    /// A styled layer with no pool of its own is written as null, meaning
    /// nothing spawns there.
    pub(super) fn write_layer_pool_bindings(&mut self) {
        let Some(project) = self.current_project.as_ref() else {
            self.log.push(
                "Save the project first: the layer bindings are written beside its build."
                    .to_string(),
            );
            return;
        };
        let bindings = self.layer_pool_bindings();
        if bindings.is_empty() {
            self.log.push(
                "No per-layer pools exist yet, so there is nothing for the runtime patch to route."
                    .to_string(),
            );
            return;
        }
        let mut entries = Vec::new();
        for (index, _) in self.goop_layer_choices() {
            let value = match bindings.get(&index) {
                Some(manager) => format!(
                    "\"{index}\": {}",
                    serde_json::to_string(manager).unwrap_or_default()
                ),
                None => format!("\"{index}\": null"),
            };
            entries.push(format!("    {value}"));
        }
        let text = format!(
            "{{\n  \"every_frame\": false,\n  \"layers\": {{\n{}\n  }}\n}}\n",
            entries.join(",\n")
        );
        let path = project
            .managed_build_root()
            .join("goop-layer-bindings.json");
        if let Some(parent) = path.parent() {
            if let Err(error) = std::fs::create_dir_all(parent) {
                self.log
                    .push(format!("Could not prepare the build folder: {error}"));
                return;
            }
        }
        match std::fs::write(&path, text) {
            Ok(()) => self.log.push(format!(
                "Wrote {} layer binding(s) to {}.",
                bindings.len(),
                path.display()
            )),
            Err(error) => self
                .log
                .push(format!("Could not write the layer bindings: {error}")),
        }
    }

    /// The goop layer a placed actor samples, read from its manager name.
    pub(super) fn selected_actor_goop_layer(object: &SceneObject) -> Option<usize> {
        let manager = object.raw_param("manager_name")?;
        let (_, suffix) = manager.rsplit_once("_L")?;
        (suffix.len() == 2 && suffix.chars().all(|item| item.is_ascii_digit()))
            .then(|| suffix.parse().ok())
            .flatten()
    }

    /// Binds the selected placed actor to a goop layer, or back to the stage.
    ///
    /// The actor is rebound to that layer's clone bundle, whose model copy
    /// carries the layer's baked goop texture shielded from the runtime's
    /// stage-wide replacement -- so the actor wears the goop it stands in.
    /// `None` returns it to its base manager and the stage texture.
    pub(super) fn set_selected_actor_goop_layer(&mut self, layer: Option<usize>) {
        let Some(object) = self.selected_object().cloned() else {
            return;
        };
        let Some(current_manager) = object.raw_param("manager_name").map(str::to_owned) else {
            self.log
                .push("That actor has no manager to rebind.".to_string());
            return;
        };
        let base_manager = match current_manager.rsplit_once("_L") {
            Some((base, suffix))
                if suffix.len() == 2 && suffix.chars().all(|item| item.is_ascii_digit()) =>
            {
                base.to_string()
            }
            _ => current_manager.clone(),
        };
        let target_manager = match layer {
            Some(index) => format!("{base_manager}{}", Self::layer_pool_suffix(index)),
            None => base_manager.clone(),
        };
        if target_manager == current_manager {
            return;
        }

        if let Some(index) = layer {
            let manager_factory = {
                let (Some(document), Some(registry)) =
                    (self.document.as_ref(), self.registry.as_ref())
                else {
                    return;
                };
                let Some(instance) = enemy_manager_instances(document, registry)
                    .get(&base_manager)
                    .cloned()
                else {
                    self.log.push(format!(
                        "Could not rebind '{}': {base_manager:?} is not a decomp-identified \
                         TEnemyManager in this stage.",
                        object.id
                    ));
                    return;
                };
                instance.factory_name
            };
            let actor_factory = object.factory_name.clone();
            let suffix = Self::layer_pool_suffix(index);
            let clone_exists = self.document.as_ref().is_some_and(|document| {
                document
                    .objects
                    .iter()
                    .any(|item| item.raw_param("manager_name") == Some(target_manager.as_str()))
            });
            let prepared = if clone_exists {
                self.ensure_cloned_manager_pool_resources(&actor_factory, &base_manager, &suffix)
                    .map(|_| ())
            } else {
                self.ensure_cloned_enemy_manager_pool(
                    &actor_factory,
                    &manager_factory,
                    &base_manager,
                    &suffix,
                )
                .map(|message| self.log.push(message))
            };
            if let Err(error) = prepared {
                self.log.push(format!(
                    "Could not prepare the layer {index:02} bundle: {error}"
                ));
                return;
            }
            let folders = match self.cloned_manager_folders(&actor_factory, &base_manager, &suffix)
            {
                Ok(folders) => folders,
                Err(error) => {
                    self.log.push(format!(
                        "Could not locate the layer's model folder: {error}"
                    ));
                    return;
                }
            };
            match self.stain_for_layer(index) {
                Some(stain) => {
                    let baked = self.bake_stain_into_models(
                        &stain,
                        &|path: &str| {
                            let path = path.to_ascii_lowercase();
                            folders.iter().any(|folder| {
                                let folder = folder.to_ascii_lowercase();
                                path == folder || path.starts_with(&format!("{folder}/"))
                            })
                        },
                        true,
                        false,
                    );
                    self.log.push(format!(
                        "Baked layer {index:02}'s goop texture into {baked} model(s)."
                    ));
                }
                None => self.log.push(format!(
                    "Layer {index:02} has no goop texture available; the actor keeps its \
                     current look."
                )),
            }
        }

        // The parameter editor refuses manager_name on a retail-placed actor
        // ("no owned dependency"), which is right in general: relinking to a
        // manager that does not exist would break export. Here the target
        // manager was just created and owned by its pool carrier, so the link
        // resolves and the edit is applied directly.
        let Some(mut after) = self.selected_object().cloned() else {
            return;
        };
        let before = after.clone();
        after.set_raw_param("manager_name", target_manager.clone());
        // The actor's own character stays on the base registration: export
        // re-applies parameters and refuses a changed character_name (it owns
        // a registration), and the model comes through the manager's keeper
        // anyway, so only the manager needs to move. Writing the base also
        // repairs actors a previous build had suffixed, which failed export.
        if let Some(character) = before.raw_param("character_name") {
            let base = match character.rsplit_once("_L") {
                Some((base, digits))
                    if digits.len() == 2 && digits.chars().all(|item| item.is_ascii_digit()) =>
                {
                    base.to_string()
                }
                _ => character.to_string(),
            };
            after.set_raw_param("character_name", base);
        }
        sms_scene::sync_scene_object_parameter_aliases(&mut after);
        if let Some(sms_scene::PlacementBinding::Authored(authored)) = after.placement.as_mut() {
            for dependency in &mut authored.dependencies {
                if dependency.record.name != current_manager {
                    continue;
                }
                dependency.record.name.clone_from(&target_manager);
                // The payload must move with the name: the clone bundle's
                // manager record points at the clone's character, and export
                // refuses two same-named records with different payloads.
                let fields = match &mut dependency.record.payload {
                    sms_formats::JDramaRecordPayload::Fields { fields }
                    | sms_formats::JDramaRecordPayload::Actor { fields, .. } => fields,
                    _ => continue,
                };
                for field in fields {
                    if field.name != "character_name" {
                        continue;
                    }
                    if let sms_formats::JDramaFieldValue::String(value) = &mut field.value {
                        let base = match value.rsplit_once("_L") {
                            Some((base, digits))
                                if digits.len() == 2
                                    && digits.chars().all(|item| item.is_ascii_digit()) =>
                            {
                                base.to_string()
                            }
                            _ => value.clone(),
                        };
                        *value = match layer {
                            Some(index) => {
                                format!("{base}{}", Self::layer_pool_suffix(index))
                            }
                            None => base,
                        };
                    }
                }
            }
        }
        self.apply_object_edit(
            "Bound actor to a goop layer",
            ObjectUndoRecord {
                deltas: vec![ObjectDelta::Update {
                    before: Box::new(before),
                    after: Box::new(after),
                }],
                resource_deltas: Vec::new(),
                route_delta: None,
                dialogue_delta: None,
            },
        );
        self.log.push(match layer {
            Some(index) => format!("'{}' now samples goop layer {index:02}.", object.id),
            None => format!("'{}' returned to the stage goop texture.", object.id),
        });
    }

    /// The Spawning section of the goop inspector.
    pub(super) fn gooble_spawn_section(&mut self, ui: &mut egui::Ui) {
        ui.separator();
        ui.strong("Goop enemy managers (TConductor)");
        let entities = self.goop_spawnable_entities();
        if entities.is_empty() {
            ui.label(
                "No safe enemy-manager bundles were found in the decomp-derived retail catalog.",
            );
            return;
        }
        for entity in entities {
            let flagged = self.manager_spawns_from_goop(&entity.manager_name);
            let catalog_available =
                entity.catalog_actor_factory.is_some() && entity.unavailable_reason.is_none();
            let stale = !entity.manager_present && !catalog_available;
            // A flagged entry with a missing but repairable manager is shown
            // unchecked so selecting it performs the missing bundle import.
            let mut enabled = flagged && entity.variant_active && (entity.manager_present || stale);
            let suffix = if entity.unavailable_reason.is_some() {
                if flagged {
                    " (unavailable; uncheck to remove)"
                } else {
                    " (unavailable)"
                }
            } else if entity.manager_present {
                ""
            } else if catalog_available {
                " (add manager pool)"
            } else {
                " (remove stale flag)"
            };
            let response = ui.add_enabled(
                entity.unavailable_reason.is_none() || flagged,
                egui::Checkbox::new(
                    &mut enabled,
                    format!(
                        "{} \u{2014} {}{}",
                        entity.display_name, entity.manager_name, suffix
                    ),
                ),
            );
            let hover = if let Some(reason) = entity.unavailable_reason {
                if flagged {
                    format!("{reason} Uncheck it to remove this goop spawn configuration.")
                } else {
                    reason.to_string()
                }
            } else if entity.manager_present {
                "Writes the retail enemy table: the conductor periodically picks a spot \
                         near Mario and, if that spot is goop, relocates one of this manager's \
                         enemies there."
                    .to_string()
            } else if catalog_available {
                "Adds this decomp-identified manager with its exact retail dependency and \
                         resource bundle, but no placed enemy instance, then enables it in the \
                         retail enemy table. The whole change is one undo step."
                    .to_string()
            } else {
                "This flagged table entry no longer has a TEnemyManager in map/scene.bin. \
                         Uncheck it; export also clears stale entries defensively."
                    .to_string()
            };
            if response.on_hover_text(hover).changed()
                && (entity.manager_present || catalog_available || !enabled)
            {
                self.set_manager_spawns_from_goop(&entity, enabled);
            }

            // Per-layer pools: one independent copy of this bundle per goop
            // layer, so each can carry its own baked stain. Only offered for
            // bundles the catalog can clone.
            let layers = self.goop_layer_choices();
            if catalog_available && !layers.is_empty() {
                let mut toggled = None;
                egui::CollapsingHeader::new(format!(
                    "Per-layer pools \u{2014} {}",
                    entity.display_name
                ))
                .id_salt(format!("layer-pools-{}", entity.manager_name))
                .show(ui, |ui| {
                    ui.label(
                        "Each layer gets its own copy of this enemy's manager, character and \
                         models, so it can wear that layer's stain. Needs the runtime layer \
                         patch to route spawns.",
                    );
                    for (index, name) in &layers {
                        let mut present = self.layer_pool_exists(&entity.manager_name, *index);
                        if ui
                            .checkbox(&mut present, format!("{index:02} {name}"))
                            .on_hover_text(format!(
                                "Adds pool {:?} for this layer.",
                                format!(
                                    "{}{}",
                                    entity.manager_name,
                                    Self::layer_pool_suffix(*index)
                                )
                            ))
                            .changed()
                        {
                            toggled = Some((*index, present));
                        }
                    }
                });
                if let Some((index, present)) = toggled {
                    self.set_layer_pool(&entity, index, present);
                }
            }
        }

        // The stain bake needs no placed actor -- it edits the copied Stu
        // model -- so a pool spawned purely from this panel gets its stain
        // control here too.
        if !self.layer_pool_bindings().is_empty()
            && ui
                .button("Write layer spawn bindings")
                .on_hover_text(
                    "Writes goop-layer-bindings.json beside the managed build: the layer ->                      pool mapping the runtime layer patch reads.",
                )
                .clicked()
        {
            self.write_layer_pool_bindings();
        }

        if self.stu_stain_available() {
            let mut stained = self.stu_stain_baked();
            if ui
                .checkbox(&mut stained, "Goop stain on cap")
                .on_hover_text(
                    "Bakes the stage's stain texture into the Stu model and pins its \
                     blend, so it shows regardless of what the runtime decides. Applies \
                     to the base pool; per-layer pools keep their own layer's stain.",
                )
                .changed()
            {
                match stained {
                    true => self.bake_stu_stain(),
                    false => self.unbake_stu_stain(),
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    #[ignore = "requires SMS_BASE_ROOT with extracted retail stages"]
    fn bundled_us_catalog_exposes_only_supported_reported_goop_choices() {
        let base_root = std::env::var_os("SMS_BASE_ROOT")
            .map(std::path::PathBuf::from)
            .expect("SMS_BASE_ROOT");
        let archives = sms_formats::discover_scene_archives(&base_root).unwrap();
        let registry = sms_schema::bundled_object_registry().unwrap().registry;
        let build = sms_scene::ObjectAuthoringCatalog::build_with_base_root(
            &archives, &registry, &base_root,
        );
        let choices = catalog_enemy_manager_choices(&build.catalog, &registry)
            .into_values()
            .flatten()
            .collect::<Vec<_>>();
        for expected in ["Telesa", "PukuPuku", "PoiHanaRed"] {
            assert!(
                choices
                    .iter()
                    .any(|choice| choice.actor_factory == expected),
                "missing {expected} from goop choices"
            );
        }
        let boss_manta = choices
            .iter()
            .find(|choice| choice.actor_factory == "BossManta")
            .expect("BossManta remains visible as unavailable");
        assert!(boss_manta.unavailable_reason.is_some());
        for excluded in [
            "BossTelesa",
            "Rocket",
            "TobiPukuLaunchPad",
            "MoePukuLaunchPad",
        ] {
            assert!(
                choices
                    .iter()
                    .all(|choice| choice.actor_factory != excluded),
                "unsafe/non-enemy {excluded} remained in goop choices"
            );
        }
        assert!(choices.iter().any(|choice| {
            choice.actor_factory == "PukuPuku" && choice.display_name == "TobiPuku"
        }));
    }

    #[test]
    fn boss_manta_is_unavailable_until_its_runtime_pool_is_fixed() {
        assert!(goop_enemy_unavailable_reason(Some("BossManta"), None).is_some());
        assert!(goop_enemy_unavailable_reason(None, Some("BossMantaManager")).is_some());
        assert!(goop_enemy_unavailable_reason(Some("Telesa"), Some("TelesaManager")).is_none());
    }

    fn enemy_registry() -> ObjectRegistry {
        ObjectRegistry {
            enemy_managers: vec![sms_schema::EnemyManagerDefinition {
                factory_name: "NameKuriManager".to_string(),
                class_name: "TNameKuriManager".to_string(),
                model_index: None,
                spawned_actor_class: Some("TNameKuri".to_string()),
                parameter_path: None,
                models: Vec::new(),
            }],
            enemy_actors: vec![sms_schema::EnemyActorDefinition {
                factory_name: "NameKuri".to_string(),
                class_name: "TNameKuri".to_string(),
                model_index: None,
                fallback_models: Vec::new(),
                primary_model: None,
                named_models: Vec::new(),
                indexed_models: Vec::new(),
                manager_factories: vec!["NameKuriManager".to_string()],
                runtime_uniform_scale: None,
            }],
            ..ObjectRegistry::default()
        }
    }

    fn manager_record(factory: &str, name: &str) -> sms_formats::JDramaRecord {
        sms_formats::JDramaRecord::new(
            factory,
            name,
            sms_formats::JDramaRecordPayload::Fields { fields: Vec::new() },
        )
        .unwrap()
    }

    fn enemy_template(
        actor_factory: &str,
        manager_factory: &str,
        manager_name: &str,
    ) -> sms_scene::ObjectAuthoringTemplate {
        sms_scene::ObjectAuthoringTemplate {
            factory_name: actor_factory.to_string(),
            group_index: 4,
            character_resource_records: Vec::new(),
            record: sms_formats::JDramaRecord {
                type_name: actor_factory.to_string(),
                name: "retail enemy".to_string(),
                payload: sms_formats::JDramaRecordPayload::Actor {
                    transform: sms_formats::JDramaTransform {
                        translation: [0.0; 3],
                        rotation: [0.0; 3],
                        scale: [1.0; 3],
                    },
                    character_name: "EnemyChara".to_string(),
                    light_map: sms_formats::JDramaLightMap::default(),
                    fields: vec![jdrama_field(
                        "manager_name",
                        sms_formats::JDramaFieldValue::String(manager_name.to_string()),
                    )],
                },
            },
            dependencies: vec![sms_scene::ObjectAuthoringDependency {
                group_index: 2,
                target: sms_scene::AuthoredPlacementDependencyTarget::IndexedGroup {
                    group_index: 2,
                },
                record: manager_record(manager_factory, manager_name),
            }],
            character_records: Vec::new(),
            table_dependencies: Vec::new(),
            runtime_actor_references: Vec::new(),
            required_graph_names: Vec::new(),
            resources: Vec::new(),
            preview_resource_path: None,
            source_stage: "retail0".to_string(),
        }
    }

    /// The stage-wide stain toggle must not repaint per-layer pool folders:
    /// their look belongs to their layer.
    #[test]
    fn stage_wide_stain_actions_skip_layer_pool_folders() {
        assert!(SmsEditorApp::is_layer_pool_folder("hamukuri_l00"));
        assert!(SmsEditorApp::is_layer_pool_folder("namekuri_l15"));
        assert!(!SmsEditorApp::is_layer_pool_folder("hamukuri"));
        assert!(!SmsEditorApp::is_layer_pool_folder("hamukurianm"));
        assert!(!SmsEditorApp::is_layer_pool_folder("namekuri2"));
        assert!(SmsEditorApp::outside_layer_pool_folders(
            "hamukuri/default.bmd"
        ));
        assert!(SmsEditorApp::outside_layer_pool_folders(
            "map/pollution/H_ma_rak.bti"
        ));
        assert!(!SmsEditorApp::outside_layer_pool_folders(
            "hamukuri_l01/default.bmd"
        ));
    }

    /// A per-layer pool is only useful if it is genuinely separate: same
    /// bundle, but its own manager, character and model folder, so a stain
    /// baked for one layer cannot bleed into another.
    #[test]
    fn per_layer_pools_are_independent_bundles() {
        let mut template = enemy_template("HamuKuri", "HamuKuriManager", "hamuManager");
        // Global scenecmn.bin supplies this one, exactly like the real bundle.
        template.character_resource_records = vec![sms_formats::JDramaRecord {
            type_name: "ObjChara".to_string(),
            name: "EnemyChara".to_string(),
            payload: sms_formats::JDramaRecordPayload::Fields {
                fields: vec![sms_formats::JDramaField {
                    name: "resource_folder".to_string(),
                    value: sms_formats::JDramaFieldValue::String("/scene/hamukuri".to_string()),
                }],
            },
        }];
        let first = sms_scene::clone_enemy_manager_template(&template, "hamuManager", "_L00", true)
            .unwrap()
            .template;
        let second =
            sms_scene::clone_enemy_manager_template(&template, "hamuManager", "_L01", true)
                .unwrap()
                .template;

        assert_eq!(first.dependencies[0].record.name, "hamuManager_L00");
        assert_eq!(second.dependencies[0].record.name, "hamuManager_L01");
        assert_ne!(
            first.dependencies[0].record.name,
            second.dependencies[0].record.name
        );
        assert_ne!(
            first.character_records[0].name,
            second.character_records[0].name
        );
    }

    #[test]
    fn manager_census_includes_manager_only_enemies_and_excludes_live_managers() {
        let registry = enemy_registry();
        let root = sms_formats::JDramaRecord::new(
            "NameRefGrp",
            "root",
            sms_formats::JDramaRecordPayload::Group {
                fields: Vec::new(),
                children: vec![
                    manager_record("NameKuriManager", GOOBLE_MANAGER_NAME),
                    manager_record("BoardNpcManager", "board manager"),
                ],
            },
        )
        .unwrap();
        let mut managers = BTreeMap::new();
        collect_enemy_manager_records(&root, &registry, &mut managers);

        assert_eq!(managers.len(), 1);
        assert_eq!(
            managers.get(GOOBLE_MANAGER_NAME),
            Some(&EnemyManagerInstance {
                factory_name: "NameKuriManager".to_string(),
                display_name: "NameKuri".to_string(),
            })
        );
        assert!(!managers.contains_key("board manager"));
    }

    #[test]
    fn catalog_exposes_a_decomp_compatible_manager_without_stage_presence() {
        let registry = enemy_registry();
        let choice = catalog_enemy_manager_choice(
            &enemy_template("NameKuri", "NameKuriManager", GOOBLE_MANAGER_NAME),
            &registry,
        )
        .expect("the exact actor-manager dependency should be selectable");

        assert_eq!(choice.actor_factory, "NameKuri");
        assert_eq!(choice.manager_factory, "NameKuriManager");
        assert_eq!(choice.manager_name, GOOBLE_MANAGER_NAME);

        assert!(catalog_enemy_manager_choice(
            &enemy_template("NameKuri", "BoardNpcManager", "board manager"),
            &registry,
        )
        .is_none());
    }

    /// A fresh stage gains a table whose only entry is the flagged Gooble one,
    /// and disabling clears the bit without deleting anything.
    #[test]
    fn the_flag_round_trips_through_an_empty_table() {
        let mut tables = fresh_tables_document();
        assert!(apply_spawn_to_tables(
            &mut tables,
            GOOBLE_MANAGER_NAME,
            true
        ));
        assert!(any_record(&tables.root, &|record| {
            entry_for_manager(record, GOOBLE_MANAGER_NAME) && entry_spawns_from_goop(record)
        }));
        // Setting it again is a no-op, not a duplicate entry.
        assert!(!apply_spawn_to_tables(
            &mut tables,
            GOOBLE_MANAGER_NAME,
            true
        ));

        assert!(apply_spawn_to_tables(
            &mut tables,
            GOOBLE_MANAGER_NAME,
            false
        ));
        assert!(any_record(&tables.root, &|record| entry_for_manager(
            record,
            GOOBLE_MANAGER_NAME
        )));
        assert!(!any_record(&tables.root, &|record| {
            entry_for_manager(record, GOOBLE_MANAGER_NAME) && entry_spawns_from_goop(record)
        }));

        // The document must survive the encoder, or export would corrupt the
        // archive the first time this ships.
        let encoded = sms_formats::encode_jdrama_document(&tables).expect("encode");
        let reparsed = sms_formats::parse_jdrama_document(&encoded).expect("reparse");
        assert_eq!(reparsed.root, tables.root);
    }

    /// An existing retail-style entry keeps its identity; only the bit moves.
    #[test]
    fn an_existing_entry_is_flagged_in_place() {
        let mut tables = fresh_tables_document();
        {
            let sms_formats::JDramaRecordPayload::Group { children, .. } = &mut tables.root.payload
            else {
                unreachable!()
            };
            children.push(sms_formats::JDramaRecord {
                type_name: "StageEnemyInfoHeader".to_string(),
                name: ENEMY_TABLE_NAME.to_string(),
                payload: sms_formats::JDramaRecordPayload::Group {
                    fields: Vec::new(),
                    children: vec![enemy_info_record(GOOBLE_MANAGER_NAME, 0)],
                },
            });
        }
        assert!(apply_spawn_to_tables(
            &mut tables,
            GOOBLE_MANAGER_NAME,
            true
        ));
        let mut entries = 0;
        fn count(record: &sms_formats::JDramaRecord, entries: &mut usize) {
            if entry_for_manager(record, GOOBLE_MANAGER_NAME) {
                *entries += 1;
            }
            if let sms_formats::JDramaRecordPayload::Group { children, .. } = &record.payload {
                for child in children {
                    count(child, entries);
                }
            }
        }
        count(&tables.root, &mut entries);
        assert_eq!(entries, 1, "flagging must edit the entry, not add another");
    }
}
