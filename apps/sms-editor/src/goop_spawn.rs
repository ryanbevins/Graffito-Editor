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
//! The toggle therefore edits two things and invents nothing. It flips bit
//! `0x1` on the Gooble entry in the enemy table, creating the table the way
//! retail stages carry it when a custom stage has none, and it ensures the
//! manager record exists in the scene's manager group. Measured ground truth:
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
    let sms_formats::JDramaRecordPayload::Fields { fields } = &record.payload else {
        return None;
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

fn actor_uses_stu_stain(
    registry: &ObjectRegistry,
    actor: &sms_schema::EnemyActorDefinition,
) -> bool {
    registry
        .runtime_texture_replacements_for(&actor.factory_name)
        .any(|replacement| {
            replacement.dummy_texture_name == "H_ma_rak_dummy"
                && replacement.resource_path == "/scene/map/pollution/H_ma_rak.bti"
                && replacement
                    .source_file
                    .replace('\\', "/")
                    .ends_with("/Enemy/hamukuri.cpp")
        })
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
        out.entry(record.name.clone())
            .or_insert_with(|| EnemyManagerInstance {
                factory_name: manager.factory_name.clone(),
                display_name: enemy_manager_display_name(registry, manager),
            });
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

fn goop_flagged_managers(document: &StageDocument) -> BTreeSet<String> {
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
    manager_present: bool,
}

impl SmsEditorApp {
    /// Whether the effective enemy table flags Goobles to generate from goop.
    /// Enemy managers this stage could spawn from goop, derived from what is
    /// actually in the scene.
    ///
    /// A placed enemy actor names its manager in its `manager_name` parameter,
    /// and export inserts that manager alongside the actor; a retail-derived
    /// stage additionally carries managers in its effective scene. Either way
    /// the conductor can only spawn from managers that exist, so the list is
    /// built from presence rather than from a registry of everything.
    fn goop_spawnable_entities(&self) -> Vec<GoopSpawnEntity> {
        let (Some(document), Some(registry)) = (self.document.as_ref(), self.registry.as_ref())
        else {
            return Vec::new();
        };
        let managers = enemy_manager_instances(document, registry);
        let mut entities = managers
            .iter()
            .map(|(manager_name, manager)| GoopSpawnEntity {
                display_name: manager.display_name.clone(),
                manager_name: manager_name.clone(),
                manager_present: true,
            })
            .collect::<Vec<_>>();
        for manager_name in goop_flagged_managers(document) {
            if !managers.contains_key(&manager_name) {
                entities.push(GoopSpawnEntity {
                    display_name: "Missing enemy manager".to_string(),
                    manager_name,
                    manager_present: false,
                });
            }
        }
        entities
    }

    pub(super) fn object_uses_enemy_pool(&self, object: &SceneObject) -> bool {
        let (Some(document), Some(registry)) = (self.document.as_ref(), self.registry.as_ref())
        else {
            return false;
        };
        let Some(actor) = enemy_actor_definition(registry, &object.factory_name) else {
            return false;
        };
        if !matches!(
            object.placement,
            Some(sms_scene::PlacementBinding::Authored(_))
        ) {
            return false;
        }
        let Some(manager_name) = object.raw_param("manager_name") else {
            return false;
        };
        let managers = enemy_manager_instances(document, registry);
        managers
            .get(manager_name)
            .is_some_and(|manager| manager_factory_matches_actor(actor, &manager.factory_name))
    }

    pub(super) fn object_uses_stu_stain_model(&self, object: &SceneObject) -> bool {
        let (Some(document), Some(registry)) = (self.document.as_ref(), self.registry.as_ref())
        else {
            return false;
        };
        let Some(actor) = enemy_actor_definition(registry, &object.factory_name) else {
            return false;
        };
        let Some(manager_name) = object.raw_param("manager_name") else {
            return false;
        };
        let managers = enemy_manager_instances(document, registry);
        let Some(manager_instance) = managers.get(manager_name) else {
            return false;
        };
        let Some(manager) = enemy_manager_definition(registry, &manager_instance.factory_name)
        else {
            return false;
        };
        manager_factory_matches_actor(actor, &manager.factory_name)
            && manager.spawned_actor_class.as_deref() == Some(actor.class_name.as_str())
            && actor_uses_stu_stain(registry, actor)
            && self.stu_stain_available()
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
        any_record(&tables.root, &|record| {
            entry_for_manager(record, manager) && entry_spawns_from_goop(record)
        })
    }

    /// Flips a manager's spawn flag in the enemy table. One undo step.
    pub(super) fn set_manager_spawns_from_goop(&mut self, manager: &str, enabled: bool) {
        let Some(document) = self.document.as_ref() else {
            return;
        };
        let before = document.archive_edits.clone();

        if let Err(error) = self.apply_manager_spawn(manager, enabled) {
            self.log
                .push(format!("Could not update goop spawning: {error}"));
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
        self.log.push(match enabled {
            true => format!("{manager} now spawns its enemies from painted goop."),
            false => format!("{manager} no longer spawns from goop."),
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
    pub(super) fn bake_stu_stain(&mut self) {
        const DUMMY_TEXTURE: &str = "H_ma_rak_dummy";
        const STAIN_PATH: &[u8] = b"map/pollution/H_ma_rak.bti";
        const STAIN_MATERIAL: &str = "_mat_body_top1";

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

        let Some(document) = self.document.as_mut() else {
            return;
        };
        let mut baked_models = 0usize;
        for edit in &mut document.archive_edits.resources {
            let StageResourceDocument::Model(model) = &mut edit.document else {
                continue;
            };
            if !model.has_named_texture(DUMMY_TEXTURE)
                || !model.can_pin_material_konst_alpha_half(STAIN_MATERIAL)
            {
                continue;
            }
            let mut baked = model.clone();
            let pinned = match baked.pin_material_konst_alpha_half(STAIN_MATERIAL) {
                Ok(count) if count > 0 => count,
                Ok(_) => continue,
                Err(error) => {
                    self.log.push(format!(
                        "Could not pin the stain blend in {}: {error}",
                        String::from_utf8_lossy(&edit.raw_resource_path)
                    ));
                    continue;
                }
            };
            let replaced = match baked.replace_named_texture_from_bti(DUMMY_TEXTURE, &stain) {
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
            *model = baked;
            baked_models += 1;
            self.log.push(format!(
                "Pinned {pinned} active stain selector(s) in {}.",
                String::from_utf8_lossy(&edit.raw_resource_path)
            ));
        }

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
            matches!(
                &edit.document,
                StageResourceDocument::Model(model)
                    if model.has_named_texture(DUMMY_TEXTURE)
                        && model.can_pin_material_konst_alpha_half(STAIN_MATERIAL)
            )
        })
    }

    /// Whether any copied Stu model carries the baked stain.
    pub(super) fn stu_stain_baked(&self) -> bool {
        const STAIN_MATERIAL: &str = "_mat_body_top1";
        let Some(document) = self.document.as_ref() else {
            return false;
        };
        document.archive_edits.resources.iter().any(|edit| {
            matches!(
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

    /// The Spawning section of the goop inspector.
    pub(super) fn gooble_spawn_section(&mut self, ui: &mut egui::Ui) {
        ui.separator();
        ui.strong("Spawnable entities (TConductor)");
        let entities = self.goop_spawnable_entities();
        if entities.is_empty() {
            ui.label(
                "Place an enemy from the content browser first; its decomp-identified enemy \
                 manager can be flagged to emerge from painted goop. Goobles are NameKuri.",
            );
            return;
        }
        for entity in entities {
            let mut enabled = self.manager_spawns_from_goop(&entity.manager_name);
            if ui
                .checkbox(
                    &mut enabled,
                    format!(
                        "{} \u{2014} {}{}",
                        entity.display_name,
                        entity.manager_name,
                        if entity.manager_present {
                            ""
                        } else {
                            " (remove stale flag)"
                        }
                    ),
                )
                .on_hover_text(if entity.manager_present {
                    "Writes the retail enemy table: the conductor periodically picks a spot \
                         near Mario and, if that spot is goop, relocates one of this manager's \
                         enemies there."
                } else {
                    "This flagged table entry no longer has a TEnemyManager in map/scene.bin. \
                         Uncheck it; export also clears stale entries defensively."
                })
                .changed()
                && (entity.manager_present || !enabled)
            {
                self.set_manager_spawns_from_goop(&entity.manager_name, enabled);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
    fn stu_stain_identification_excludes_other_users_of_the_dummy_texture() {
        fn actor(factory_name: &str, class_name: &str) -> sms_schema::EnemyActorDefinition {
            sms_schema::EnemyActorDefinition {
                factory_name: factory_name.to_string(),
                class_name: class_name.to_string(),
                model_index: None,
                fallback_models: Vec::new(),
                primary_model: None,
                named_models: Vec::new(),
                indexed_models: Vec::new(),
                manager_factories: Vec::new(),
                runtime_uniform_scale: None,
            }
        }
        fn replacement(
            factory_name: &str,
            source_file: &str,
        ) -> sms_schema::RuntimeTextureReplacementDefinition {
            sms_schema::RuntimeTextureReplacementDefinition {
                factory_name: factory_name.to_string(),
                dummy_texture_name: "H_ma_rak_dummy".to_string(),
                resource_path: "/scene/map/pollution/H_ma_rak.bti".to_string(),
                source_file: source_file.to_string(),
            }
        }

        let stu = actor("HamuKuri", "THamuKuri");
        let boss = actor("BossGesso", "TBossGesso");
        let mut registry = enemy_registry();
        registry.runtime_texture_replacements = vec![
            replacement("HamuKuri", "src/Enemy/hamukuri.cpp"),
            replacement("BossGesso", "src/Enemy/bossgesso.cpp"),
        ];

        assert!(actor_uses_stu_stain(&registry, &stu));
        assert!(!actor_uses_stu_stain(&registry, &boss));
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
