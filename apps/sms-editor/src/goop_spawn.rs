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
/// Retail weight on every bianco entry.
const GOOBLE_SPAWN_WEIGHT: u32 = 100;

fn jdrama_field(name: &str, value: sms_formats::JDramaFieldValue) -> sms_formats::JDramaField {
    sms_formats::JDramaField {
        name: name.to_string(),
        value,
    }
}

fn gooble_enemy_info_record(flags: i32) -> sms_formats::JDramaRecord {
    sms_formats::JDramaRecord {
        type_name: "StageEnemyInfo".to_string(),
        name: ENEMY_INFO_NAME.to_string(),
        payload: sms_formats::JDramaRecordPayload::Fields {
            fields: vec![
                jdrama_field(
                    "manager_name",
                    sms_formats::JDramaFieldValue::String(GOOBLE_MANAGER_NAME.to_string()),
                ),
                jdrama_field("flags", sms_formats::JDramaFieldValue::I32(flags)),
                jdrama_field(
                    "weight",
                    sms_formats::JDramaFieldValue::U32(GOOBLE_SPAWN_WEIGHT),
                ),
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

fn is_gooble_entry(record: &sms_formats::JDramaRecord) -> bool {
    record.type_name == "StageEnemyInfo"
        && matches!(
            record_field(record, "manager_name"),
            Some(sms_formats::JDramaFieldValue::String(manager))
                if manager == GOOBLE_MANAGER_NAME
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
fn apply_gooble_spawn_to_tables(tables: &mut sms_formats::JDramaDocument, enabled: bool) -> bool {
    let mut entry_path = Vec::new();
    if find_record_path(&tables.root, &mut entry_path, &is_gooble_entry) {
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
    children.push(gooble_enemy_info_record(SPAWN_FROM_GOOP_FLAG));
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

impl SmsEditorApp {
    /// Whether the effective enemy table flags Goobles to generate from goop.
    pub(super) fn gooble_spawn_enabled(&self) -> bool {
        let Some(document) = self.document.as_ref() else {
            return false;
        };
        let Ok(Some(StageResourceDocument::Placement(tables))) =
            document.effective_resource_clone(TABLES_PATH)
        else {
            return false;
        };
        any_record(&tables.root, &|record| {
            is_gooble_entry(record) && entry_spawns_from_goop(record)
        })
    }

    /// Flips the spawn flag, creating the enemy table and the manager record
    /// when the stage lacks them. One undo step.
    pub(super) fn set_gooble_spawn(&mut self, enabled: bool) {
        let Some(document) = self.document.as_ref() else {
            return;
        };
        let before = document.archive_edits.clone();

        if let Err(error) = self.apply_gooble_spawn(enabled) {
            self.log
                .push(format!("Could not update Gooble spawning: {error}"));
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
            true => "Goobles now spawn from painted goop.".to_string(),
            false => "Goobles no longer spawn from goop.".to_string(),
        });
    }

    fn apply_gooble_spawn(&mut self, enabled: bool) -> Result<(), String> {
        let document = self
            .document
            .as_mut()
            .ok_or_else(|| "no stage is open".to_string())?;

        // The flag, in tables.bin. Editing the effective document keeps any
        // audio-cube edits already upserted into the same file.
        let mut tables = match document
            .effective_resource_clone(TABLES_PATH)
            .map_err(|error| error.to_string())?
        {
            Some(StageResourceDocument::Placement(tables)) => tables,
            Some(_) => return Err("map/tables.bin is not typed placement data".to_string()),
            None => fresh_tables_document(),
        };
        if apply_gooble_spawn_to_tables(&mut tables, enabled) {
            document.archive_edits.upsert_resource(
                TABLES_PATH.to_vec(),
                StageResourceDocument::Placement(tables),
            );
        }

        // An earlier revision inserted a bare NameKuriManager record here. A
        // manager is not just a record: TObjManager::load resolves its
        // character archive by name and dereferences the result, so a manager
        // without its resources crashes the stage on load. Placing the manager
        // is the content browser's job, where the authoring template carries
        // every resource the runtime touches. Strip any bare insert an earlier
        // revision left behind, so an affected stage heals on the next toggle.
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

    /// Whether the effective scene carries the Gooble manager, from any
    /// source: the base archive or a placed template.
    pub(super) fn gooble_manager_present(&self) -> bool {
        let Some(document) = self.document.as_ref() else {
            return false;
        };
        let placed = document
            .archive_edits
            .placement_inserts
            .iter()
            .any(|insert| {
                insert.raw_resource_path == SCENE_PATH
                    && any_record(&insert.record, &|record| record.name == GOOBLE_MANAGER_NAME)
            });
        if placed {
            return true;
        }
        let Ok(Some(StageResourceDocument::Placement(scene))) =
            document.effective_resource_clone(SCENE_PATH)
        else {
            return false;
        };
        any_record(&scene.root, &|record| record.name == GOOBLE_MANAGER_NAME)
    }

    /// The Spawning section of the goop inspector.
    pub(super) fn gooble_spawn_section(&mut self, ui: &mut egui::Ui) {
        ui.separator();
        ui.strong("Spawning");
        let mut enabled = self.gooble_spawn_enabled();
        if ui
            .checkbox(&mut enabled, "Spawn Goobles from goop")
            .on_hover_text(
                "Writes the retail enemy table: the conductor periodically picks a spot near \
                 Mario and spawns a Gooble there if that spot is goop. The stage also needs \
                 a NameKuri Manager placed from the content browser to supply the actors.",
            )
            .changed()
        {
            self.set_gooble_spawn(enabled);
        }
        if enabled && !self.gooble_manager_present() {
            ui.colored_label(
                egui::Color32::from_rgb(245, 180, 70),
                "No NameKuri Manager in this stage: nothing will spawn. Place one from the \
                 content browser (search \"namekuri\").",
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A fresh stage gains a table whose only entry is the flagged Gooble one,
    /// and disabling clears the bit without deleting anything.
    #[test]
    fn the_flag_round_trips_through_an_empty_table() {
        let mut tables = fresh_tables_document();
        assert!(apply_gooble_spawn_to_tables(&mut tables, true));
        assert!(any_record(&tables.root, &|record| {
            is_gooble_entry(record) && entry_spawns_from_goop(record)
        }));
        // Setting it again is a no-op, not a duplicate entry.
        assert!(!apply_gooble_spawn_to_tables(&mut tables, true));

        assert!(apply_gooble_spawn_to_tables(&mut tables, false));
        assert!(any_record(&tables.root, &|record| is_gooble_entry(record)));
        assert!(!any_record(&tables.root, &|record| {
            is_gooble_entry(record) && entry_spawns_from_goop(record)
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
                    children: vec![gooble_enemy_info_record(0)],
                },
            });
        }
        assert!(apply_gooble_spawn_to_tables(&mut tables, true));
        let mut entries = 0;
        fn count(record: &sms_formats::JDramaRecord, entries: &mut usize) {
            if is_gooble_entry(record) {
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
