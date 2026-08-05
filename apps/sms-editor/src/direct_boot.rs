use std::collections::{BTreeMap, BTreeSet, VecDeque};

const DOL_HEADER_SIZE: usize = 0x100;
const DOL_TEXT_SECTION_COUNT: usize = 7;
const DOL_DATA_SECTION_COUNT: usize = 11;
const DOL_TEXT_FILE_OFFSETS: usize = 0x00;
const DOL_DATA_FILE_OFFSETS: usize = 0x1c;
const DOL_TEXT_ADDRESSES: usize = 0x48;
const DOL_DATA_ADDRESSES: usize = 0x64;
const DOL_TEXT_SIZES: usize = 0x90;
const DOL_DATA_SIZES: usize = 0xac;
const DOL_BSS_ADDRESS: usize = 0xd8;
const DOL_BSS_SIZE: usize = 0xdc;
const DOL_ENTRY_POINT: usize = 0xe0;

const PPC_NOP: u32 = 0x6000_0000;
const PPC_BLR: u32 = 0x4e80_0020;
const DIRECT_BOOT_FLAG: u16 = 0x534d;
const POST_NLOGO_STATE: i16 = 5;
const FILE_ALIGNMENT: u32 = 0x20;
const MIN_STAGE_MUSIC_STACK_GAP: u32 = 0x100;
const THIS_SEARCH_WORDS: usize = 0x100;
const STATE_COMPARE_SEARCH_WORDS: usize = 0x40;
const NLOGO_DIRECT_SEARCH_WORDS: usize = 0x40;
const MOVIE_SEARCH_WORDS: usize = 0xc0;
const ENTRY_BL_SEARCH_WORDS: usize = 0x40;
const INIT_REGISTER_SEARCH_WORDS: usize = 0x40;
const TRANSITION_CAVE_WORDS: usize = 7;
const MOVIE_PRIMARY_CAVE_WORDS: usize = 7;
const MOVIE_SECONDARY_CAVE_WORDS: usize = 3;
const DEATH_RESTART_CAVE_WORDS: usize = 7;
const TRANSITION_WORD_COUNT: u32 = 8;
const MOVIE_WRAPPER_WORD_COUNT: u32 = 9;
const DIRECT_BOOT_MARKER: &[u8] = b"SMS_EDITOR_DIRECT_BOOT_V1\0";
const STAGE_MUSIC_MARKER: &[u8] = b"SMS_EDITOR_STAGE_MUSIC_V1\0";
const DIALOGUE_MARKER: &[u8] = b"GRAFFITO_DIALOGUE_OVERRIDE_V1\0";
const BALLOON_DIALOGUE_MARKER: &[u8] = b"GRAFFITO_BALLOON_OVERRIDE_V1\0";
const MAX_STAGE_MUSIC_OVERRIDES: usize = 128;
const MAX_DIALOGUE_OVERRIDES: usize = 4096;
const DIALOGUE_TABLE_ENTRY_SIZE: usize = 36;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct RuntimeStageTarget {
    pub(super) area_index: u8,
    pub(super) scenario_index: u8,
    pub(super) archive_name: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum RuntimeMusicRoleOverride {
    GameDefault,
    Silent,
    Track {
        bgm_id: u32,
        wave_scene_id: Option<u32>,
    },
}

impl RuntimeMusicRoleOverride {
    fn bgm_id(self) -> Option<u32> {
        match self {
            Self::GameDefault => None,
            Self::Silent => Some(0xffff_fff0),
            Self::Track { bgm_id, .. } => Some(bgm_id),
        }
    }

    fn track(self) -> Option<(u32, Option<u32>)> {
        match self {
            Self::Track {
                bgm_id,
                wave_scene_id,
            } => Some((bgm_id, wave_scene_id)),
            Self::GameDefault | Self::Silent => None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum RuntimeStageMusicTransition {
    GameDefault,
    Disabled,
    InsideCrossfade,
    SwitchRegion,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct RuntimeStageMusicOverride {
    pub(super) area_index: u8,
    pub(super) scenario_index: u8,
    pub(super) main: RuntimeMusicRoleOverride,
    pub(super) entrance: RuntimeMusicRoleOverride,
    pub(super) inside: RuntimeMusicRoleOverride,
    pub(super) switch_a: RuntimeMusicRoleOverride,
    pub(super) switch_b: RuntimeMusicRoleOverride,
    pub(super) transition: RuntimeStageMusicTransition,
}

/// A fully resolved runtime dialogue redirect.
///
/// `factory_name` is retained for diagnostics only. The runtime object does not
/// retain its JDrama factory string, so callers must resolve that factory to the
/// decomp-derived `actor_type` before constructing this low-level patch input.
/// `runtime_name_shift_jis` is compared byte-for-byte with `TNameRef::mName`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct RuntimeDialogueOverride {
    pub(super) area_index: u8,
    pub(super) scenario_index: u8,
    /// `TLiveActor::mInstanceIndex`, assigned by the actor's exact live manager.
    pub(super) manager_instance_index: u16,
    pub(super) original_message_id: u32,
    pub(super) replacement_message_id: u32,
    pub(super) factory_name: String,
    pub(super) actor_type: u32,
    pub(super) runtime_name_shift_jis: Vec<u8>,
    pub(super) reset_position_bits: [u32; 3],
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct RuntimeBalloonOverride {
    pub(super) area_index: u8,
    pub(super) scenario_index: u8,
    /// `TLiveActor::mInstanceIndex`, assigned by the actor's exact live manager.
    pub(super) manager_instance_index: u16,
    pub(super) original_message_id: u32,
    pub(super) replacement_message_id: u32,
    pub(super) factory_name: String,
    pub(super) actor_type: u32,
    pub(super) runtime_name_shift_jis: Vec<u8>,
    pub(super) reset_position_bits: [u32; 3],
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
struct RuntimeDialogueGuardKey {
    area_index: u8,
    scenario_index: u8,
    manager_instance_index: u16,
    original_message_id: u32,
    actor_type: u32,
    runtime_name_shift_jis: Vec<u8>,
    reset_position_bits: [u32; 3],
}

impl RuntimeDialogueOverride {
    fn guard_key(&self) -> RuntimeDialogueGuardKey {
        RuntimeDialogueGuardKey {
            area_index: self.area_index,
            scenario_index: self.scenario_index,
            manager_instance_index: self.manager_instance_index,
            original_message_id: self.original_message_id,
            actor_type: self.actor_type,
            runtime_name_shift_jis: self.runtime_name_shift_jis.clone(),
            reset_position_bits: self.reset_position_bits,
        }
    }
}

impl RuntimeBalloonOverride {
    fn guard_key(&self) -> RuntimeDialogueGuardKey {
        RuntimeDialogueGuardKey {
            area_index: self.area_index,
            scenario_index: self.scenario_index,
            manager_instance_index: self.manager_instance_index,
            original_message_id: self.original_message_id,
            actor_type: self.actor_type,
            runtime_name_shift_jis: self.runtime_name_shift_jis.clone(),
            reset_position_bits: self.reset_position_bits,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub(super) enum RuntimeSoundAssignmentKind {
    MapStatic,
    Graph,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct RuntimeSoundAssignment {
    pub(super) kind: RuntimeSoundAssignmentKind,
    pub(super) source_name: String,
    pub(super) original_sound_id: u32,
    pub(super) sound_id: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub(super) enum RuntimeGoopManagerPatch {
    /// `TPoiHanaManager::createEnemyInstance` normally returns null outside
    /// area 0x38, after which `initSetEnemies` dereferences `getObj(0)`.
    PoiHanaAnyArea,
    /// `TConductor::init` deliberately skips HinoKuri2 while constructing
    /// ordinary reusable manager pools.
    HinoKuri2Pool,
    /// `TMameGesso::reset` restores its original scene position after
    /// `TSpineEnemy::resetToPosition` has selected a pollution spawn point.
    MameGessoRelocatableReset,
    /// Make the PoiHana manager construct the retail red subclass for an
    /// editor-authored red PoiHana pollution pool.
    PoiHanaRedPool,
    /// `TTelesa::reset` hides a normal Telesa and leaves its fade state
    /// invisible, so a pollution-spawned pool member never becomes visible.
    TelesaVisibleReset,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct StageMusicDol {
    pub(super) bytes: Vec<u8>,
    pub(super) hook_address: u32,
    pub(super) stub_address: u32,
}

struct StageMusicStub {
    words: Vec<u32>,
    wrapper_word: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct DirectBootDol {
    pub(super) bytes: Vec<u8>,
    pub(super) logo_bypass_address: u32,
    pub(super) hook_address: u32,
    pub(super) movie_hook_address: u32,
    pub(super) death_hook_address: u32,
    pub(super) death_stub_address: u32,
    pub(super) stub_address: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct DolSection {
    text: bool,
    slot: usize,
    file_offset: u32,
    address: u32,
    size: u32,
}

impl DolSection {
    fn file_end(self) -> Result<u32, String> {
        self.file_offset
            .checked_add(self.size)
            .ok_or_else(|| format!("DOL section {} file range overflows", self.label()))
    }

    fn address_end(self) -> Result<u32, String> {
        self.address
            .checked_add(self.size)
            .ok_or_else(|| format!("DOL section {} address range overflows", self.label()))
    }

    fn label(self) -> String {
        format!("{}[{}]", if self.text { "text" } else { "data" }, self.slot)
    }
}

#[derive(Debug)]
struct DolImage {
    sections: Vec<DolSection>,
    bss: Option<(u32, u32)>,
    entry_point: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct WordAnchor {
    section: DolSection,
    word_index: usize,
}

impl WordAnchor {
    fn address(self) -> Result<u32, String> {
        let byte_offset = u32::try_from(self.word_index)
            .ok()
            .and_then(|index| index.checked_mul(4))
            .ok_or_else(|| "DOL word offset overflows u32".to_string())?;
        self.section
            .address
            .checked_add(byte_offset)
            .ok_or_else(|| "DOL word address overflows u32".to_string())
    }

    fn file_offset(self) -> Result<usize, String> {
        let byte_offset = self
            .word_index
            .checked_mul(4)
            .ok_or_else(|| "DOL word file offset overflows usize".to_string())?;
        usize::try_from(self.section.file_offset)
            .ok()
            .and_then(|offset| offset.checked_add(byte_offset))
            .ok_or_else(|| "DOL word file offset overflows usize".to_string())
    }
}

#[derive(Debug, Clone, Copy)]
struct NlogoHook {
    anchor: WordAnchor,
    this_register: u8,
    next_state_register: u8,
}

#[derive(Debug, Clone, Copy)]
struct NlogoDirectorBypass {
    branch_anchor: WordAnchor,
    completion_anchor: WordAnchor,
}

#[derive(Debug, Clone, Copy)]
struct NlogoSetupBypass {
    case_anchor: WordAnchor,
    resume_address: u32,
}

#[derive(Debug, Clone, Copy)]
struct NextAreaSetter {
    anchor: WordAnchor,
    base_register: u8,
    next_area_offset: i16,
}

#[derive(Debug, Clone, Copy)]
struct MovieHook {
    call_anchor: WordAnchor,
    original_target: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct CodeCave {
    anchor: WordAnchor,
    word_count: usize,
}

#[derive(Debug, Clone, Copy)]
struct DirectBootCaves {
    transition: CodeCave,
    movie_primary: CodeCave,
    movie_secondary: CodeCave,
    death_restart: CodeCave,
}

#[derive(Debug, Clone, Copy)]
struct DeathRestartHook {
    call_anchor: WordAnchor,
    current_area_register: u8,
}

#[derive(Debug, Clone, Copy)]
struct SoundStageHook {
    dispatch_anchor: WordAnchor,
    enter_stage_anchor: WordAnchor,
    enter_stage_target: u32,
    area_register: u8,
    scenario_register: u8,
    ms_stg_offset: i16,
}

#[derive(Debug, Clone, Copy)]
struct DialogueHook {
    replay_anchor: WordAnchor,
    replay_instruction: u32,
    this_register: u8,
    director_sda_offset: i16,
}

#[derive(Debug, Clone, Copy)]
struct BalloonDialogueHook {
    entry_anchor: WordAnchor,
    replay_instruction: u32,
    director_sda_offset: i16,
}

pub(super) fn patch_sms_sound_assignments_dol(
    source: &[u8],
    assignments: &[RuntimeSoundAssignment],
) -> Result<Vec<u8>, String> {
    if assignments.is_empty() {
        return Ok(source.to_vec());
    }
    let image = parse_dol(source)?;
    let mut seen = std::collections::BTreeSet::new();
    let mut patches = Vec::with_capacity(assignments.len());
    for assignment in assignments {
        if assignment.source_name.is_empty() || assignment.source_name.as_bytes().contains(&0) {
            return Err("Sound assignment source name is empty or contains NUL".to_string());
        }
        if assignment.original_sound_id > u16::MAX.into() || assignment.sound_id > u16::MAX.into() {
            return Err(format!(
                "Sound assignment '{}' is outside Sunshine's 16-bit SE identifier range",
                assignment.source_name
            ));
        }
        let identity = (assignment.kind.clone(), assignment.source_name.clone());
        if !seen.insert(identity) {
            return Err(format!(
                "Duplicate sound assignment for '{}'",
                assignment.source_name
            ));
        }
        let field_offset = match assignment.kind {
            RuntimeSoundAssignmentKind::MapStatic => 48,
            RuntimeSoundAssignmentKind::Graph => 4,
        };
        let addresses = find_c_string_addresses(source, &image, &assignment.source_name)?;
        let mut candidates = Vec::new();
        for address in addresses {
            for section in image
                .sections
                .iter()
                .copied()
                .filter(|section| !section.text)
            {
                let start = usize::try_from(section.file_offset)
                    .map_err(|_| "DOL data section offset does not fit usize".to_string())?;
                let section_end = usize::try_from(section.file_end()?)
                    .map_err(|_| "DOL data section end does not fit usize".to_string())?;
                for pointer_offset in (start..section_end.saturating_sub(3)).step_by(4) {
                    if read_be_u32(source, pointer_offset)? != address {
                        continue;
                    }
                    let sound_offset = pointer_offset
                        .checked_add(field_offset)
                        .ok_or_else(|| "Sound-table field offset overflows usize".to_string())?;
                    if sound_offset
                        .checked_add(4)
                        .is_some_and(|field_end| field_end <= section_end)
                        && read_be_u32(source, sound_offset)? == assignment.original_sound_id
                    {
                        candidates.push(sound_offset);
                    }
                }
            }
        }
        candidates.sort_unstable();
        candidates.dedup();
        if candidates.len() != 1 {
            return Err(format!(
                "Could not uniquely locate the {:?} sound-table row '{}' with original SE 0x{:04X}; found {} candidate(s)",
                assignment.kind,
                assignment.source_name,
                assignment.original_sound_id,
                candidates.len()
            ));
        }
        patches.push((candidates[0], assignment.sound_id));
    }
    let mut bytes = source.to_vec();
    for (offset, sound_id) in patches {
        write_be_u32(&mut bytes, offset, sound_id)?;
    }
    parse_dol(&bytes)?;
    Ok(bytes)
}

pub(super) fn patch_sms_goop_enemy_managers_dol(
    source: &[u8],
    requested: &BTreeSet<RuntimeGoopManagerPatch>,
) -> Result<Vec<u8>, String> {
    if requested.is_empty() {
        return Ok(source.to_vec());
    }
    let image = parse_dol(source)?;
    let mut bytes = source.to_vec();

    if requested.contains(&RuntimeGoopManagerPatch::PoiHanaAnyArea) {
        let mut candidates = Vec::new();
        for section in image
            .sections
            .iter()
            .copied()
            .filter(|section| section.text)
        {
            let words = section_words(source, section)?;
            for word_index in 0..words.len().saturating_sub(6) {
                let sequence = &words[word_index..word_index + 7];
                if sequence[0] == 0x8803_000e
                    && sequence[1] == 0x2800_0038
                    && (sequence[2] == 0x4082_002c || sequence[2] == PPC_NOP)
                    && sequence[3] == 0x3860_01cc
                    && is_relative_bl(sequence[4])
                    && sequence[5] == 0x7c7f_1b79
                    && sequence[6] == 0x4182_0014
                {
                    candidates.push(WordAnchor {
                        section,
                        word_index: word_index + 2,
                    });
                }
            }
        }
        let anchor = require_unique_anchor(
            candidates,
            "TPoiHanaManager area-gated createEnemyInstance branch",
        )?;
        write_be_u32(&mut bytes, anchor.file_offset()?, PPC_NOP)?;
    }

    if requested.contains(&RuntimeGoopManagerPatch::HinoKuri2Pool) {
        let mut candidates = Vec::new();
        for section in image
            .sections
            .iter()
            .copied()
            .filter(|section| section.text)
        {
            let words = section_words(source, section)?;
            for word_index in 0..words.len().saturating_sub(14) {
                let sequence = &words[word_index..word_index + 15];
                if sequence[0] == 0x2803_0000
                    && (sequence[1] == 0x4082_0058 || sequence[1] == PPC_NOP)
                    && sequence[2] == 0x8081_0068
                    && sequence[3] == 0x387e_0114
                    && sequence[4] == 0x83a4_0008
                    && is_relative_bl(sequence[5])
                    && sequence[6] == 0x819d_0000
                    && sequence[7] == 0x3883_0000
                    && sequence[8] == 0x387d_0000
                    && sequence[9] == 0x818c_001c
                    && sequence[10] == 0x38be_0114
                    && sequence[11] == 0x7d88_03a6
                    && sequence[12] == 0x4e80_0021
                    && sequence[13] == 0x2803_0000
                    && sequence[14] == 0x4082_0024
                {
                    candidates.push(WordAnchor {
                        section,
                        word_index: word_index + 1,
                    });
                }
            }
        }
        let anchor = require_unique_anchor(
            candidates,
            "TConductor HinoKuri2 reusable-pool exclusion branch",
        )?;
        write_be_u32(&mut bytes, anchor.file_offset()?, PPC_NOP)?;
    }

    if requested.contains(&RuntimeGoopManagerPatch::MameGessoRelocatableReset) {
        let mut candidates = Vec::new();
        for section in image
            .sections
            .iter()
            .copied()
            .filter(|section| section.text)
        {
            let words = section_words(source, section)?;
            for word_index in 0..words.len().saturating_sub(9) {
                let sequence = &words[word_index..word_index + 10];
                if sequence[0] == 0x807d_01d4
                    && sequence[1] == 0x801d_01d8
                    && (sequence[2] == 0x907d_0010 || sequence[2] == PPC_NOP)
                    && (sequence[3] == 0x901d_0014 || sequence[3] == PPC_NOP)
                    && sequence[4] == 0x801d_01dc
                    && (sequence[5] == 0x901d_0018 || sequence[5] == PPC_NOP)
                    && sequence[6] == 0xc03d_0014
                    && (sequence[7] & 0xffff_0000) == 0xc002_0000
                    && sequence[8] == 0xec01_002a
                    && sequence[9] == 0xd01d_0014
                {
                    candidates.push(WordAnchor {
                        section,
                        word_index,
                    });
                }
            }
        }
        let anchor =
            require_unique_anchor(candidates, "TMameGesso reset original-position restore")?;
        for relative_word in [2usize, 3, 5] {
            write_be_u32(
                &mut bytes,
                WordAnchor {
                    section: anchor.section,
                    word_index: anchor.word_index + relative_word,
                }
                .file_offset()?,
                PPC_NOP,
            )?;
        }
    }

    if requested.contains(&RuntimeGoopManagerPatch::PoiHanaRedPool) {
        let mut factory_candidates = Vec::new();
        for section in image
            .sections
            .iter()
            .copied()
            .filter(|section| section.text)
        {
            let words = section_words(source, section)?;
            for word_index in 0..words.len().saturating_sub(10) {
                let sequence = &words[word_index..word_index + 11];
                if sequence[0] == 0x8803_000e
                    && sequence[1] == 0x2800_0038
                    && (sequence[2] == 0x4082_002c || sequence[2] == PPC_NOP)
                    && sequence[3] == 0x3860_01cc
                    && is_relative_bl(sequence[4])
                    && sequence[5] == 0x7c7f_1b79
                    && sequence[6] == 0x4182_0014
                    && is_relative_bl(sequence[10])
                {
                    factory_candidates.push(WordAnchor {
                        section,
                        word_index,
                    });
                }
            }
        }
        let factory = require_unique_anchor(
            factory_candidates,
            "TPoiHanaManager actor-construction call",
        )?;
        let factory_words = section_words(source, factory.section)?;
        let constructor_word_index = factory.word_index + 10;
        let constructor_call = WordAnchor {
            section: factory.section,
            word_index: constructor_word_index,
        };
        let base_constructor = decode_branch_target(
            factory_words[constructor_word_index],
            constructor_call.address()?,
        )?;

        let mut subclass_vtables = BTreeSet::new();
        for section in image
            .sections
            .iter()
            .copied()
            .filter(|section| section.text)
        {
            let words = section_words(source, section)?;
            for word_index in 0..words.len().saturating_sub(7) {
                let sequence = &words[word_index..word_index + 8];
                let call_address = WordAnchor {
                    section,
                    word_index: word_index + 2,
                }
                .address()?;
                if !is_relative_bl(sequence[2])
                    || decode_branch_target(sequence[2], call_address)? != base_constructor
                    || sequence[3] & 0xffff_0000 != 0x3c60_0000
                    || sequence[4] & 0xffff_0000 != 0x3863_0000
                    || sequence[5] & 0xffe0_ffff != 0x9060_0000
                    || sequence[6] != 0x3803_0024
                    || sequence[7] & 0xffe0_ffff != 0x9000_0020
                {
                    continue;
                }
                let high = (sequence[3] & 0xffff) << 16;
                let low = i32::from(sequence[4] as i16) as u32;
                let vtable = high.wrapping_add(low);
                if image.sections.iter().any(|candidate| {
                    !candidate.text
                        && vtable >= candidate.address
                        && vtable < candidate.address.saturating_add(candidate.size)
                }) {
                    subclass_vtables.insert(vtable);
                }
            }
        }
        if subclass_vtables.len() != 1 {
            return Err(format!(
                "Expected one inline TPoiHanaRed vtable assignment, found {} candidate(s)",
                subclass_vtables.len()
            ));
        }
        let red_vtable = subclass_vtables
            .into_iter()
            .next()
            .expect("one candidate was checked");
        let text_slot = (0..DOL_TEXT_SECTION_COUNT)
            .find(|slot| {
                !image
                    .sections
                    .iter()
                    .any(|section| section.text && section.slot == *slot)
            })
            .ok_or_else(|| "The DOL has no unused text section for red PoiHana".to_string())?;
        let file_offset = align_up_usize(source.len(), FILE_ALIGNMENT as usize)?;
        let loaded_end = image
            .sections
            .iter()
            .map(|section| section.address_end())
            .chain(image.bss.map(|(_, end)| Ok(end)))
            .collect::<Result<Vec<_>, _>>()?
            .into_iter()
            .max()
            .ok_or_else(|| "The DOL has no loaded sections".to_string())?;
        let stub_address = align_up_u32(loaded_end, FILE_ALIGNMENT)?;
        let vtable_high = ((red_vtable.wrapping_add(0x8000)) >> 16) as i16;
        let vtable_low = red_vtable as u16 as i16;
        let stub = vec![
            0x7c08_02a6, // mflr r0
            encode_d_form(37, 1, 1, -0x10),
            encode_d_form(36, 0, 1, 0x14),
            encode_branch(stub_address + 12, base_constructor, true)?,
            encode_d_form(15, 4, 0, vtable_high),
            encode_d_form(14, 4, 4, vtable_low),
            encode_d_form(36, 4, 3, 0),
            encode_d_form(14, 4, 4, 0x24),
            encode_d_form(36, 4, 3, 0x20),
            encode_d_form(32, 0, 1, 0x14),
            encode_d_form(14, 1, 1, 0x10),
            0x7c08_03a6, // mtlr r0
            PPC_BLR,
        ];
        let payload_size = align_up_usize(stub.len() * 4, FILE_ALIGNMENT as usize)?;
        let stub_end = stub_address
            .checked_add(u32::try_from(payload_size).map_err(|_| {
                "Red PoiHana stub size does not fit the DOL address space".to_string()
            })?)
            .ok_or_else(|| "Red PoiHana stub range overflows u32".to_string())?;
        let stack_top = find_stack_top(source, &image)?;
        if stub_end
            .checked_add(MIN_STAGE_MUSIC_STACK_GAP)
            .is_none_or(|safe_end| safe_end > stack_top)
        {
            return Err(format!(
                "Red PoiHana stub 0x{stub_address:08X}..0x{stub_end:08X} is too close to the original stack top 0x{stack_top:08X}"
            ));
        }
        reject_injected_range_overlap(&image, stub_address, stub_end)?;
        bytes.resize(file_offset, 0);
        for word in &stub {
            bytes.extend_from_slice(&word.to_be_bytes());
        }
        bytes.resize(file_offset + payload_size, 0);
        write_be_u32(
            &mut bytes,
            DOL_TEXT_FILE_OFFSETS + text_slot * 4,
            u32::try_from(file_offset)
                .map_err(|_| "Red PoiHana file offset does not fit u32".to_string())?,
        )?;
        write_be_u32(&mut bytes, DOL_TEXT_ADDRESSES + text_slot * 4, stub_address)?;
        write_be_u32(
            &mut bytes,
            DOL_TEXT_SIZES + text_slot * 4,
            u32::try_from(payload_size)
                .map_err(|_| "Red PoiHana payload size does not fit u32".to_string())?,
        )?;
        write_be_u32(
            &mut bytes,
            constructor_call.file_offset()?,
            encode_branch(constructor_call.address()?, stub_address, true)?,
        )?;
    }

    if requested.contains(&RuntimeGoopManagerPatch::TelesaVisibleReset) {
        let mut candidates = Vec::new();
        for section in image
            .sections
            .iter()
            .copied()
            .filter(|section| section.text)
        {
            let words = section_words(source, section)?;
            for word_index in 0..words.len().saturating_sub(10) {
                let sequence = &words[word_index..word_index + 11];
                if word_index >= 10
                    && words[word_index - 10] == 0x7c08_02a6
                    && words[word_index - 9] == 0x9001_0004
                    && words[word_index - 8] == 0x9421_ff90
                    && words[word_index - 7] == 0xdbe1_0068
                    && words[word_index - 6] == 0x93e1_0064
                    && words[word_index - 5] == 0x7c7f_1b78
                    && words[word_index - 4] == 0x93c1_0060
                    && words[word_index - 3] == 0x93a1_005c
                    && words[word_index - 2] == 0x9381_0058
                    && is_relative_bl(words[word_index - 1])
                    && sequence[0] == 0x801f_00f0
                    && sequence[1] == 0x6000_0008
                    && sequence[2] == 0x901f_00f0
                    && sequence[3] == 0x801f_00f0
                    && sequence[4] == 0x6000_0002
                    && sequence[5] == 0x901f_00f0
                    && sequence[6] == 0x807f_0194
                    && sequence[7] == 0x3bc3_0458
                    && sequence[8] == 0xc023_045c
                    && sequence[9] == 0xc003_0458
                    && sequence[10] == 0xefe1_0028
                {
                    candidates.push(WordAnchor {
                        section,
                        word_index: word_index + 4,
                    });
                }
            }
        }
        let hidden_flag = require_unique_anchor(candidates, "TTelesa reset hidden-state setup")?;
        let current_image = parse_dol(&bytes)?;
        let text_slot = (0..DOL_TEXT_SECTION_COUNT)
            .rev()
            .find(|slot| {
                !current_image
                    .sections
                    .iter()
                    .any(|section| section.text && section.slot == *slot)
            })
            .ok_or_else(|| "The DOL has no unused text section for Telesa reset".to_string())?;
        let file_offset = align_up_usize(bytes.len(), FILE_ALIGNMENT as usize)?;
        let loaded_end = current_image
            .sections
            .iter()
            .map(|section| section.address_end())
            .chain(current_image.bss.map(|(_, end)| Ok(end)))
            .collect::<Result<Vec<_>, _>>()?
            .into_iter()
            .max()
            .ok_or_else(|| "The DOL has no loaded sections".to_string())?;
        let stub_address = align_up_u32(loaded_end, FILE_ALIGNMENT)?;
        let stub = [
            0x5400_07fa, // rlwinm r0,r0,0,31,29: clear LIVE_FLAG_HIDDEN
            encode_d_form(14, 12, 0, 0),
            encode_d_form(36, 12, 31, 0x1c0), // mFadeLoopTimer = 0
            encode_d_form(36, 12, 31, 0x1d8), // mFadeTimer = 0
            encode_d_form(14, 12, 0, 1),
            encode_d_form(36, 12, 31, 0x1dc), // mFadeState = FADE_STATE_FADE_IN
            PPC_BLR,
        ];
        let payload_size = align_up_usize(stub.len() * 4, FILE_ALIGNMENT as usize)?;
        let stub_end = stub_address
            .checked_add(u32::try_from(payload_size).map_err(|_| {
                "Telesa reset stub size does not fit the DOL address space".to_string()
            })?)
            .ok_or_else(|| "Telesa reset stub range overflows u32".to_string())?;
        let stack_top = find_stack_top(&bytes, &current_image)?;
        if stub_end
            .checked_add(MIN_STAGE_MUSIC_STACK_GAP)
            .is_none_or(|safe_end| safe_end > stack_top)
        {
            return Err(format!(
                "Telesa reset stub 0x{stub_address:08X}..0x{stub_end:08X} is too close to the original stack top 0x{stack_top:08X}"
            ));
        }
        reject_injected_range_overlap(&current_image, stub_address, stub_end)?;
        bytes.resize(file_offset, 0);
        for word in stub {
            bytes.extend_from_slice(&word.to_be_bytes());
        }
        bytes.resize(file_offset + payload_size, 0);
        write_be_u32(
            &mut bytes,
            DOL_TEXT_FILE_OFFSETS + text_slot * 4,
            u32::try_from(file_offset)
                .map_err(|_| "Telesa reset file offset does not fit u32".to_string())?,
        )?;
        write_be_u32(&mut bytes, DOL_TEXT_ADDRESSES + text_slot * 4, stub_address)?;
        write_be_u32(
            &mut bytes,
            DOL_TEXT_SIZES + text_slot * 4,
            u32::try_from(payload_size)
                .map_err(|_| "Telesa reset payload size does not fit u32".to_string())?,
        )?;
        write_be_u32(
            &mut bytes,
            hidden_flag.file_offset()?,
            encode_branch(hidden_flag.address()?, stub_address, true)?,
        )?;
    }

    parse_dol(&bytes)?;
    Ok(bytes)
}

fn find_c_string_addresses(
    source: &[u8],
    image: &DolImage,
    value: &str,
) -> Result<Vec<u32>, String> {
    let mut needle = value.as_bytes().to_vec();
    needle.push(0);
    let mut addresses = Vec::new();
    for section in image.sections.iter().copied() {
        let start = usize::try_from(section.file_offset)
            .map_err(|_| "DOL section offset does not fit usize".to_string())?;
        let end = usize::try_from(section.file_end()?)
            .map_err(|_| "DOL section end does not fit usize".to_string())?;
        let Some(bytes) = source.get(start..end) else {
            return Err(format!(
                "DOL section {} is outside the file",
                section.label()
            ));
        };
        for (offset, window) in bytes.windows(needle.len()).enumerate() {
            if window == needle {
                addresses.push(
                    section
                        .address
                        .checked_add(u32::try_from(offset).map_err(|_| {
                            "DOL string offset does not fit the address space".to_string()
                        })?)
                        .ok_or_else(|| "DOL string address overflows u32".to_string())?,
                );
            }
        }
    }
    addresses.sort_unstable();
    addresses.dedup();
    Ok(addresses)
}

pub(super) fn patch_sms_dialogue_dol(
    source: &[u8],
    overrides: &[RuntimeDialogueOverride],
    balloon_overrides: &[RuntimeBalloonOverride],
) -> Result<Vec<u8>, String> {
    // Locate the balloon entry while the shared post-selection convergence is
    // still pristine; it supplies the gpMarDirector SDA reference without
    // requiring the separate talk binding to be patchable.
    let balloon_bytes = patch_sms_balloon_dialogue_dol(source, balloon_overrides)?;
    patch_sms_talk_dialogue_dol(&balloon_bytes, overrides)
}

fn patch_sms_talk_dialogue_dol(
    source: &[u8],
    overrides: &[RuntimeDialogueOverride],
) -> Result<Vec<u8>, String> {
    let effective = overrides
        .iter()
        .filter(|override_| override_.original_message_id != override_.replacement_message_id)
        .collect::<Vec<_>>();
    if effective.is_empty() {
        return Ok(source.to_vec());
    }
    if effective.len() > MAX_DIALOGUE_OVERRIDES {
        return Err(format!(
            "Dialogue patch has {} effective overrides; the safe limit is {MAX_DIALOGUE_OVERRIDES}",
            effective.len()
        ));
    }
    if source
        .windows(DIALOGUE_MARKER.len())
        .any(|window| window == DIALOGUE_MARKER)
    {
        return Err("The executable already contains a dialogue override patch".to_string());
    }

    let mut unique = std::collections::BTreeSet::new();
    for override_ in &effective {
        if override_.factory_name.is_empty() {
            return Err("Dialogue override has an empty factory name".to_string());
        }
        if override_.actor_type == 0 {
            return Err(format!(
                "Dialogue override for '{}' has no decomp-derived runtime actor type",
                override_.factory_name
            ));
        }
        if override_.runtime_name_shift_jis.is_empty()
            || override_.runtime_name_shift_jis.contains(&0)
        {
            return Err(format!(
                "Dialogue override for '{}' has an empty runtime name or embedded NUL",
                override_.factory_name
            ));
        }
        if !unique.insert(override_.guard_key()) {
            return Err(format!(
                "Duplicate dialogue override with the same runtime guard for area {}, scenario {}, manager instance {}, original message 0x{:08X}",
                override_.area_index,
                override_.scenario_index,
                override_.manager_instance_index,
                override_.original_message_id
            ));
        }
    }

    let image = parse_dol(source)?;
    let hook = find_dialogue_hook(source, &image)?;
    let data_slot = (0..DOL_DATA_SECTION_COUNT)
        .find(|slot| {
            !image
                .sections
                .iter()
                .any(|section| !section.text && section.slot == *slot)
        })
        .ok_or_else(|| "The DOL has no unused data section for dialogue overrides".to_string())?;
    let file_offset = align_up_usize(source.len(), FILE_ALIGNMENT as usize)?;
    let loaded_end = image
        .sections
        .iter()
        .map(|section| section.address_end())
        .chain(image.bss.map(|(_, end)| Ok(end)))
        .collect::<Result<Vec<_>, _>>()?
        .into_iter()
        .max()
        .ok_or_else(|| "The DOL has no loaded sections".to_string())?;
    let stub_address = align_up_u32(loaded_end, FILE_ALIGNMENT)?;

    // The stub has a fixed instruction count. Build it once to establish the
    // table address, then rebuild with that address materialized in the code.
    let provisional_words = build_dialogue_stub(stub_address, 0, hook, effective.len())?;
    let code_size = provisional_words
        .len()
        .checked_mul(4)
        .ok_or_else(|| "Dialogue stub size overflows usize".to_string())?;
    let table_offset = align_up_usize(code_size, 4)?;
    let table_address = stub_address
        .checked_add(
            u32::try_from(table_offset)
                .map_err(|_| "Dialogue table offset does not fit u32".to_string())?,
        )
        .ok_or_else(|| "Dialogue table address overflows u32".to_string())?;
    let words = build_dialogue_stub(stub_address, table_address, hook, effective.len())?;
    debug_assert_eq!(words.len(), provisional_words.len());

    let table_size = effective
        .len()
        .checked_mul(DIALOGUE_TABLE_ENTRY_SIZE)
        .ok_or_else(|| "Dialogue table size overflows usize".to_string())?;
    let strings_offset = table_offset
        .checked_add(table_size)
        .ok_or_else(|| "Dialogue string-table offset overflows usize".to_string())?;
    let names_size = effective.iter().try_fold(0_usize, |size, override_| {
        size.checked_add(override_.runtime_name_shift_jis.len() + 1)
            .ok_or_else(|| "Dialogue runtime-name table size overflows usize".to_string())
    })?;
    let marker_offset = strings_offset
        .checked_add(names_size)
        .ok_or_else(|| "Dialogue marker offset overflows usize".to_string())?;
    let unaligned_payload_size = marker_offset
        .checked_add(DIALOGUE_MARKER.len())
        .ok_or_else(|| "Dialogue payload size overflows usize".to_string())?;
    let payload_size = align_up_usize(unaligned_payload_size, 4)?;
    let stub_end = stub_address
        .checked_add(
            u32::try_from(payload_size)
                .map_err(|_| "Dialogue payload size does not fit u32".to_string())?,
        )
        .ok_or_else(|| "Dialogue payload address range overflows u32".to_string())?;
    let stack_top = find_stack_top(source, &image)?;
    let safe_stub_end = stub_end
        .checked_add(MIN_STAGE_MUSIC_STACK_GAP)
        .ok_or_else(|| "Dialogue stack guard overflows u32".to_string())?;
    if safe_stub_end > stack_top {
        return Err(format!(
            "Dialogue payload 0x{stub_address:08X}..0x{stub_end:08X} leaves less than 0x{MIN_STAGE_MUSIC_STACK_GAP:X} bytes below the original stack top 0x{stack_top:08X}"
        ));
    }
    reject_injected_range_overlap(&image, stub_address, stub_end)?;

    let mut payload = vec![0_u8; payload_size];
    for (index, word) in words.iter().copied().enumerate() {
        write_be_u32(&mut payload, index * 4, word)?;
    }
    let mut name_cursor = strings_offset;
    for (index, override_) in effective.iter().enumerate() {
        let entry_offset = table_offset + index * DIALOGUE_TABLE_ENTRY_SIZE;
        payload[entry_offset] = override_.area_index;
        payload[entry_offset + 1] = override_.scenario_index;
        payload[entry_offset + 2..entry_offset + 4]
            .copy_from_slice(&override_.manager_instance_index.to_be_bytes());
        write_be_u32(
            &mut payload,
            entry_offset + 4,
            override_.original_message_id,
        )?;
        write_be_u32(
            &mut payload,
            entry_offset + 8,
            override_.replacement_message_id,
        )?;
        write_be_u32(&mut payload, entry_offset + 12, override_.actor_type)?;
        let name_key = override_
            .runtime_name_shift_jis
            .iter()
            .fold(0_u32, |key, byte| {
                key.wrapping_mul(3).wrapping_add(u32::from(*byte))
            }) as u16;
        payload[entry_offset + 16..entry_offset + 18].copy_from_slice(&name_key.to_be_bytes());
        let name_address = stub_address
            .checked_add(
                u32::try_from(name_cursor)
                    .map_err(|_| "Dialogue runtime-name offset does not fit u32".to_string())?,
            )
            .ok_or_else(|| "Dialogue runtime-name address overflows u32".to_string())?;
        write_be_u32(&mut payload, entry_offset + 20, name_address)?;
        for (axis, bits) in override_.reset_position_bits.iter().copied().enumerate() {
            write_be_u32(&mut payload, entry_offset + 24 + axis * 4, bits)?;
        }
        let name_end = name_cursor
            .checked_add(override_.runtime_name_shift_jis.len())
            .ok_or_else(|| "Dialogue runtime-name range overflows usize".to_string())?;
        payload[name_cursor..name_end].copy_from_slice(&override_.runtime_name_shift_jis);
        payload[name_end] = 0;
        name_cursor = name_end + 1;
    }
    payload[marker_offset..marker_offset + DIALOGUE_MARKER.len()].copy_from_slice(DIALOGUE_MARKER);

    let hook_address = hook.replay_anchor.address()?;
    let mut bytes = source.to_vec();
    bytes.resize(file_offset, 0);
    bytes.extend_from_slice(&payload);
    write_be_u32(
        &mut bytes,
        DOL_DATA_FILE_OFFSETS + data_slot * 4,
        u32::try_from(file_offset)
            .map_err(|_| "Dialogue payload file offset does not fit u32".to_string())?,
    )?;
    write_be_u32(&mut bytes, DOL_DATA_ADDRESSES + data_slot * 4, stub_address)?;
    write_be_u32(
        &mut bytes,
        DOL_DATA_SIZES + data_slot * 4,
        u32::try_from(payload_size)
            .map_err(|_| "Dialogue payload size does not fit u32".to_string())?,
    )?;
    write_be_u32(
        &mut bytes,
        hook.replay_anchor.file_offset()?,
        encode_branch(hook_address, stub_address, false)?,
    )?;
    parse_dol(&bytes)?;
    Ok(bytes)
}

fn patch_sms_balloon_dialogue_dol(
    source: &[u8],
    overrides: &[RuntimeBalloonOverride],
) -> Result<Vec<u8>, String> {
    let effective = overrides
        .iter()
        .filter(|override_| override_.original_message_id != override_.replacement_message_id)
        .collect::<Vec<_>>();
    if effective.is_empty() {
        return Ok(source.to_vec());
    }
    if effective.len() > MAX_DIALOGUE_OVERRIDES {
        return Err(format!(
            "Balloon dialogue patch has {} effective overrides; the safe limit is {MAX_DIALOGUE_OVERRIDES}",
            effective.len()
        ));
    }
    if source
        .windows(BALLOON_DIALOGUE_MARKER.len())
        .any(|window| window == BALLOON_DIALOGUE_MARKER)
    {
        return Err(
            "The executable already contains a balloon dialogue override patch".to_string(),
        );
    }
    let mut unique = std::collections::BTreeSet::new();
    for override_ in &effective {
        if override_.factory_name.is_empty() || override_.actor_type == 0 {
            return Err(
                "Balloon dialogue override lacks a factory name or decomp-derived actor type"
                    .to_string(),
            );
        }
        if override_.runtime_name_shift_jis.is_empty()
            || override_.runtime_name_shift_jis.contains(&0)
        {
            return Err(format!(
                "Balloon dialogue override for '{}' has an empty runtime name or embedded NUL",
                override_.factory_name
            ));
        }
        if !unique.insert(override_.guard_key()) {
            return Err(format!(
                "Duplicate balloon override with the same runtime guard for area {}, scenario {}, manager instance {}, original message 0x{:08X}",
                override_.area_index,
                override_.scenario_index,
                override_.manager_instance_index,
                override_.original_message_id
            ));
        }
    }

    let image = parse_dol(source)?;
    // The semantic post-selection convergence supplies the version-independent
    // gpMarDirector SDA reference used to guard the current area/scenario. A
    // balloon-only patch does not need (and must not depend on) the separate
    // setTalkMsgID binding and its talk-callsite CFG validation.
    let convergence = find_dialogue_convergence_hook(source, &image)?;
    let hook = find_balloon_dialogue_hook(source, &image, convergence.director_sda_offset)?;
    let data_slot = (0..DOL_DATA_SECTION_COUNT)
        .find(|slot| {
            !image
                .sections
                .iter()
                .any(|section| !section.text && section.slot == *slot)
        })
        .ok_or_else(|| "The DOL has no unused data section for balloon overrides".to_string())?;
    let file_offset = align_up_usize(source.len(), FILE_ALIGNMENT as usize)?;
    let loaded_end = image
        .sections
        .iter()
        .map(|section| section.address_end())
        .chain(image.bss.map(|(_, end)| Ok(end)))
        .collect::<Result<Vec<_>, _>>()?
        .into_iter()
        .max()
        .ok_or_else(|| "The DOL has no loaded sections".to_string())?;
    let stub_address = align_up_u32(loaded_end, FILE_ALIGNMENT)?;
    let provisional_words = build_balloon_dialogue_stub(stub_address, 0, hook, effective.len())?;
    let table_offset = provisional_words
        .len()
        .checked_mul(4)
        .ok_or_else(|| "Balloon dialogue stub size overflows usize".to_string())?;
    let table_address = stub_address
        .checked_add(
            u32::try_from(table_offset)
                .map_err(|_| "Balloon dialogue table offset does not fit u32".to_string())?,
        )
        .ok_or_else(|| "Balloon dialogue table address overflows u32".to_string())?;
    let words = build_balloon_dialogue_stub(stub_address, table_address, hook, effective.len())?;
    let table_size = effective
        .len()
        .checked_mul(DIALOGUE_TABLE_ENTRY_SIZE)
        .ok_or_else(|| "Balloon dialogue table size overflows usize".to_string())?;
    let strings_offset = table_offset
        .checked_add(table_size)
        .ok_or_else(|| "Balloon dialogue string-table offset overflows usize".to_string())?;
    let names_size = effective.iter().try_fold(0_usize, |size, override_| {
        size.checked_add(override_.runtime_name_shift_jis.len() + 1)
            .ok_or_else(|| "Balloon dialogue name table size overflows usize".to_string())
    })?;
    let marker_offset = strings_offset
        .checked_add(names_size)
        .ok_or_else(|| "Balloon dialogue marker offset overflows usize".to_string())?;
    let payload_size = align_up_usize(
        marker_offset
            .checked_add(BALLOON_DIALOGUE_MARKER.len())
            .ok_or_else(|| "Balloon dialogue payload size overflows usize".to_string())?,
        4,
    )?;
    let stub_end = stub_address
        .checked_add(
            u32::try_from(payload_size)
                .map_err(|_| "Balloon dialogue payload size does not fit u32".to_string())?,
        )
        .ok_or_else(|| "Balloon dialogue payload address range overflows u32".to_string())?;
    let stack_top = find_stack_top(source, &image)?;
    if stub_end
        .checked_add(MIN_STAGE_MUSIC_STACK_GAP)
        .ok_or_else(|| "Balloon dialogue stack guard overflows u32".to_string())?
        > stack_top
    {
        return Err(format!(
            "Balloon dialogue payload 0x{stub_address:08X}..0x{stub_end:08X} is too close to the original stack top 0x{stack_top:08X}"
        ));
    }
    reject_injected_range_overlap(&image, stub_address, stub_end)?;

    let mut payload = vec![0_u8; payload_size];
    for (index, word) in words.iter().copied().enumerate() {
        write_be_u32(&mut payload, index * 4, word)?;
    }
    let mut name_cursor = strings_offset;
    for (index, override_) in effective.iter().enumerate() {
        let entry_offset = table_offset + index * DIALOGUE_TABLE_ENTRY_SIZE;
        payload[entry_offset] = override_.area_index;
        payload[entry_offset + 1] = override_.scenario_index;
        payload[entry_offset + 2..entry_offset + 4]
            .copy_from_slice(&override_.manager_instance_index.to_be_bytes());
        write_be_u32(
            &mut payload,
            entry_offset + 4,
            override_.original_message_id,
        )?;
        write_be_u32(
            &mut payload,
            entry_offset + 8,
            override_.replacement_message_id,
        )?;
        write_be_u32(&mut payload, entry_offset + 12, override_.actor_type)?;
        let name_key = override_
            .runtime_name_shift_jis
            .iter()
            .fold(0_u32, |key, byte| {
                key.wrapping_mul(3).wrapping_add(u32::from(*byte))
            }) as u16;
        payload[entry_offset + 16..entry_offset + 18].copy_from_slice(&name_key.to_be_bytes());
        let name_address = stub_address
            .checked_add(
                u32::try_from(name_cursor)
                    .map_err(|_| "Balloon dialogue name offset does not fit u32".to_string())?,
            )
            .ok_or_else(|| "Balloon dialogue name address overflows u32".to_string())?;
        write_be_u32(&mut payload, entry_offset + 20, name_address)?;
        for (axis, bits) in override_.reset_position_bits.iter().copied().enumerate() {
            write_be_u32(&mut payload, entry_offset + 24 + axis * 4, bits)?;
        }
        let name_end = name_cursor
            .checked_add(override_.runtime_name_shift_jis.len())
            .ok_or_else(|| "Balloon dialogue name range overflows usize".to_string())?;
        payload[name_cursor..name_end].copy_from_slice(&override_.runtime_name_shift_jis);
        payload[name_end] = 0;
        name_cursor = name_end + 1;
    }
    payload[marker_offset..marker_offset + BALLOON_DIALOGUE_MARKER.len()]
        .copy_from_slice(BALLOON_DIALOGUE_MARKER);

    let hook_address = hook.entry_anchor.address()?;
    let mut bytes = source.to_vec();
    bytes.resize(file_offset, 0);
    bytes.extend_from_slice(&payload);
    write_be_u32(
        &mut bytes,
        DOL_DATA_FILE_OFFSETS + data_slot * 4,
        u32::try_from(file_offset)
            .map_err(|_| "Balloon dialogue file offset does not fit u32".to_string())?,
    )?;
    write_be_u32(&mut bytes, DOL_DATA_ADDRESSES + data_slot * 4, stub_address)?;
    write_be_u32(
        &mut bytes,
        DOL_DATA_SIZES + data_slot * 4,
        u32::try_from(payload_size)
            .map_err(|_| "Balloon dialogue payload size does not fit u32".to_string())?,
    )?;
    write_be_u32(
        &mut bytes,
        hook.entry_anchor.file_offset()?,
        encode_branch(hook_address, stub_address, false)?,
    )?;
    parse_dol(&bytes)?;
    Ok(bytes)
}

fn find_dialogue_hook(source: &[u8], image: &DolImage) -> Result<DialogueHook, String> {
    let hook = find_dialogue_convergence_hook(source, image)?;
    let binding_string_addresses = find_c_string_addresses(source, image, "setTalkMsgID")?;
    if binding_string_addresses.len() != 1 {
        return Err(format!(
            "Could not uniquely locate the setTalkMsgID binding string; found {} candidate(s)",
            binding_string_addresses.len()
        ));
    }
    let binding_wrapper =
        find_bound_function_address(source, image, binding_string_addresses[0], "setTalkMsgID")?;

    let call_targets = direct_call_targets_from_function(source, image, binding_wrapper, 0x300)?;
    let hook_address = hook.replay_anchor.address()?;
    let mut matching_entries = Vec::new();
    for entry in call_targets {
        if function_cfg_converges_at(source, image, entry, hook_address, 0x800)? {
            matching_entries.push(entry);
        }
    }
    let _set_message_entry = require_unique_value(
        matching_entries,
        "setTalkMsgID wrapper call to TTalk2D2::setMessageID",
    )?;
    Ok(hook)
}

fn find_dialogue_convergence_hook(source: &[u8], image: &DolImage) -> Result<DialogueHook, String> {
    let mut candidates = Vec::new();
    for section in image
        .sections
        .iter()
        .copied()
        .filter(|section| section.text)
    {
        let words = section_words(source, section)?;
        for word_index in 0..words.len().saturating_sub(3) {
            let sequence = &words[word_index..word_index + 4];
            let Some((message_register, this_register, message_offset)) =
                decode_d_form(sequence[0], 36)
            else {
                continue;
            };
            let Some((_actor_type_register, npc_register, actor_type_offset)) =
                decode_d_form(sequence[1], 32)
            else {
                continue;
            };
            if message_register != 4
                || message_offset != 0x264
                || actor_type_offset != 0x4c
                || opcode(sequence[2]) != 15
                || register_a(sequence[2]) != register_t(sequence[1])
                || immediate_u16(sequence[2]) != 0xfc00
                || !is_cmplwi(sequence[3], register_t(sequence[2]), 0x1d)
            {
                continue;
            }
            let replay_anchor = WordAnchor {
                section,
                word_index: word_index + 1,
            };
            let search_start = word_index.saturating_sub(0x200);
            let mut director_offsets = Vec::new();
            for index in search_start..word_index {
                let Some(director_register) = decode_lwz_from_r13(words[index]) else {
                    continue;
                };
                let Some((loaded_npc_register, loaded_base, loaded_offset)) = words
                    .get(index + 1)
                    .and_then(|word| decode_d_form(*word, 32))
                else {
                    continue;
                };
                if loaded_base != director_register || loaded_offset != 0xa0 {
                    continue;
                }
                let reaches_npc_register = loaded_npc_register == npc_register
                    || words[index + 2..(index + 8).min(words.len())]
                        .iter()
                        .any(|word| {
                            is_mr(*word, npc_register, loaded_npc_register)
                                || decode_d_form(*word, 14)
                                    == Some((npc_register, loaded_npc_register, 0))
                        });
                if reaches_npc_register {
                    director_offsets.push(immediate_i16(words[index]));
                }
            }
            director_offsets.sort_unstable();
            director_offsets.dedup();
            if director_offsets.len() != 1 {
                continue;
            }
            candidates.push(DialogueHook {
                replay_anchor,
                replay_instruction: sequence[1],
                this_register,
                director_sda_offset: director_offsets[0],
            });
        }
    }
    require_unique_value(candidates, "post-selection TTalk2D2 message convergence")
}

fn find_bound_function_address(
    source: &[u8],
    image: &DolImage,
    string_address: u32,
    description: &str,
) -> Result<u32, String> {
    let mut candidates = Vec::new();
    for section in image
        .sections
        .iter()
        .copied()
        .filter(|section| section.text)
    {
        let words = section_words(source, section)?;
        for index in 3..words.len().saturating_sub(1) {
            let Some((argument_register, base_register, low)) = decode_d_form(words[index], 14)
            else {
                continue;
            };
            if argument_register != 4 || base_register == 0 || !is_relative_bl(words[index + 1]) {
                continue;
            }
            let mut base_values = Vec::new();
            let base_search_start = index.saturating_sub(0x400);
            for base_index in base_search_start..index {
                let Some((target, source_register, base_low)) =
                    decode_d_form(words[base_index], 14)
                else {
                    continue;
                };
                if target != base_register || source_register == 0 {
                    continue;
                }
                let lis_start = base_index.saturating_sub(8);
                for &lis in &words[lis_start..base_index] {
                    if opcode(lis) == 15
                        && register_a(lis) == 0
                        && register_t(lis) == source_register
                    {
                        let upper = u32::from(immediate_u16(lis)) << 16;
                        base_values.push(upper.wrapping_add_signed(i32::from(base_low)));
                    }
                }
            }
            if !base_values
                .into_iter()
                .any(|base| base.wrapping_add_signed(i32::from(low)) == string_address)
            {
                continue;
            }
            let lis = words[index - 3];
            let low_word = words[index - 2];
            if opcode(lis) != 15
                || register_a(lis) != 0
                || opcode(low_word) != 14
                || register_a(low_word) != register_t(lis)
                || register_t(low_word) != 5
            {
                continue;
            }
            let wrapper = (u32::from(immediate_u16(lis)) << 16)
                .wrapping_add_signed(i32::from(immediate_i16(low_word)));
            if address_is_in_text(&image.sections, wrapper, 4)? {
                candidates.push(wrapper);
            }
        }
    }
    candidates.sort_unstable();
    candidates.dedup();
    require_unique_value(candidates, &format!("{description} bound-function pointer"))
}

fn direct_call_targets_from_function(
    source: &[u8],
    image: &DolImage,
    function_address: u32,
    max_size: usize,
) -> Result<Vec<u32>, String> {
    let section = image
        .sections
        .iter()
        .copied()
        .find(|section| {
            section.text
                && function_address >= section.address
                && function_address < section.address.saturating_add(section.size)
        })
        .ok_or_else(|| format!("Function 0x{function_address:08X} is outside DOL text"))?;
    let start_word = usize::try_from((function_address - section.address) / 4)
        .map_err(|_| "Function word offset does not fit usize".to_string())?;
    let words = section_words(source, section)?;
    let end_word = start_word
        .checked_add(max_size / 4)
        .map(|end| end.min(words.len()))
        .ok_or_else(|| "Function search range overflows usize".to_string())?;
    let mut targets = Vec::new();
    for (relative, word) in words[start_word..end_word].iter().copied().enumerate() {
        let word_index = start_word + relative;
        if is_relative_bl(word) {
            let address = section
                .address
                .checked_add(
                    u32::try_from(word_index * 4)
                        .map_err(|_| "Callsite word offset does not fit u32".to_string())?,
                )
                .ok_or_else(|| "Callsite address overflows u32".to_string())?;
            let target = decode_branch_target(word, address)?;
            if address_is_in_text(&image.sections, target, 4)? {
                targets.push(target);
            }
        }
        if word == PPC_BLR && relative > 4 {
            break;
        }
    }
    targets.sort_unstable();
    targets.dedup();
    Ok(targets)
}

/// Proves that a bound call target reaches the semantic hook through its own
/// direct control-flow graph. This rejects a coincidentally nearby function:
/// every reachable return/indirect escape before the hook is disqualifying,
/// and every reachable basic block must retain a path to the convergence.
fn function_cfg_converges_at(
    source: &[u8],
    image: &DolImage,
    function_address: u32,
    hook_address: u32,
    max_size: usize,
) -> Result<bool, String> {
    let Some(entry) = address_to_word_anchor(image, function_address)? else {
        return Ok(false);
    };
    let Some(hook) = address_to_word_anchor(image, hook_address)? else {
        return Ok(false);
    };
    if entry.section != hook.section || hook.word_index < entry.word_index {
        return Ok(false);
    }
    let words = section_words(source, entry.section)?;
    let end_word = entry
        .word_index
        .checked_add(max_size / 4)
        .map(|end| end.min(words.len()))
        .ok_or_else(|| "Dialogue CFG range overflows usize".to_string())?;
    if hook.word_index >= end_word {
        return Ok(false);
    }

    let mut pending = VecDeque::from([entry.word_index]);
    let mut reachable = BTreeSet::new();
    let mut edges = BTreeMap::<usize, Vec<usize>>::new();
    let mut escaped_before_hook = false;
    while let Some(index) = pending.pop_front() {
        if !reachable.insert(index) || index == hook.word_index {
            continue;
        }
        let Some(word) = words.get(index).copied() else {
            escaped_before_hook = true;
            continue;
        };
        let address = WordAnchor {
            section: entry.section,
            word_index: index,
        }
        .address()?;
        let next = index.saturating_add(1);
        let mut successors = Vec::new();
        match opcode(word) {
            18 if word & 1 != 0 => successors.push(next), // direct call returns
            18 if word & 2 == 0 => {
                let target = decode_branch_target(word, address)?;
                let Some(target) = address_to_word_anchor(image, target)? else {
                    escaped_before_hook = true;
                    continue;
                };
                if target.section != entry.section {
                    escaped_before_hook = true;
                    continue;
                }
                successors.push(target.word_index);
            }
            18 => {
                escaped_before_hook = true;
                continue;
            }
            16 if word & 1 != 0 => successors.push(next), // conditional call returns
            16 if word & 2 == 0 => {
                let target = decode_conditional_branch_target(word, address)?;
                let Some(target) = address_to_word_anchor(image, target)? else {
                    escaped_before_hook = true;
                    continue;
                };
                if target.section != entry.section {
                    escaped_before_hook = true;
                    continue;
                }
                successors.extend([next, target.word_index]);
            }
            16 => {
                escaped_before_hook = true;
                continue;
            }
            19 if matches!((word >> 1) & 0x3ff, 16 | 528) && word & 1 != 0 => {
                successors.push(next); // indirect call returns
            }
            19 if matches!((word >> 1) & 0x3ff, 16 | 528) => {
                // bclr/blr/bcctr/bctr leave direct CFG analysis.
                escaped_before_hook = true;
                continue;
            }
            _ => successors.push(next),
        }
        successors.sort_unstable();
        successors.dedup();
        if successors
            .iter()
            .any(|successor| *successor < entry.word_index || *successor >= end_word)
        {
            escaped_before_hook = true;
            continue;
        }
        for successor in &successors {
            pending.push_back(*successor);
        }
        edges.insert(index, successors);
    }
    if escaped_before_hook || !reachable.contains(&hook.word_index) {
        return Ok(false);
    }

    let mut can_reach_hook = BTreeSet::from([hook.word_index]);
    loop {
        let before = can_reach_hook.len();
        for (from, successors) in &edges {
            if successors
                .iter()
                .any(|successor| can_reach_hook.contains(successor))
            {
                can_reach_hook.insert(*from);
            }
        }
        if can_reach_hook.len() == before {
            break;
        }
    }
    Ok(reachable.is_subset(&can_reach_hook))
}

fn find_balloon_dialogue_hook(
    source: &[u8],
    image: &DolImage,
    director_sda_offset: i16,
) -> Result<BalloonDialogueHook, String> {
    let binding_string_addresses = find_c_string_addresses(source, image, "setNpcBalloonMessage")?;
    if binding_string_addresses.len() != 1 {
        return Err(format!(
            "Could not uniquely locate the setNpcBalloonMessage binding string; found {} candidate(s)",
            binding_string_addresses.len()
        ));
    }
    let binding_wrapper = find_bound_function_address(
        source,
        image,
        binding_string_addresses[0],
        "setNpcBalloonMessage",
    )?;
    let call_targets = direct_call_targets_from_function(source, image, binding_wrapper, 0x400)?;
    let mut candidates = Vec::new();
    for target in call_targets {
        let Some(anchor) = address_to_word_anchor(image, target)? else {
            continue;
        };
        let words = section_words(source, anchor.section)?;
        let Some(sequence) = words.get(anchor.word_index..anchor.word_index.saturating_add(9))
        else {
            continue;
        };
        if sequence.len() != 9
            || sequence[0] != 0x7c08_02a6
            || decode_d_form(sequence[1], 36) != Some((0, 1, 4))
            || decode_d_form(sequence[2], 37) != Some((1, 1, -8))
            || decode_d_form(sequence[3], 32) != Some((3, 3, 0x188))
            || !is_relative_bl(sequence[4])
            || decode_d_form(sequence[5], 32) != Some((0, 1, 12))
            || decode_d_form(sequence[6], 14) != Some((1, 1, 8))
            || sequence[7] != 0x7c08_03a6
            || sequence[8] != PPC_BLR
        {
            continue;
        }
        candidates.push(BalloonDialogueHook {
            entry_anchor: anchor,
            replay_instruction: sequence[0],
            director_sda_offset,
        });
    }
    require_unique_value(candidates, "TBaseNPC::setBalloonMessage entry")
}

fn address_to_word_anchor(image: &DolImage, address: u32) -> Result<Option<WordAnchor>, String> {
    for section in image
        .sections
        .iter()
        .copied()
        .filter(|section| section.text)
    {
        if address >= section.address && address < section.address_end()? && address & 3 == 0 {
            return Ok(Some(WordAnchor {
                section,
                word_index: usize::try_from((address - section.address) / 4)
                    .map_err(|_| "DOL word index does not fit usize".to_string())?,
            }));
        }
    }
    Ok(None)
}

fn build_balloon_dialogue_stub(
    stub_address: u32,
    table_address: u32,
    hook: BalloonDialogueHook,
    entry_count: usize,
) -> Result<Vec<u32>, String> {
    let mut words = vec![
        encode_d_form(37, 1, 1, -0x40),
        encode_d_form(36, 0, 1, 4),
        0x7c00_0026, // mfcr r0
        encode_d_form(36, 0, 1, 8),
        0x7c09_02a6, // mfctr r0
        encode_d_form(36, 0, 1, 12),
    ];
    for register in 3..=12 {
        words.push(encode_d_form(
            36,
            register,
            1,
            16 + i16::from(register - 3) * 4,
        ));
    }
    words.extend([
        encode_mr_word(10, 3),
        encode_mr_word(7, 4),
        encode_d_form(32, 11, 13, hook.director_sda_offset),
        encode_cmpwi(11, 0),
        0, // beq epilogue
        encode_d_form(34, 8, 11, 0x7c),
        encode_d_form(34, 9, 11, 0x7d),
        encode_d_form(40, 6, 10, 0x7c),
        encode_d_form(32, 5, 10, 0x4c),
        encode_d_form(40, 4, 10, 8),
    ]);
    words.extend(encode_u32(3, table_address));
    words.extend(encode_u32(
        0,
        u32::try_from(entry_count)
            .map_err(|_| "Balloon override count does not fit u32".to_string())?,
    ));
    words.push(0x7c09_03a6); // mtctr r0
    let loop_index = words.len();
    words.extend([
        encode_d_form(34, 0, 3, 0),
        encode_cmplw(0, 8),
        0,
        encode_d_form(34, 0, 3, 1),
        encode_cmplw(0, 9),
        0,
        encode_d_form(40, 0, 3, 2),
        encode_cmplw(0, 6),
        0,
        encode_d_form(32, 0, 3, 4),
        encode_cmplw(0, 7),
        0,
        encode_d_form(32, 0, 3, 12),
        encode_cmplw(0, 5),
        0,
        encode_d_form(40, 0, 3, 16),
        encode_cmplw(0, 4),
        0,
        encode_d_form(32, 0, 3, 24),
        encode_d_form(32, 11, 10, 0x194),
        encode_cmplw(0, 11),
        0,
        encode_d_form(32, 0, 3, 28),
        encode_d_form(32, 11, 10, 0x198),
        encode_cmplw(0, 11),
        0,
        encode_d_form(32, 0, 3, 32),
        encode_d_form(32, 11, 10, 0x19c),
        encode_cmplw(0, 11),
        0,
        encode_d_form(32, 11, 3, 20),
        encode_d_form(32, 9, 10, 4),
    ]);
    let name_loop_index = words.len();
    words.extend([
        encode_d_form(34, 8, 9, 0),
        encode_d_form(34, 6, 11, 0),
        encode_cmplw(8, 6),
        0,
        encode_cmpwi(8, 0),
        0,
        encode_d_form(14, 9, 9, 1),
        encode_d_form(14, 11, 11, 1),
        0,
    ]);
    let matched_index = words.len();
    words.extend([
        encode_d_form(32, 0, 3, 8),
        encode_d_form(36, 0, 1, 20), // replace saved incoming r4
        0,
    ]);
    let next_index = words.len();
    words.extend([
        encode_d_form(
            14,
            3,
            3,
            i16::try_from(DIALOGUE_TABLE_ENTRY_SIZE)
                .map_err(|_| "Dialogue table entry size does not fit i16".to_string())?,
        ),
        0,
    ]);
    let epilogue_index = words.len();
    words.extend([
        encode_d_form(32, 0, 1, 8),
        0x7c0f_f120,
        encode_d_form(32, 0, 1, 12),
        0x7c09_03a6,
    ]);
    for register in 3..=12 {
        words.push(encode_d_form(
            32,
            register,
            1,
            16 + i16::from(register - 3) * 4,
        ));
    }
    words.extend([
        encode_d_form(32, 0, 1, 4),
        encode_d_form(14, 1, 1, 0x40),
        hook.replay_instruction,
        0,
    ]);

    for branch_index in [
        loop_index + 2,
        loop_index + 5,
        loop_index + 8,
        loop_index + 11,
        loop_index + 14,
        loop_index + 17,
        loop_index + 21,
        loop_index + 25,
        loop_index + 29,
        name_loop_index + 3,
    ] {
        words[branch_index] = encode_bne(
            dialogue_word_address(stub_address, branch_index)?,
            dialogue_word_address(stub_address, next_index)?,
        )?;
    }
    words[20] = encode_beq(
        dialogue_word_address(stub_address, 20)?,
        dialogue_word_address(stub_address, epilogue_index)?,
    )?;
    words[name_loop_index + 5] = encode_beq(
        dialogue_word_address(stub_address, name_loop_index + 5)?,
        dialogue_word_address(stub_address, matched_index)?,
    )?;
    words[name_loop_index + 8] = encode_branch(
        dialogue_word_address(stub_address, name_loop_index + 8)?,
        dialogue_word_address(stub_address, name_loop_index)?,
        false,
    )?;
    words[matched_index + 2] = encode_branch(
        dialogue_word_address(stub_address, matched_index + 2)?,
        dialogue_word_address(stub_address, epilogue_index)?,
        false,
    )?;
    words[next_index + 1] = encode_bdnz(
        dialogue_word_address(stub_address, next_index + 1)?,
        dialogue_word_address(stub_address, loop_index)?,
    )?;
    let final_branch_index = words.len() - 1;
    words[final_branch_index] = encode_branch(
        dialogue_word_address(stub_address, final_branch_index)?,
        hook.entry_anchor
            .address()?
            .checked_add(4)
            .ok_or_else(|| "Balloon hook resume address overflows u32".to_string())?,
        false,
    )?;
    Ok(words)
}

fn build_dialogue_stub(
    stub_address: u32,
    table_address: u32,
    hook: DialogueHook,
    entry_count: usize,
) -> Result<Vec<u32>, String> {
    let mut words = vec![
        encode_d_form(37, 1, 1, -0x40), // stwu r1, -0x40(r1)
        encode_d_form(36, 0, 1, 4),
        0x7c00_0026, // mfcr r0
        encode_d_form(36, 0, 1, 8),
        0x7c09_02a6, // mfctr r0
        encode_d_form(36, 0, 1, 12),
    ];
    for register in 3..=12 {
        words.push(encode_d_form(
            36,
            register,
            1,
            16 + i16::from(register - 3) * 4,
        ));
    }
    words.extend([
        encode_mr_word(12, hook.this_register),
        encode_d_form(32, 11, 13, hook.director_sda_offset),
        encode_cmpwi(11, 0),
        0, // beq epilogue
        encode_d_form(32, 10, 11, 0xa0),
        encode_cmpwi(10, 0),
        0, // beq epilogue
        encode_d_form(34, 8, 11, 0x7c),
        encode_d_form(34, 9, 11, 0x7d),
        encode_d_form(32, 7, 12, 0x264),
        encode_d_form(40, 6, 10, 0x7c),
        encode_d_form(32, 5, 10, 0x4c),
        encode_d_form(40, 4, 10, 8),
    ]);
    words.extend(encode_u32(3, table_address));
    words.extend(encode_u32(
        0,
        u32::try_from(entry_count)
            .map_err(|_| "Dialogue override count does not fit u32".to_string())?,
    ));
    words.push(0x7c09_03a6); // mtctr r0
    let loop_index = words.len();
    words.extend([
        encode_d_form(34, 0, 3, 0),
        encode_cmplw(0, 8),
        0, // bne next
        encode_d_form(34, 0, 3, 1),
        encode_cmplw(0, 9),
        0,
        encode_d_form(40, 0, 3, 2),
        encode_cmplw(0, 6),
        0,
        encode_d_form(32, 0, 3, 4),
        encode_cmplw(0, 7),
        0,
        encode_d_form(32, 0, 3, 12),
        encode_cmplw(0, 5),
        0,
        encode_d_form(40, 0, 3, 16),
        encode_cmplw(0, 4),
        0,
        encode_d_form(32, 0, 3, 24),
        encode_d_form(32, 11, 10, 0x194),
        encode_cmplw(0, 11),
        0,
        encode_d_form(32, 0, 3, 28),
        encode_d_form(32, 11, 10, 0x198),
        encode_cmplw(0, 11),
        0,
        encode_d_form(32, 0, 3, 32),
        encode_d_form(32, 11, 10, 0x19c),
        encode_cmplw(0, 11),
        0,
        encode_d_form(32, 11, 3, 20),
        encode_d_form(32, 9, 10, 4),
    ]);
    let name_loop_index = words.len();
    words.extend([
        encode_d_form(34, 8, 9, 0),
        encode_d_form(34, 6, 11, 0),
        encode_cmplw(8, 6),
        0, // bne next
        encode_cmpwi(8, 0),
        0, // beq matched
        encode_d_form(14, 9, 9, 1),
        encode_d_form(14, 11, 11, 1),
        0, // b name loop
    ]);
    let matched_index = words.len();
    words.extend([
        encode_d_form(32, 0, 3, 8),
        encode_d_form(36, 0, 12, 0x264),
        0, // b epilogue
    ]);
    let next_index = words.len();
    words.extend([
        encode_d_form(
            14,
            3,
            3,
            i16::try_from(DIALOGUE_TABLE_ENTRY_SIZE)
                .map_err(|_| "Dialogue table entry size does not fit i16".to_string())?,
        ),
        0, // bdnz loop
    ]);
    let epilogue_index = words.len();
    words.extend([
        encode_d_form(32, 0, 1, 8),
        0x7c0f_f120, // mtcrf 0xff, r0
        encode_d_form(32, 0, 1, 12),
        0x7c09_03a6, // mtctr r0
    ]);
    for register in 3..=12 {
        words.push(encode_d_form(
            32,
            register,
            1,
            16 + i16::from(register - 3) * 4,
        ));
    }
    words.extend([
        encode_d_form(32, 0, 1, 4),
        encode_d_form(14, 1, 1, 0x40),
        hook.replay_instruction,
        0, // b resume
    ]);

    let next_branches = [
        loop_index + 2,
        loop_index + 5,
        loop_index + 8,
        loop_index + 11,
        loop_index + 14,
        loop_index + 17,
        loop_index + 21,
        loop_index + 25,
        loop_index + 29,
        name_loop_index + 3,
    ];
    for branch_index in next_branches {
        words[branch_index] = encode_bne(
            dialogue_word_address(stub_address, branch_index)?,
            dialogue_word_address(stub_address, next_index)?,
        )?;
    }
    words[19] = encode_beq(
        dialogue_word_address(stub_address, 19)?,
        dialogue_word_address(stub_address, epilogue_index)?,
    )?;
    words[22] = encode_beq(
        dialogue_word_address(stub_address, 22)?,
        dialogue_word_address(stub_address, epilogue_index)?,
    )?;
    words[name_loop_index + 5] = encode_beq(
        dialogue_word_address(stub_address, name_loop_index + 5)?,
        dialogue_word_address(stub_address, matched_index)?,
    )?;
    words[name_loop_index + 8] = encode_branch(
        dialogue_word_address(stub_address, name_loop_index + 8)?,
        dialogue_word_address(stub_address, name_loop_index)?,
        false,
    )?;
    words[matched_index + 2] = encode_branch(
        dialogue_word_address(stub_address, matched_index + 2)?,
        dialogue_word_address(stub_address, epilogue_index)?,
        false,
    )?;
    words[next_index + 1] = encode_bdnz(
        dialogue_word_address(stub_address, next_index + 1)?,
        dialogue_word_address(stub_address, loop_index)?,
    )?;
    let final_branch_index = words.len() - 1;
    let resume_address = hook
        .replay_anchor
        .address()?
        .checked_add(4)
        .ok_or_else(|| "Dialogue hook resume address overflows u32".to_string())?;
    words[final_branch_index] = encode_branch(
        dialogue_word_address(stub_address, final_branch_index)?,
        resume_address,
        false,
    )?;
    Ok(words)
}

fn dialogue_word_address(base: u32, index: usize) -> Result<u32, String> {
    base.checked_add(
        u32::try_from(index)
            .map_err(|_| "Dialogue instruction index does not fit u32".to_string())?
            .checked_mul(4)
            .ok_or_else(|| "Dialogue instruction offset overflows u32".to_string())?,
    )
    .ok_or_else(|| "Dialogue instruction address overflows u32".to_string())
}

pub(super) fn patch_sms_stage_music_dol(
    source: &[u8],
    overrides: &[RuntimeStageMusicOverride],
) -> Result<StageMusicDol, String> {
    if overrides.is_empty() {
        return Err("Stage music patch requires at least one override".to_string());
    }
    if overrides.len() > MAX_STAGE_MUSIC_OVERRIDES {
        return Err(format!(
            "Stage music patch has {} overrides; the safe limit is {MAX_STAGE_MUSIC_OVERRIDES}",
            overrides.len()
        ));
    }
    if source
        .windows(STAGE_MUSIC_MARKER.len())
        .any(|window| window == STAGE_MUSIC_MARKER)
    {
        return Err("The executable already contains a stage music patch".to_string());
    }
    let mut unique = std::collections::BTreeSet::new();
    for override_ in overrides {
        if !unique.insert((override_.area_index, override_.scenario_index)) {
            return Err(format!(
                "Stage music has duplicate runtime area {}, scenario {} overrides",
                override_.area_index, override_.scenario_index
            ));
        }
        for (name, role) in [
            ("main", override_.main),
            ("entrance", override_.entrance),
            ("inside", override_.inside),
            ("switch A", override_.switch_a),
            ("switch B", override_.switch_b),
        ] {
            if let Some((bgm_id, wave_scene_id)) = role.track() {
                if bgm_id & 0xffff_0000 != 0x8001_0000
                    || wave_scene_id.is_some_and(|wave| wave > u16::MAX.into())
                {
                    return Err(format!(
                        "Stage music area {}, scenario {} has an invalid {name} BGM/wave identifier",
                        override_.area_index, override_.scenario_index
                    ));
                }
            }
        }
    }

    let image = parse_dol(source)?;
    let hook = find_sound_stage_hook(source, &image)?;
    let wave_loader = find_wave_bank_load_wave(source, &image)?;
    let text_slot = (0..DOL_TEXT_SECTION_COUNT)
        .find(|slot| {
            !image
                .sections
                .iter()
                .any(|section| section.text && section.slot == *slot)
        })
        .ok_or_else(|| "The DOL has no unused text section for stage music".to_string())?;
    let file_offset = align_up_usize(source.len(), FILE_ALIGNMENT as usize)?;
    let stack_top = find_stack_top(source, &image)?;
    let loaded_end = image
        .sections
        .iter()
        .map(|section| section.address_end())
        .chain(image.bss.map(|(_, end)| Ok(end)))
        .collect::<Result<Vec<_>, _>>()?
        .into_iter()
        .max()
        .ok_or_else(|| "The DOL has no loaded sections".to_string())?;
    // Extending the loaded image keeps the dispatcher outside the arena without changing
    // Sunshine's linker-defined startup stack or any runtime stack metadata derived from it.
    let stub_address = align_up_u32(loaded_end, FILE_ALIGNMENT)?;
    let stub = build_stage_music_stub(stub_address, hook, wave_loader, overrides)?;
    let unaligned_payload_size = stub
        .words
        .len()
        .checked_mul(4)
        .and_then(|size| size.checked_add(STAGE_MUSIC_MARKER.len()))
        .ok_or_else(|| "Stage music payload size overflows usize".to_string())?;
    let payload_size = align_up_usize(unaligned_payload_size, 4)?;
    let stub_end = stub_address
        .checked_add(u32::try_from(payload_size).map_err(|_| {
            "Stage music payload size does not fit the DOL address space".to_string()
        })?)
        .ok_or_else(|| "Stage music stub range overflows u32".to_string())?;
    let safe_stub_end = stub_end
        .checked_add(MIN_STAGE_MUSIC_STACK_GAP)
        .ok_or_else(|| "Stage music stack guard overflows u32".to_string())?;
    if safe_stub_end > stack_top {
        return Err(format!(
            "Stage music payload 0x{stub_address:08X}..0x{stub_end:08X} leaves less than 0x{MIN_STAGE_MUSIC_STACK_GAP:X} bytes below the original stack top 0x{stack_top:08X}"
        ));
    }
    reject_injected_range_overlap(&image, stub_address, stub_end)?;

    let hook_address = hook.dispatch_anchor.address()?;
    let mut bytes = source.to_vec();
    bytes.resize(file_offset, 0);
    for word in stub.words {
        bytes.extend_from_slice(&word.to_be_bytes());
    }
    bytes.extend_from_slice(STAGE_MUSIC_MARKER);
    bytes.resize(
        file_offset
            .checked_add(payload_size)
            .ok_or_else(|| "Stage music output size overflows usize".to_string())?,
        0,
    );
    write_be_u32(
        &mut bytes,
        DOL_TEXT_FILE_OFFSETS + text_slot * 4,
        u32::try_from(file_offset)
            .map_err(|_| "Stage music file offset does not fit u32".to_string())?,
    )?;
    write_be_u32(&mut bytes, DOL_TEXT_ADDRESSES + text_slot * 4, stub_address)?;
    write_be_u32(
        &mut bytes,
        DOL_TEXT_SIZES + text_slot * 4,
        u32::try_from(payload_size)
            .map_err(|_| "Stage music section size does not fit u32".to_string())?,
    )?;
    write_be_u32(
        &mut bytes,
        hook.dispatch_anchor.file_offset()?,
        encode_branch(hook_address, stub_address, false)?,
    )?;
    let wrapper_address = stub_address
        .checked_add(
            u32::try_from(stub.wrapper_word * 4)
                .map_err(|_| "Stage music wrapper address does not fit u32".to_string())?,
        )
        .ok_or_else(|| "Stage music wrapper address overflows u32".to_string())?;
    write_be_u32(
        &mut bytes,
        hook.enter_stage_anchor.file_offset()?,
        encode_branch(hook.enter_stage_anchor.address()?, wrapper_address, true)?,
    )?;
    parse_dol(&bytes)?;
    Ok(StageMusicDol {
        bytes,
        hook_address,
        stub_address,
    })
}

pub(super) fn patch_sms_direct_boot_dol(
    source: &[u8],
    target: &RuntimeStageTarget,
) -> Result<DirectBootDol, String> {
    if target.archive_name.as_bytes().contains(&0) {
        return Err("Runtime stage archive name contains a NUL byte".to_string());
    }

    let image = parse_dol(source)?;
    let hook = find_nlogo_hook(source, &image)?;
    let director_bypass = find_nlogo_director_bypass(source, hook)?;
    let setup_bypass = find_nlogo_setup_bypass(source, &image, hook.this_register)?;
    let setter = find_next_area_setter(source, &image)?;
    let movie = find_movie_hook(source, &image, setter)?;
    let death = find_death_restart_hook(source, &image, setter.next_area_offset)?;
    if hook.this_register == hook.next_state_register {
        return Err(
            "Post-NLogo state register aliases the TApplication register; refusing unsafe patch"
                .to_string(),
        );
    }

    let area_load_address = hook
        .anchor
        .address()?
        .checked_add(12)
        .ok_or_else(|| "Post-NLogo area-load address overflows u32".to_string())?;
    let hook_address = area_load_address
        .checked_add(4)
        .ok_or_else(|| "Post-NLogo transition hook address overflows u32".to_string())?;
    let movie_hook_address = movie.call_anchor.address()?;
    let death_hook_address = death.call_anchor.address()?;
    let original_transition_target =
        decode_branch_target(section_word(source, hook.anchor, 4)?, hook_address)?;
    let caves = choose_direct_boot_caves(
        find_zero_alignment_code_caves(source, &image)?,
        hook_address,
        movie_hook_address,
        death_hook_address,
        original_transition_target,
        movie.original_target,
    )?;
    let transition_address = caves.transition.anchor.address()?;
    let movie_primary_address = caves.movie_primary.anchor.address()?;
    let movie_secondary_address = caves.movie_secondary.anchor.address()?;
    let death_restart_address = caves.death_restart.anchor.address()?;

    let transition_words = build_transition_cave(
        transition_address,
        original_transition_target,
        hook.this_register,
        hook.next_state_register,
        setter.next_area_offset,
        target,
    )?;
    let (movie_primary_words, movie_secondary_words) = build_movie_caves(
        movie_primary_address,
        movie_secondary_address,
        movie.original_target,
        setter.next_area_offset,
    )?;
    let death_restart_words = build_death_restart_cave(death.current_area_register, target);

    let mut bytes = source.to_vec();
    let director_branch_address = director_bypass.branch_anchor.address()?;
    write_be_u32(
        &mut bytes,
        director_bypass.branch_anchor.file_offset()?,
        encode_branch(
            director_branch_address,
            director_bypass.completion_anchor.address()?,
            false,
        )?,
    )?;
    let setup_case_address = setup_bypass.case_anchor.address()?;
    write_be_u32(
        &mut bytes,
        setup_bypass.case_anchor.file_offset()?,
        encode_branch(setup_case_address, setup_bypass.resume_address, false)?,
    )?;
    write_be_u32(
        &mut bytes,
        hook.anchor
            .file_offset()?
            .checked_add(12)
            .ok_or_else(|| "Post-NLogo area-load file offset overflows usize".to_string())?,
        encode_li(hook.next_state_register, i16::from(target.area_index)),
    )?;
    write_be_u32(
        &mut bytes,
        hook.anchor
            .file_offset()?
            .checked_add(16)
            .ok_or_else(|| "Post-NLogo hook file offset overflows usize".to_string())?,
        encode_branch(hook_address, transition_address, false)?,
    )?;
    write_be_u32(
        &mut bytes,
        movie.call_anchor.file_offset()?,
        encode_branch(movie_hook_address, movie_primary_address, true)?,
    )?;
    write_be_u32(
        &mut bytes,
        death.call_anchor.file_offset()?,
        encode_branch(death_hook_address, death_restart_address, true)?,
    )?;
    write_words(&mut bytes, caves.transition.anchor, &transition_words)?;
    write_words(&mut bytes, caves.movie_primary.anchor, &movie_primary_words)?;
    write_words(
        &mut bytes,
        caves.movie_secondary.anchor,
        &movie_secondary_words,
    )?;
    write_words(&mut bytes, caves.death_restart.anchor, &death_restart_words)?;
    parse_dol(&bytes)?;

    Ok(DirectBootDol {
        bytes,
        logo_bypass_address: setup_case_address,
        hook_address,
        movie_hook_address,
        death_hook_address,
        death_stub_address: death_restart_address,
        stub_address: transition_address,
    })
}

fn find_sound_stage_hook(source: &[u8], image: &DolImage) -> Result<SoundStageHook, String> {
    let mut candidates = Vec::new();
    for section in image
        .sections
        .iter()
        .copied()
        .filter(|section| section.text)
    {
        let words = section_words(source, section)?;
        for word_index in 0..words.len().saturating_sub(9) {
            let sequence = &words[word_index..word_index + 10];
            let Some(sound_register) = decode_lwz_from_r13(sequence[0]) else {
                continue;
            };
            let dispatch_anchor = WordAnchor {
                section,
                word_index: word_index + 2,
            };
            let Some(ms_stg_offset) =
                decode_sound_stage_offset(source, image, dispatch_anchor, sequence[2])?
            else {
                continue;
            };
            let tail = if is_cmpwi(sequence[3], 4, -1) && is_beq(sequence[4]) {
                5
            } else if decode_d_form(sequence[3], 15) == Some((0, 4, 1))
                && is_cmplwi(sequence[4], 0, u16::MAX)
                && is_beq(sequence[5])
            {
                6
            } else {
                continue;
            };
            let Some((area_argument, area_register, area_immediate)) =
                decode_d_form(sequence[tail + 1], 14)
            else {
                continue;
            };
            let Some((scenario_argument, scenario_register, scenario_immediate)) =
                decode_d_form(sequence[tail + 2], 14)
            else {
                continue;
            };
            if sound_register != 3
                || !is_relative_bl(sequence[1])
                || decode_lwz_from_r13(sequence[tail]) != Some(3)
                || immediate_i16(sequence[tail]) != immediate_i16(sequence[0])
                || area_argument != 5
                || area_immediate != 0
                || scenario_argument != 6
                || scenario_immediate != 0
                || !is_relative_bl(sequence[tail + 3])
            {
                continue;
            }
            let call_anchor = WordAnchor {
                section,
                word_index: word_index + 1,
            };
            let original_target = decode_branch_target(sequence[1], call_anchor.address()?)?;
            if !address_is_in_text(&image.sections, original_target, 4)? {
                continue;
            }
            candidates.push(SoundStageHook {
                dispatch_anchor,
                enter_stage_anchor: WordAnchor {
                    section,
                    word_index: word_index + tail + 3,
                },
                enter_stage_target: decode_branch_target(
                    sequence[tail + 3],
                    section
                        .address
                        .checked_add(u32::try_from((word_index + tail + 3) * 4).map_err(|_| {
                            "MSound enterStage call address does not fit u32".to_string()
                        })?)
                        .ok_or_else(|| "MSound enterStage call address overflows".to_string())?,
                )?,
                area_register,
                scenario_register,
                ms_stg_offset,
            });
        }
    }
    require_unique_value(candidates, "MSound stage initialization call")
}

fn decode_sound_stage_offset(
    source: &[u8],
    image: &DolImage,
    dispatch_anchor: WordAnchor,
    instruction: u32,
) -> Result<Option<i16>, String> {
    if decode_lwz_from_r13(instruction) == Some(4) {
        return Ok(Some(immediate_i16(instruction)));
    }
    if !is_unconditional_branch(instruction) {
        return Ok(None);
    }
    let dispatch_address = dispatch_anchor.address()?;
    let dispatcher_target = decode_branch_target(instruction, dispatch_address)?;
    let Some(dispatcher_anchor) = address_to_word_anchor(image, dispatcher_target)? else {
        return Ok(None);
    };
    if !dispatcher_anchor.section.text {
        return Ok(None);
    }
    let resume_address = dispatch_address
        .checked_add(4)
        .ok_or_else(|| "MSound dispatcher resume address overflows u32".to_string())?;
    let words = section_words(source, dispatcher_anchor.section)?;
    let mut offsets = Vec::new();
    for word_index in dispatcher_anchor.word_index..words.len().saturating_sub(1) {
        if decode_lwz_from_r13(words[word_index]) != Some(4)
            || !is_unconditional_branch(words[word_index + 1])
        {
            continue;
        }
        let branch_anchor = WordAnchor {
            section: dispatcher_anchor.section,
            word_index: word_index + 1,
        };
        if decode_branch_target(words[word_index + 1], branch_anchor.address()?)? == resume_address
        {
            offsets.push(immediate_i16(words[word_index]));
        }
    }
    offsets.sort_unstable();
    offsets.dedup();
    Ok(match offsets.as_slice() {
        [offset] => Some(*offset),
        _ => None,
    })
}

fn find_wave_bank_load_wave(source: &[u8], image: &DolImage) -> Result<u32, String> {
    let mut candidates = Vec::new();
    for section in image
        .sections
        .iter()
        .copied()
        .filter(|section| section.text)
    {
        let words = section_words(source, section)?;
        for word_index in 0..words.len().saturating_sub(58) {
            let sequence = &words[word_index..word_index + 58];
            if sequence[0] != 0x7c08_02a6
                || sequence[1] != 0x9001_0004
                || sequence[2] != 0x5460_103a
                || sequence[3] != 0x9421_ffe8
                || sequence[4] != 0x93e1_0014
                || sequence[5] != 0x93c1_0010
                || (sequence[6] != 0x7c9e_2378
                    && decode_d_form(sequence[6], 14) != Some((30, 4, 0)))
                || decode_d_form(sequence[7], 32).map(|(rt, ra, _)| (rt, ra)) != Some((5, 13))
                || sequence[8] != 0x7fe5_002e
                || sequence[17] != 0x4e80_0021
                || sequence[18] != 0x3c03_bdad
                || sequence[19] != 0x2800_4943
                || sequence[43] != 0x4e80_0021
                || sequence[44] != 0x3c03_acb3
                || sequence[45] != 0x2800_504c
            {
                continue;
            }
            candidates.push(WordAnchor {
                section,
                word_index,
            });
        }
    }
    candidates.sort_by_key(|anchor| anchor.address().unwrap_or(u32::MAX));
    let pairs = candidates
        .windows(2)
        .filter(|pair| {
            pair[0].section == pair[1].section
                && pair[0]
                    .address()
                    .ok()
                    .zip(pair[1].address().ok())
                    .is_some_and(|(left, right)| right > left && right - left < 0x200)
        })
        .collect::<Vec<_>>();
    if pairs.len() != 1 {
        return Err(format!(
            "Expected one adjacent WaveBankMgr load/erase pair, found {}",
            pairs.len()
        ));
    }
    pairs[0][0].address()
}

fn build_stage_music_stub(
    stub_address: u32,
    hook: SoundStageHook,
    wave_loader: u32,
    overrides: &[RuntimeStageMusicOverride],
) -> Result<StageMusicStub, String> {
    const BGM_NONE: u32 = 0xffff_fff0;
    let stage_bgm_offset = hook
        .ms_stg_offset
        .checked_add(8)
        .ok_or_else(|| "MSStageInfo::stageBgm SDA offset overflows i16".to_string())?;
    let stage_bgm_silent_offset = hook
        .ms_stg_offset
        .checked_add(12)
        .ok_or_else(|| "MSStageInfo::stageBgmSilent SDA offset overflows i16".to_string())?;
    let stage_bgm_silent_status_offset = hook.ms_stg_offset.checked_add(16).ok_or_else(|| {
        "MSStageInfo::stageBgmSilentStartStatus SDA offset overflows i16".to_string()
    })?;
    let fade_event_offset = hook
        .ms_stg_offset
        .checked_add(20)
        .ok_or_else(|| "MSStageInfo::fadeEvent SDA offset overflows i16".to_string())?;
    let demo_bgm_offset = hook
        .ms_stg_offset
        .checked_add(4)
        .ok_or_else(|| "MSStageInfo::demoBgm SDA offset overflows i16".to_string())?;
    let switch_bgm_offset = hook
        .ms_stg_offset
        .checked_add(24)
        .ok_or_else(|| "MSStageInfo::switchBgm SDA offset overflows i16".to_string())?;
    let switch_bgm2_offset = hook
        .ms_stg_offset
        .checked_add(28)
        .ok_or_else(|| "MSStageInfo::switchBgm2 SDA offset overflows i16".to_string())?;
    let word_address = |index: usize| {
        stub_address
            .checked_add(
                u32::try_from(index)
                    .map_err(|_| "Stage music word index does not fit u32".to_string())?
                    .checked_mul(4)
                    .ok_or_else(|| "Stage music word offset overflows u32".to_string())?,
            )
            .ok_or_else(|| "Stage music word address overflows u32".to_string())
    };
    let mut words = Vec::new();
    let mut tail_branches = Vec::new();
    for override_ in overrides {
        let entry_word = words.len();
        words.push(encode_cmpwi(
            hook.area_register,
            i16::from(override_.area_index),
        ));
        let area_mismatch = words.len();
        words.push(0);
        words.push(encode_cmpwi(
            hook.scenario_register,
            i16::from(override_.scenario_index),
        ));
        let scenario_mismatch = words.len();
        words.push(0);

        if let Some(main_bgm_id) = override_.main.bgm_id() {
            // Snapshot the retail primary before replacing it. Any Game Default
            // role that aliases this non-silent cue follows the authored Main cue.
            words.push(encode_d_form(32, 11, 13, stage_bgm_offset));
            words.extend(encode_u32(0, main_bgm_id));
            words.push(encode_d_form(36, 0, 13, stage_bgm_offset));
            if let RuntimeMusicRoleOverride::Track {
                wave_scene_id: Some(wave_scene_id),
                ..
            } = override_.main
            {
                words.extend(encode_u32(12, wave_scene_id));
                words.push(encode_d_form(36, 12, 13, hook.ms_stg_offset));
            }
            words.extend(encode_u32(12, BGM_NONE));
            words.push(encode_cmplw(11, 12));
            let original_silent = words.len();
            words.push(0);
            for (role, offset) in [
                (override_.entrance, demo_bgm_offset),
                (override_.inside, stage_bgm_silent_offset),
                (override_.switch_a, switch_bgm_offset),
                (override_.switch_b, switch_bgm2_offset),
            ] {
                if role != RuntimeMusicRoleOverride::GameDefault {
                    continue;
                }
                words.push(encode_d_form(32, 12, 13, offset));
                words.push(encode_cmplw(12, 11));
                let not_alias = words.len();
                words.push(0);
                words.push(encode_d_form(36, 0, 13, offset));
                let after_store = words.len();
                words[not_alias] =
                    encode_bne(word_address(not_alias)?, word_address(after_store)?)?;
            }
            let after_aliases = words.len();
            words[original_silent] =
                encode_beq(word_address(original_silent)?, word_address(after_aliases)?)?;
        }

        for (role, offset) in [
            (override_.entrance, demo_bgm_offset),
            (override_.inside, stage_bgm_silent_offset),
            (override_.switch_a, switch_bgm_offset),
            (override_.switch_b, switch_bgm2_offset),
        ] {
            if let Some(bgm_id) = role.bgm_id() {
                words.extend(encode_u32(0, bgm_id));
                words.push(encode_d_form(36, 0, 13, offset));
            }
        }

        match override_.transition {
            RuntimeStageMusicTransition::GameDefault => {}
            RuntimeStageMusicTransition::Disabled => {
                words.push(encode_li(0, 0));
                words.push(encode_d_form(38, 0, 13, fade_event_offset));
            }
            RuntimeStageMusicTransition::InsideCrossfade => {
                words.push(encode_li(0, 2));
                words.push(encode_d_form(38, 0, 13, stage_bgm_silent_status_offset));
                words.push(encode_d_form(38, 0, 13, fade_event_offset));
            }
            RuntimeStageMusicTransition::SwitchRegion => {
                words.push(encode_li(0, 3));
                words.push(encode_d_form(38, 0, 13, fade_event_offset));
            }
        }

        let tail_branch = words.len();
        words.push(0);
        tail_branches.push(tail_branch);
        let next_word = words.len();
        words[area_mismatch] = encode_bne(word_address(area_mismatch)?, word_address(next_word)?)?;
        words[scenario_mismatch] =
            encode_bne(word_address(scenario_mismatch)?, word_address(next_word)?)?;
        debug_assert!(next_word > entry_word);
    }
    let tail_word = words.len();
    for branch in tail_branches {
        words[branch] = encode_branch(word_address(branch)?, word_address(tail_word)?, false)?;
    }
    words.push(encode_d_form(32, 4, 13, hook.ms_stg_offset));
    words.push(encode_branch(
        word_address(tail_word + 1)?,
        hook.dispatch_anchor
            .address()?
            .checked_add(4)
            .ok_or_else(|| "Stage music resume address overflows u32".to_string())?,
        false,
    )?);

    let wrapper_word = words.len();
    let wrapper_address = word_address(wrapper_word)?;
    words.extend([
        0x7c08_02a6, // mflr r0
        0x9001_0004, // stw r0, 4(r1)
        0x9421_ffe0, // stwu r1, -32(r1)
        0x93c1_0018, // stw r30, 24(r1)
        0x93e1_001c, // stw r31, 28(r1)
        0x7cbe_2b78, // mr r30, r5
        0x7cdf_3378, // mr r31, r6
    ]);
    let enter_stage_call = wrapper_address + 7 * 4;
    words.push(encode_branch(
        enter_stage_call,
        hook.enter_stage_target,
        true,
    )?);
    for override_ in overrides {
        let main_wave = override_.main.track().and_then(|(_, wave)| wave);
        let mut preload_waves = std::collections::BTreeSet::new();
        let active_roles = [
            Some(override_.entrance),
            matches!(
                override_.transition,
                RuntimeStageMusicTransition::GameDefault
                    | RuntimeStageMusicTransition::InsideCrossfade
            )
            .then_some(override_.inside),
            matches!(
                override_.transition,
                RuntimeStageMusicTransition::GameDefault
                    | RuntimeStageMusicTransition::SwitchRegion
            )
            .then_some(override_.switch_a),
            matches!(
                override_.transition,
                RuntimeStageMusicTransition::GameDefault
                    | RuntimeStageMusicTransition::SwitchRegion
            )
            .then_some(override_.switch_b),
        ];
        for role in active_roles.into_iter().flatten() {
            if let Some((_, Some(wave))) = role.track() {
                if Some(wave) != main_wave {
                    preload_waves.insert(wave);
                }
            }
        }
        for wave_scene in preload_waves {
            let entry_word = words.len();
            let entry_address = word_address(entry_word)?;
            words.push(encode_cmpwi(30, i16::from(override_.area_index)));
            let area_mismatch = words.len();
            words.push(0);
            words.push(encode_cmpwi(31, i16::from(override_.scenario_index)));
            let scenario_mismatch = words.len();
            words.push(0);
            words.push(encode_d_form(32, 0, 13, hook.ms_stg_offset));
            words.extend(encode_u32(11, wave_scene));
            words.push(encode_cmplw(0, 11));
            let already_loaded = words.len();
            words.push(0);
            words.push(encode_li(3, (wave_scene >> 8) as i16));
            words.push(encode_li(4, (wave_scene & 0xff) as i16));
            words.push(encode_branch(
                word_address(words.len())?,
                wave_loader,
                true,
            )?);
            let next_word = words.len();
            let next_address = word_address(next_word)?;
            words[area_mismatch] = encode_bne(entry_address + 4, next_address)?;
            words[scenario_mismatch] = encode_bne(word_address(scenario_mismatch)?, next_address)?;
            words[already_loaded] = encode_beq(word_address(already_loaded)?, next_address)?;
        }
    }
    words.extend([
        0x8001_0024, // lwz r0, 36(r1)
        0x83e1_001c, // lwz r31, 28(r1)
        0x83c1_0018, // lwz r30, 24(r1)
        0x7c08_03a6, // mtlr r0
        0x3821_0020, // addi r1, r1, 32
        PPC_BLR,
    ]);
    Ok(StageMusicStub {
        words,
        wrapper_word,
    })
}

fn parse_dol(source: &[u8]) -> Result<DolImage, String> {
    if source.len() < DOL_HEADER_SIZE {
        return Err(format!(
            "DOL is truncated: expected at least 0x{DOL_HEADER_SIZE:X} header bytes, found 0x{:X}",
            source.len()
        ));
    }
    if source.len() > u32::MAX as usize {
        return Err("DOL exceeds the 32-bit file-offset limit".to_string());
    }

    let mut sections = Vec::new();
    for slot in 0..DOL_TEXT_SECTION_COUNT {
        if let Some(section) = parse_section(
            source,
            true,
            slot,
            DOL_TEXT_FILE_OFFSETS,
            DOL_TEXT_ADDRESSES,
            DOL_TEXT_SIZES,
        )? {
            sections.push(section);
        }
    }
    for slot in 0..DOL_DATA_SECTION_COUNT {
        if let Some(section) = parse_section(
            source,
            false,
            slot,
            DOL_DATA_FILE_OFFSETS,
            DOL_DATA_ADDRESSES,
            DOL_DATA_SIZES,
        )? {
            sections.push(section);
        }
    }

    for (index, left) in sections.iter().enumerate() {
        for right in sections.iter().skip(index + 1) {
            if ranges_overlap(
                left.file_offset,
                left.file_end()?,
                right.file_offset,
                right.file_end()?,
            ) {
                return Err(format!(
                    "DOL sections {} and {} overlap in the file",
                    left.label(),
                    right.label()
                ));
            }
            if ranges_overlap(
                left.address,
                left.address_end()?,
                right.address,
                right.address_end()?,
            ) {
                return Err(format!(
                    "DOL sections {} and {} overlap in memory",
                    left.label(),
                    right.label()
                ));
            }
        }
    }

    let bss_address = read_be_u32(source, DOL_BSS_ADDRESS)?;
    let bss_size = read_be_u32(source, DOL_BSS_SIZE)?;
    let bss = match (bss_address, bss_size) {
        (0, 0) => None,
        (0, _) => return Err("DOL BSS has a size but no address".to_string()),
        (_, 0) => return Err("DOL BSS has an address but no size".to_string()),
        (address, size) => Some((
            address,
            address
                .checked_add(size)
                .ok_or_else(|| "DOL BSS address range overflows u32".to_string())?,
        )),
    };
    let entry_point = read_be_u32(source, DOL_ENTRY_POINT)?;
    if entry_point & 3 != 0 {
        return Err(format!(
            "DOL entry point 0x{entry_point:08X} is not word-aligned"
        ));
    }
    if !address_is_in_text(&sections, entry_point, 4)? {
        return Err(format!(
            "DOL entry point 0x{entry_point:08X} is outside every text section"
        ));
    }

    Ok(DolImage {
        sections,
        bss,
        entry_point,
    })
}

fn parse_section(
    source: &[u8],
    text: bool,
    slot: usize,
    file_table: usize,
    address_table: usize,
    size_table: usize,
) -> Result<Option<DolSection>, String> {
    let file_offset = read_be_u32(source, file_table + slot * 4)?;
    let address = read_be_u32(source, address_table + slot * 4)?;
    let size = read_be_u32(source, size_table + slot * 4)?;
    let label = format!("{}[{slot}]", if text { "text" } else { "data" });
    if size == 0 {
        if file_offset != 0 || address != 0 {
            return Err(format!(
                "DOL {label} is unused but has nonzero offset/address metadata"
            ));
        }
        return Ok(None);
    }
    if file_offset < DOL_HEADER_SIZE as u32 {
        return Err(format!(
            "DOL {label} starts inside the header at 0x{file_offset:X}"
        ));
    }
    if address == 0 {
        return Err(format!("DOL {label} has data but no load address"));
    }
    if text && (file_offset & 3 != 0 || address & 3 != 0 || size & 3 != 0) {
        return Err(format!("DOL {label} is not word-aligned"));
    }
    let file_end = file_offset
        .checked_add(size)
        .ok_or_else(|| format!("DOL {label} file range overflows u32"))?;
    if file_end as usize > source.len() {
        return Err(format!(
            "DOL {label} file range 0x{file_offset:X}..0x{file_end:X} exceeds the 0x{:X}-byte file",
            source.len()
        ));
    }
    address
        .checked_add(size)
        .ok_or_else(|| format!("DOL {label} address range overflows u32"))?;
    Ok(Some(DolSection {
        text,
        slot,
        file_offset,
        address,
        size,
    }))
}

fn find_nlogo_hook(source: &[u8], image: &DolImage) -> Result<NlogoHook, String> {
    let mut candidates = Vec::new();
    for section in image
        .sections
        .iter()
        .copied()
        .filter(|section| section.text)
    {
        let words = section_words(source, section)?;
        for word_index in 0..words.len().saturating_sub(4) {
            let sequence = &words[word_index..word_index + 5];
            let Some(global_register) = decode_lwz_from_r13(sequence[0]) else {
                continue;
            };
            if !is_cmpwi(sequence[1], global_register, 3)
                || !is_bne(sequence[2])
                || !is_li(sequence[3], 4)
                || !is_unconditional_branch(sequence[4])
            {
                continue;
            }
            let anchor = WordAnchor {
                section,
                word_index,
            };
            let address = anchor.address()?;
            let conditional_target = decode_conditional_branch_target(sequence[2], address + 8)?;
            let direct_target = decode_branch_target(sequence[4], address + 16)?;
            if conditional_target == direct_target {
                candidates.push(anchor);
            }
        }
    }
    let anchor = require_unique_anchor(candidates, "post-NLogo transition tail")?;
    let words = section_words(source, anchor.section)?;
    let next_state_register = register_t(words[anchor.word_index + 3]);
    let this_register = find_game_loop_this_register(&words, anchor.word_index)?;
    Ok(NlogoHook {
        anchor,
        this_register,
        next_state_register,
    })
}

fn find_nlogo_director_bypass(
    source: &[u8],
    hook: NlogoHook,
) -> Result<NlogoDirectorBypass, String> {
    let words = section_words(source, hook.anchor.section)?;
    let search_start = hook
        .anchor
        .word_index
        .saturating_sub(NLOGO_DIRECT_SEARCH_WORDS);
    let mut candidates = Vec::new();
    for word_index in search_start..hook.anchor.word_index.saturating_sub(13) {
        let sequence = &words[word_index..word_index + 14];
        let Some(flag_register) = decode_lwz_from_r13(sequence[0]) else {
            continue;
        };
        let Some((director_register, director_base, director_offset)) =
            decode_d_form(sequence[4], 32)
        else {
            continue;
        };
        if !is_li(sequence[1], 0)
            || !is_low_bit_test(sequence[2], flag_register)
            || !is_bne(sequence[3])
            || director_base != hook.this_register
            || director_offset != 4
        {
            continue;
        }
        let Some((vtable_register, vtable_base, vtable_offset)) = decode_d_form(sequence[5], 32)
        else {
            continue;
        };
        let Some((method_register, method_base, method_offset)) = decode_d_form(sequence[6], 32)
        else {
            continue;
        };
        if vtable_base != director_register
            || vtable_offset != 0
            || method_base != vtable_register
            || method_offset != 0x64
            || !is_mtctr(sequence[7], method_register)
            || sequence[8] != 0x4e80_0021
            || !is_cmpwi(sequence[9], 3, 4)
            || !is_bne(sequence[10])
            || decode_lwz_from_r13(sequence[11]) != Some(flag_register)
            || !is_ori(sequence[12], flag_register, flag_register, 1)
            || decode_d_form(sequence[13], 36)
                != Some((flag_register, 13, immediate_i16(sequence[0])))
        {
            continue;
        }
        let sequence_anchor = WordAnchor {
            section: hook.anchor.section,
            word_index,
        };
        let skip_target =
            decode_conditional_branch_target(sequence[3], sequence_anchor.address()? + 3 * 4)?;
        let result_skip_target =
            decode_conditional_branch_target(sequence[10], sequence_anchor.address()? + 10 * 4)?;
        let expected_skip_target = sequence_anchor
            .address()?
            .checked_add(14 * 4)
            .ok_or_else(|| "NLogo skip target overflows u32".to_string())?;
        if skip_target != expected_skip_target || result_skip_target != expected_skip_target {
            continue;
        }
        candidates.push(NlogoDirectorBypass {
            branch_anchor: WordAnchor {
                section: hook.anchor.section,
                word_index: word_index + 3,
            },
            completion_anchor: WordAnchor {
                section: hook.anchor.section,
                word_index: word_index + 11,
            },
        });
    }
    require_unique_value(candidates, "NLogo director completion path")
}

fn find_nlogo_setup_bypass(
    source: &[u8],
    image: &DolImage,
    this_register: u8,
) -> Result<NlogoSetupBypass, String> {
    let mut candidates = Vec::new();
    for section in image
        .sections
        .iter()
        .copied()
        .filter(|section| section.text)
    {
        let words = section_words(source, section)?;
        for word_index in 0..words.len().saturating_sub(13) {
            let sequence = &words[word_index..word_index + 14];
            let Some((display_register, display_base, display_offset)) =
                decode_d_form(sequence[0], 32)
            else {
                continue;
            };
            if display_register != 3
                || display_base != this_register
                || display_offset != 0x1c
                || !is_relative_bl(sequence[1])
                || !is_li(sequence[2], 0x48)
                || register_t(sequence[2]) != 3
                || !is_relative_bl(sequence[3])
            {
                continue;
            }
            let Some(object_register) = decode_or_dot_same_source(sequence[4], 3) else {
                continue;
            };
            if !is_beq(sequence[5])
                || !is_mr(sequence[6], 3, object_register)
                || !is_relative_bl(sequence[7])
                || decode_d_form(sequence[8], 36) != Some((object_register, this_register, 4))
                || !is_mr(sequence[9], 3, object_register)
                || decode_d_form(sequence[10], 32) != Some((4, this_register, 0x1c))
                || decode_d_form(sequence[11], 32) != Some((5, this_register, 0x20))
                || !is_relative_bl(sequence[12])
                || !is_unconditional_branch(sequence[13])
            {
                continue;
            }
            let case_anchor = WordAnchor {
                section,
                word_index,
            };
            let constructor_skip_address = case_anchor
                .address()?
                .checked_add(5 * 4)
                .ok_or_else(|| "NLogo constructor-skip branch address overflows u32".to_string())?;
            let constructor_skip_target =
                decode_conditional_branch_target(sequence[5], constructor_skip_address)?;
            if constructor_skip_target != case_anchor.address()? + 8 * 4 {
                continue;
            }
            let branch_address = case_anchor
                .address()?
                .checked_add(13 * 4)
                .ok_or_else(|| "NLogo setup resume branch address overflows u32".to_string())?;
            let resume_address = decode_branch_target(sequence[13], branch_address)?;
            if !address_is_in_text(&image.sections, resume_address, 4)? {
                continue;
            }
            candidates.push(NlogoSetupBypass {
                case_anchor,
                resume_address,
            });
        }
    }
    require_unique_value(candidates, "NLogo setup case")
}
fn find_game_loop_this_register(words: &[u32], hook_word: usize) -> Result<u8, String> {
    let start = hook_word.saturating_sub(THIS_SEARCH_WORDS);
    let mut candidates = Vec::new();
    for word_index in start..hook_word.saturating_sub(1) {
        let Some((state_register, this_register, _state_offset)) = decode_lbz(words[word_index])
        else {
            continue;
        };
        if this_register == 0 || !is_cmplwi(words[word_index + 1], state_register, 2) {
            continue;
        }
        let compare_end = (word_index + STATE_COMPARE_SEARCH_WORDS).min(hook_word);
        if words[word_index + 2..compare_end]
            .iter()
            .any(|word| is_cmplwi(*word, state_register, 3))
        {
            candidates.push(this_register);
        }
    }
    match candidates.as_slice() {
        [register] => Ok(*register),
        [] => Err(
            "Could not derive TApplication register from the game-loop state 2/3 comparisons"
                .to_string(),
        ),
        _ => Err(format!(
            "Ambiguous game-loop TApplication register anchor: found {} candidates",
            candidates.len()
        )),
    }
}

fn find_next_area_setter(source: &[u8], image: &DolImage) -> Result<NextAreaSetter, String> {
    let mut candidates = Vec::new();
    for section in image
        .sections
        .iter()
        .copied()
        .filter(|section| section.text)
    {
        let words = section_words(source, section)?;
        for word_index in 0..words.len().saturating_sub(4) {
            let sequence = &words[word_index..word_index + 5];
            if !is_li(sequence[0], 15) || !is_li(sequence[1], 0) {
                continue;
            }
            let stage_register = register_t(sequence[0]);
            let zero_register = register_t(sequence[1]);
            if stage_register == zero_register {
                continue;
            }
            let Some((stored_stage, base_register, next_offset)) = decode_d_form(sequence[2], 38)
            else {
                continue;
            };
            let Some((stored_zero_byte, byte_base, scenario_offset)) =
                decode_d_form(sequence[3], 38)
            else {
                continue;
            };
            let Some((stored_zero_half, half_base, flag_offset)) = decode_d_form(sequence[4], 44)
            else {
                continue;
            };
            if stored_stage != stage_register
                || stored_zero_byte != zero_register
                || stored_zero_half != zero_register
                || base_register == 0
                || byte_base != base_register
                || half_base != base_register
                || next_offset.checked_add(1) != Some(scenario_offset)
                || next_offset.checked_add(2) != Some(flag_offset)
            {
                continue;
            }
            candidates.push(NextAreaSetter {
                anchor: WordAnchor {
                    section,
                    word_index,
                },
                base_register,
                next_area_offset: next_offset,
            });
        }
    }
    require_unique_value(candidates, "next-area 15/0/0 setter")
}

fn find_movie_hook(
    source: &[u8],
    image: &DolImage,
    setter: NextAreaSetter,
) -> Result<MovieHook, String> {
    let words = section_words(source, setter.anchor.section)?;
    let search_start = setter.anchor.word_index.saturating_sub(MOVIE_SEARCH_WORDS);
    let mut candidates = Vec::new();
    for word_index in search_start..setter.anchor.word_index.saturating_sub(3) {
        let sequence = &words[word_index..word_index + 4];
        if !is_mr_r3(sequence[0], setter.base_register)
            || !is_relative_bl(sequence[1])
            || !is_clrlwi_dot_r0_r3_24(sequence[2])
            || !is_beq(sequence[3])
        {
            continue;
        }
        let sequence_anchor = WordAnchor {
            section: setter.anchor.section,
            word_index,
        };
        let call_address = sequence_anchor
            .address()?
            .checked_add(4)
            .ok_or_else(|| "Movie call address overflows u32".to_string())?;
        let original_target = decode_branch_target(sequence[1], call_address)?;
        if !address_is_in_text(&image.sections, original_target, 4)? {
            continue;
        }
        candidates.push(MovieHook {
            call_anchor: WordAnchor {
                section: setter.anchor.section,
                word_index: word_index + 1,
            },
            original_target,
        });
    }
    require_unique_value(candidates, "checkAdditionalMovie call")
}

fn find_death_restart_hook(
    source: &[u8],
    image: &DolImage,
    next_area_offset: i16,
) -> Result<DeathRestartHook, String> {
    let mut candidates = Vec::new();
    for section in image
        .sections
        .iter()
        .copied()
        .filter(|section| section.text)
    {
        let words = section_words(source, section)?;
        for word_index in 0..words.len().saturating_sub(7) {
            if !is_relative_bl(words[word_index]) {
                continue;
            }
            let Some((loaded_flags, director_register, flags_offset)) =
                decode_d_form(words[word_index + 1], 40)
            else {
                continue;
            };
            let director_forwarded = decode_d_form(words[word_index + 2], 14)
                == Some((3, director_register, 0))
                || is_mr(words[word_index + 2], 3, director_register);
            if loaded_flags != 0
                || flags_offset != 0x4c
                || !director_forwarded
                || decode_d_form(words[word_index + 4], 44) != Some((0, director_register, 0x4c))
                || !is_relative_bl(words[word_index + 5])
                || !is_li(words[word_index + 6], 15)
            {
                continue;
            }

            let call_anchor = WordAnchor {
                section,
                word_index,
            };
            let resume_address = call_anchor
                .address()?
                .checked_add(4)
                .ok_or_else(|| "Death-restart resume address overflows u32".to_string())?;
            let search_start = word_index.saturating_sub(0x60);
            let mut current_registers = Vec::new();
            for branch_word in search_start..word_index {
                if !is_unconditional_branch(words[branch_word]) {
                    continue;
                }
                let branch_anchor = WordAnchor {
                    section,
                    word_index: branch_word,
                };
                if decode_branch_target(words[branch_word], branch_anchor.address()?)?
                    != resume_address
                {
                    continue;
                }
                if let Some(register) = find_special_miss_restart_current_register(
                    &words[search_start..branch_word],
                    next_area_offset,
                ) {
                    current_registers.push(register);
                }
            }
            current_registers.sort_unstable();
            current_registers.dedup();
            if let [current_area_register] = current_registers.as_slice() {
                candidates.push(DeathRestartHook {
                    call_anchor,
                    current_area_register: *current_area_register,
                });
            }
        }
    }
    require_unique_value(candidates, "ordinary-stage miss redirect")
}

fn find_special_miss_restart_current_register(words: &[u32], next_area_offset: i16) -> Option<u8> {
    for first_store in (0..words.len()).rev() {
        let store_opcode = opcode(words[first_store]);
        if store_opcode != 38 && store_opcode != 39 {
            continue;
        }
        let Some((stored_stage, application_register, stage_offset)) =
            decode_d_form(words[first_store], store_opcode)
        else {
            continue;
        };
        if stored_stage != 0 || application_register == 0 || stage_offset != next_area_offset {
            continue;
        }
        let updated_base = store_opcode == 39;
        let scenario_offset = if updated_base {
            1
        } else {
            next_area_offset.checked_add(1)?
        };
        let flag_offset = if updated_base {
            2
        } else {
            next_area_offset.checked_add(2)?
        };
        let tail = &words[first_store + 1..];
        let has_scenario_store = tail.iter().any(|word| {
            decode_d_form(*word, 38) == Some((0, application_register, scenario_offset))
        });
        let has_flag_store = tail
            .iter()
            .any(|word| decode_d_form(*word, 44) == Some((0, application_register, flag_offset)));
        if !has_scenario_store || !has_flag_store {
            continue;
        }
        let current_search_start = first_store.saturating_sub(8);
        for word in words[current_search_start..first_store].iter().rev() {
            if let Some((loaded_stage, current_area_register, 0)) = decode_lbz(*word) {
                if loaded_stage == 0 && current_area_register != 0 {
                    return Some(current_area_register);
                }
            }
        }
    }
    None
}

fn find_stack_top(source: &[u8], image: &DolImage) -> Result<u32, String> {
    let entry_section = image
        .sections
        .iter()
        .copied()
        .find(|section| {
            section.text
                && image.entry_point >= section.address
                && image.entry_point < section.address.saturating_add(section.size)
        })
        .ok_or_else(|| "DOL entry point is not in a text section".to_string())?;
    let entry_words = section_words(source, entry_section)?;
    let entry_word = usize::try_from((image.entry_point - entry_section.address) / 4)
        .map_err(|_| "Entry-point word index does not fit usize".to_string())?;
    let search_end = (entry_word + ENTRY_BL_SEARCH_WORDS).min(entry_words.len());
    let mut init_registers_target = None;
    for (word_index, word) in entry_words
        .iter()
        .enumerate()
        .take(search_end)
        .skip(entry_word)
    {
        if is_relative_bl(*word) {
            let address = entry_section
                .address
                .checked_add(
                    u32::try_from(word_index)
                        .map_err(|_| "Entry BL word index does not fit u32".to_string())?
                        * 4,
                )
                .ok_or_else(|| "Entry BL address overflows u32".to_string())?;
            let target = decode_branch_target(*word, address)?;
            if !address_is_in_text(&image.sections, target, 4)? {
                return Err(format!(
                    "DOL entry point's first BL targets 0x{target:08X}, outside all text sections"
                ));
            }
            init_registers_target = Some(target);
            break;
        }
    }
    let init_registers_target = init_registers_target.ok_or_else(|| {
        "Could not find the DOL entry point's initial register-setup BL".to_string()
    })?;
    let init_section = image
        .sections
        .iter()
        .copied()
        .find(|section| {
            section.text
                && init_registers_target >= section.address
                && init_registers_target < section.address.saturating_add(section.size)
        })
        .ok_or_else(|| "Initial register setup is outside all text sections".to_string())?;
    let init_words = section_words(source, init_section)?;
    let init_word = usize::try_from((init_registers_target - init_section.address) / 4)
        .map_err(|_| "Register-setup word index does not fit usize".to_string())?;
    let search_end = (init_word + INIT_REGISTER_SEARCH_WORDS).min(init_words.len());
    let mut candidates = Vec::new();
    for word_index in init_word..search_end.saturating_sub(1) {
        if let Some(value) =
            decode_materialized_address(init_words[word_index], init_words[word_index + 1])
                .filter(|_| register_t(init_words[word_index]) == 1)
        {
            candidates.push(value);
        }
        if init_words[word_index] == PPC_BLR {
            break;
        }
    }
    match candidates.as_slice() {
        [stack_top] => Ok(*stack_top),
        [] => Err("Could not derive r1 stack top from the initial register setup".to_string()),
        _ => Err(format!(
            "Ambiguous r1 stack-top setup: found {} materializations",
            candidates.len()
        )),
    }
}

#[allow(dead_code, clippy::too_many_arguments)]
fn build_stub(
    stub_address: u32,
    wrapper_address: u32,
    hook_address: u32,
    original_movie_target: u32,
    this_register: u8,
    next_state_register: u8,
    next_area_offset: i16,
    target: &RuntimeStageTarget,
) -> Result<Vec<u8>, String> {
    let scenario_offset = next_area_offset
        .checked_add(1)
        .ok_or_else(|| "Next-area scenario offset overflows i16".to_string())?;
    let next_flag_offset = next_area_offset
        .checked_add(2)
        .ok_or_else(|| "Next-area flag offset overflows i16".to_string())?;
    let current_flag_offset = next_flag_offset
        .checked_sub(4)
        .ok_or_else(|| "Current-area flag offset underflows i16".to_string())?;

    let mut words = Vec::with_capacity(
        usize::try_from(TRANSITION_WORD_COUNT + MOVIE_WRAPPER_WORD_COUNT)
            .map_err(|_| "Direct-boot word count does not fit usize".to_string())?,
    );
    words.push(encode_li(0, i16::from(target.area_index)));
    words.push(encode_d_form(38, 0, this_register, next_area_offset));
    words.push(encode_li(0, i16::from(target.scenario_index)));
    words.push(encode_d_form(38, 0, this_register, scenario_offset));
    words.push(encode_li(0, DIRECT_BOOT_FLAG as i16));
    words.push(encode_d_form(44, 0, this_register, next_flag_offset));
    words.push(encode_li(next_state_register, POST_NLOGO_STATE));
    let transition_branch_address = stub_address
        .checked_add(7 * 4)
        .ok_or_else(|| "Transition return-branch address overflows u32".to_string())?;
    words.push(encode_branch(
        transition_branch_address,
        hook_address
            .checked_add(4)
            .ok_or_else(|| "Transition resume address overflows u32".to_string())?,
        false,
    )?);

    words.push(encode_d_form(40, 0, 3, current_flag_offset));
    words.push(encode_cmplwi(0, DIRECT_BOOT_FLAG));
    let wrapper_bne_address = wrapper_address
        .checked_add(2 * 4)
        .ok_or_else(|| "Movie-wrapper branch address overflows u32".to_string())?;
    let wrapper_tail_address = wrapper_address
        .checked_add(8 * 4)
        .ok_or_else(|| "Movie-wrapper tail address overflows u32".to_string())?;
    words.push(encode_bne(wrapper_bne_address, wrapper_tail_address)?);
    words.push(encode_li(0, 0));
    words.push(encode_d_form(44, 0, 3, current_flag_offset));
    words.push(encode_d_form(44, 0, 3, next_flag_offset));
    words.push(encode_li(3, 0));
    words.push(PPC_BLR);
    words.push(encode_branch(
        wrapper_tail_address,
        original_movie_target,
        false,
    )?);

    if words.len() as u32 != TRANSITION_WORD_COUNT + MOVIE_WRAPPER_WORD_COUNT {
        return Err("Internal direct-boot stub word-count mismatch".to_string());
    }
    let mut payload = Vec::with_capacity(words.len() * 4 + DIRECT_BOOT_MARKER.len());
    for word in words {
        payload.extend_from_slice(&word.to_be_bytes());
    }
    payload.extend_from_slice(DIRECT_BOOT_MARKER);
    let aligned_len = align_up_usize(payload.len(), FILE_ALIGNMENT as usize)?;
    payload.resize(aligned_len, 0);
    Ok(payload)
}

#[allow(dead_code)]
fn reject_injected_range_overlap(
    image: &DolImage,
    stub_address: u32,
    stub_end: u32,
) -> Result<(), String> {
    for section in &image.sections {
        if ranges_overlap(
            stub_address,
            stub_end,
            section.address,
            section.address_end()?,
        ) {
            return Err(format!(
                "Direct-boot text range 0x{stub_address:08X}..0x{stub_end:08X} overlaps DOL {}",
                section.label()
            ));
        }
    }
    if let Some((bss_start, bss_end)) = image.bss {
        if ranges_overlap(stub_address, stub_end, bss_start, bss_end) {
            return Err(format!(
                "Direct-boot text range 0x{stub_address:08X}..0x{stub_end:08X} overlaps DOL BSS 0x{bss_start:08X}..0x{bss_end:08X}"
            ));
        }
    }
    Ok(())
}

fn build_transition_cave(
    cave_address: u32,
    original_transition_target: u32,
    this_register: u8,
    next_state_register: u8,
    next_area_offset: i16,
    target: &RuntimeStageTarget,
) -> Result<Vec<u32>, String> {
    let scenario_offset = next_area_offset
        .checked_add(1)
        .ok_or_else(|| "Next-area scenario offset overflows i16".to_string())?;
    let next_flag_offset = next_area_offset
        .checked_add(2)
        .ok_or_else(|| "Next-area flag offset overflows i16".to_string())?;
    let return_address = cave_address
        .checked_add(24)
        .ok_or_else(|| "Transition return-branch address overflows u32".to_string())?;
    let words = vec![
        // The replaced hook instruction already loaded the area into the
        // next-state register, which keeps this within linker alignment space.
        encode_d_form(38, next_state_register, this_register, next_area_offset),
        encode_li(next_state_register, i16::from(target.scenario_index)),
        encode_d_form(38, next_state_register, this_register, scenario_offset),
        encode_li(0, DIRECT_BOOT_FLAG as i16),
        encode_d_form(44, 0, this_register, next_flag_offset),
        encode_li(next_state_register, POST_NLOGO_STATE),
        encode_branch(return_address, original_transition_target, false)?,
    ];
    debug_assert_eq!(words.len(), TRANSITION_CAVE_WORDS);
    Ok(words)
}

fn build_movie_caves(
    primary_address: u32,
    secondary_address: u32,
    original_movie_target: u32,
    next_area_offset: i16,
) -> Result<(Vec<u32>, Vec<u32>), String> {
    let next_flag_offset = next_area_offset
        .checked_add(2)
        .ok_or_else(|| "Next-area flag offset overflows i16".to_string())?;
    let current_flag_offset = next_flag_offset
        .checked_sub(4)
        .ok_or_else(|| "Current-area flag offset underflows i16".to_string())?;
    let secondary_tail_address = secondary_address
        .checked_add(8)
        .ok_or_else(|| "Movie secondary tail address overflows u32".to_string())?;
    let primary_bne_address = primary_address
        .checked_add(8)
        .ok_or_else(|| "Movie primary condition address overflows u32".to_string())?;
    let primary_return_address = primary_address
        .checked_add(24)
        .ok_or_else(|| "Movie primary return address overflows u32".to_string())?;
    let primary = vec![
        encode_d_form(40, 0, 3, current_flag_offset),
        encode_cmplwi(0, DIRECT_BOOT_FLAG),
        encode_bne(primary_bne_address, secondary_tail_address)?,
        encode_li(0, 0),
        encode_d_form(44, 0, 3, current_flag_offset),
        encode_d_form(44, 0, 3, next_flag_offset),
        encode_branch(primary_return_address, secondary_address, false)?,
    ];
    let secondary = vec![
        encode_li(3, 0),
        PPC_BLR,
        encode_branch(secondary_tail_address, original_movie_target, false)?,
    ];
    debug_assert_eq!(primary.len(), MOVIE_PRIMARY_CAVE_WORDS);
    debug_assert_eq!(secondary.len(), MOVIE_SECONDARY_CAVE_WORDS);
    Ok((primary, secondary))
}

fn build_death_restart_cave(current_area_register: u8, target: &RuntimeStageTarget) -> Vec<u32> {
    let words = vec![
        encode_li(0, i16::from(target.area_index)),
        encode_d_form(38, 0, current_area_register, 4),
        encode_li(0, i16::from(target.scenario_index)),
        encode_d_form(38, 0, current_area_register, 5),
        encode_li(0, 0),
        encode_d_form(44, 0, current_area_register, 6),
        PPC_BLR,
    ];
    debug_assert_eq!(words.len(), DEATH_RESTART_CAVE_WORDS);
    words
}

fn find_zero_alignment_code_caves(
    source: &[u8],
    image: &DolImage,
) -> Result<Vec<CodeCave>, String> {
    let mut branch_targets = Vec::new();
    for section in image
        .sections
        .iter()
        .copied()
        .filter(|section| section.text)
    {
        let words = section_words(source, section)?;
        for (word_index, word) in words.iter().copied().enumerate() {
            let address = WordAnchor {
                section,
                word_index,
            }
            .address()?;
            if opcode(word) == 18 && word & 2 == 0 {
                branch_targets.push(decode_branch_target(word, address)?);
            } else if opcode(word) == 16 && word & 2 == 0 {
                let displacement = sign_extend(word & 0x0000_fffc, 16);
                branch_targets.push(add_signed_address(address, displacement)?);
            }
        }
    }

    let mut caves = Vec::new();
    for section in image.sections.iter().copied().filter(|section| {
        section.text
            && !(image.entry_point >= section.address
                && image.entry_point < section.address.saturating_add(section.size))
    }) {
        let words = section_words(source, section)?;
        let mut index = 1_usize;
        while index < words.len() {
            if words[index] != 0 {
                index += 1;
                continue;
            }
            let start = index;
            while index < words.len() && words[index] == 0 {
                index += 1;
            }
            let word_count = index - start;
            if words[start - 1] != PPC_BLR || index >= words.len() {
                continue;
            }
            let anchor = WordAnchor {
                section,
                word_index: start,
            };
            let start_address = anchor.address()?;
            let byte_count = u32::try_from(word_count)
                .ok()
                .and_then(|count| count.checked_mul(4))
                .ok_or_else(|| "Code-cave byte count overflows u32".to_string())?;
            let end_address = start_address
                .checked_add(byte_count)
                .ok_or_else(|| "Code-cave address range overflows u32".to_string())?;
            if end_address & 0x1f != 0
                || branch_targets
                    .iter()
                    .any(|target| *target >= start_address && *target < end_address)
            {
                continue;
            }
            caves.push(CodeCave { anchor, word_count });
        }
    }
    caves.sort_by_key(|cave| cave.anchor.address().unwrap_or(u32::MAX));
    Ok(caves)
}

fn choose_direct_boot_caves(
    caves: Vec<CodeCave>,
    hook_address: u32,
    movie_hook_address: u32,
    death_hook_address: u32,
    original_transition_target: u32,
    original_movie_target: u32,
) -> Result<DirectBootCaves, String> {
    for transition in caves
        .iter()
        .copied()
        .filter(|cave| cave.word_count >= TRANSITION_CAVE_WORDS)
    {
        let transition_address = transition.anchor.address()?;
        if encode_branch(hook_address, transition_address, false).is_err()
            || encode_branch(transition_address + 24, original_transition_target, false).is_err()
        {
            continue;
        }
        for movie_primary in caves
            .iter()
            .copied()
            .filter(|cave| cave.word_count >= MOVIE_PRIMARY_CAVE_WORDS)
        {
            if movie_primary.anchor == transition.anchor {
                continue;
            }
            let primary_address = movie_primary.anchor.address()?;
            if encode_branch(movie_hook_address, primary_address, true).is_err() {
                continue;
            }
            for movie_secondary in caves
                .iter()
                .copied()
                .filter(|cave| cave.word_count >= MOVIE_SECONDARY_CAVE_WORDS)
            {
                if movie_secondary.anchor == transition.anchor
                    || movie_secondary.anchor == movie_primary.anchor
                {
                    continue;
                }
                let secondary_address = movie_secondary.anchor.address()?;
                let secondary_tail_address = secondary_address + 8;
                if encode_bne(primary_address + 8, secondary_tail_address).is_err()
                    || encode_branch(primary_address + 24, secondary_address, false).is_err()
                    || encode_branch(secondary_tail_address, original_movie_target, false).is_err()
                {
                    continue;
                }
                for death_restart in caves
                    .iter()
                    .copied()
                    .filter(|cave| cave.word_count >= DEATH_RESTART_CAVE_WORDS)
                {
                    if death_restart.anchor == transition.anchor
                        || death_restart.anchor == movie_primary.anchor
                        || death_restart.anchor == movie_secondary.anchor
                    {
                        continue;
                    }
                    if encode_branch(death_hook_address, death_restart.anchor.address()?, true)
                        .is_ok()
                    {
                        return Ok(DirectBootCaves {
                            transition,
                            movie_primary,
                            movie_secondary,
                            death_restart,
                        });
                    }
                }
            }
        }
    }
    Err(format!(
        "Could not find four safe executable alignment caves for direct boot (need {TRANSITION_CAVE_WORDS}, {MOVIE_PRIMARY_CAVE_WORDS}, {MOVIE_SECONDARY_CAVE_WORDS}, and {DEATH_RESTART_CAVE_WORDS} words)"
    ))
}

fn section_word(source: &[u8], anchor: WordAnchor, relative_word: usize) -> Result<u32, String> {
    let relative_bytes = relative_word
        .checked_mul(4)
        .ok_or_else(|| "Section word offset overflows usize".to_string())?;
    read_be_u32(
        source,
        anchor
            .file_offset()?
            .checked_add(relative_bytes)
            .ok_or_else(|| "Section word file offset overflows usize".to_string())?,
    )
}

fn write_words(destination: &mut [u8], anchor: WordAnchor, words: &[u32]) -> Result<(), String> {
    let start = anchor.file_offset()?;
    for (index, word) in words.iter().copied().enumerate() {
        let relative_bytes = index
            .checked_mul(4)
            .ok_or_else(|| "Code-cave word offset overflows usize".to_string())?;
        write_be_u32(
            destination,
            start
                .checked_add(relative_bytes)
                .ok_or_else(|| "Code-cave file offset overflows usize".to_string())?,
            word,
        )?;
    }
    Ok(())
}

fn require_unique_anchor(
    candidates: Vec<WordAnchor>,
    description: &str,
) -> Result<WordAnchor, String> {
    require_unique_value(candidates, description)
}

fn require_unique_value<T>(mut candidates: Vec<T>, description: &str) -> Result<T, String> {
    match candidates.len() {
        0 => Err(format!("Could not locate semantic {description} anchor")),
        1 => Ok(candidates.remove(0)),
        count => Err(format!(
            "Ambiguous semantic {description} anchor: found {count} candidates"
        )),
    }
}

fn section_words(source: &[u8], section: DolSection) -> Result<Vec<u32>, String> {
    if !section.text {
        return Err(format!(
            "Attempted to decode non-text DOL {} as instructions",
            section.label()
        ));
    }
    let start = usize::try_from(section.file_offset)
        .map_err(|_| format!("DOL {} file offset does not fit usize", section.label()))?;
    let size = usize::try_from(section.size)
        .map_err(|_| format!("DOL {} size does not fit usize", section.label()))?;
    let end = start
        .checked_add(size)
        .ok_or_else(|| format!("DOL {} file range overflows usize", section.label()))?;
    let bytes = source
        .get(start..end)
        .ok_or_else(|| format!("DOL {} file range is truncated", section.label()))?;
    let mut words = Vec::with_capacity(bytes.len() / 4);
    for chunk in bytes.chunks_exact(4) {
        words.push(u32::from_be_bytes(
            chunk
                .try_into()
                .map_err(|_| "Instruction chunk is not four bytes".to_string())?,
        ));
    }
    Ok(words)
}

fn address_is_in_text(sections: &[DolSection], address: u32, size: u32) -> Result<bool, String> {
    let end = address
        .checked_add(size)
        .ok_or_else(|| "Instruction address range overflows u32".to_string())?;
    for section in sections.iter().filter(|section| section.text) {
        if address >= section.address && end <= section.address_end()? {
            return Ok(true);
        }
    }
    Ok(false)
}

fn decode_lwz_from_r13(word: u32) -> Option<u8> {
    (opcode(word) == 32 && register_a(word) == 13).then(|| register_t(word))
}

fn decode_lbz(word: u32) -> Option<(u8, u8, i16)> {
    (opcode(word) == 34).then(|| (register_t(word), register_a(word), immediate_i16(word)))
}

fn decode_d_form(word: u32, expected_opcode: u8) -> Option<(u8, u8, i16)> {
    (opcode(word) == expected_opcode)
        .then(|| (register_t(word), register_a(word), immediate_i16(word)))
}

fn is_cmpwi(word: u32, register: u8, immediate: i16) -> bool {
    opcode(word) == 11
        && word & 0x03e0_0000 == 0
        && register_a(word) == register
        && immediate_i16(word) == immediate
}

fn is_cmplwi(word: u32, register: u8, immediate: u16) -> bool {
    opcode(word) == 10
        && word & 0x03e0_0000 == 0
        && register_a(word) == register
        && immediate_u16(word) == immediate
}

fn is_li(word: u32, immediate: i16) -> bool {
    opcode(word) == 14 && register_a(word) == 0 && immediate_i16(word) == immediate
}

fn is_bne(word: u32) -> bool {
    word & 0xffff_0003 == 0x4082_0000
}

fn is_beq(word: u32) -> bool {
    word & 0xffff_0003 == 0x4182_0000
}

fn is_unconditional_branch(word: u32) -> bool {
    opcode(word) == 18 && word & 3 == 0
}

fn is_relative_bl(word: u32) -> bool {
    opcode(word) == 18 && word & 3 == 1
}

fn decode_or_dot_same_source(word: u32, source_register: u8) -> Option<u8> {
    (opcode(word) == 31
        && ((word >> 1) & 0x3ff) == 444
        && word & 1 == 1
        && register_t(word) == source_register
        && ((word >> 11) & 0x1f) as u8 == source_register)
        .then(|| register_a(word))
}

fn is_mr(word: u32, target_register: u8, source_register: u8) -> bool {
    opcode(word) == 31
        && ((word >> 1) & 0x3ff) == 444
        && word & 1 == 0
        && register_t(word) == source_register
        && register_a(word) == target_register
        && ((word >> 11) & 0x1f) as u8 == source_register
}

fn is_mtctr(word: u32, source_register: u8) -> bool {
    word & !0x03e0_0000 == 0x7c08_03a6 && register_t(word) == source_register
}

fn is_ori(word: u32, source_register: u8, target_register: u8, immediate: u16) -> bool {
    opcode(word) == 24
        && register_t(word) == source_register
        && register_a(word) == target_register
        && immediate_u16(word) == immediate
}

fn is_low_bit_test(word: u32, register: u8) -> bool {
    opcode(word) == 21
        && register_t(word) == register
        && register_a(word) == register
        && (word >> 11) & 0x1f == 0
        && (word >> 6) & 0x1f == 31
        && (word >> 1) & 0x1f == 31
        && word & 1 == 1
}

fn is_mr_r3(word: u32, source_register: u8) -> bool {
    opcode(word) == 31
        && ((word >> 1) & 0x3ff) == 444
        && word & 1 == 0
        && register_t(word) == source_register
        && register_a(word) == 3
        && ((word >> 11) & 0x1f) as u8 == source_register
}

fn is_clrlwi_dot_r0_r3_24(word: u32) -> bool {
    opcode(word) == 21
        && register_t(word) == 3
        && register_a(word) == 0
        && (word >> 11) & 0x1f == 0
        && (word >> 6) & 0x1f == 24
        && (word >> 1) & 0x1f == 31
        && word & 1 == 1
}

fn decode_materialized_address(lis: u32, low: u32) -> Option<u32> {
    if opcode(lis) != 15 || register_a(lis) != 0 {
        return None;
    }
    let register = register_t(lis);
    let upper = u32::from(immediate_u16(lis)) << 16;
    if opcode(low) == 24 && register_t(low) == register && register_a(low) == register {
        Some(upper | u32::from(immediate_u16(low)))
    } else if opcode(low) == 14 && register_t(low) == register && register_a(low) == register {
        Some(upper.wrapping_add_signed(i32::from(immediate_i16(low))))
    } else {
        None
    }
}

fn decode_branch_target(word: u32, address: u32) -> Result<u32, String> {
    if opcode(word) != 18 || word & 2 != 0 {
        return Err(format!(
            "Instruction 0x{word:08X} at 0x{address:08X} is not a relative direct branch"
        ));
    }
    let displacement = sign_extend(word & 0x03ff_fffc, 26);
    add_signed_address(address, displacement)
}

fn decode_conditional_branch_target(word: u32, address: u32) -> Result<u32, String> {
    if opcode(word) != 16 || word & 3 != 0 {
        return Err(format!(
            "Instruction 0x{word:08X} at 0x{address:08X} is not a relative conditional branch"
        ));
    }
    let displacement = sign_extend(word & 0x0000_fffc, 16);
    add_signed_address(address, displacement)
}

fn encode_branch(from: u32, to: u32, link: bool) -> Result<u32, String> {
    if from & 3 != 0 || to & 3 != 0 {
        return Err(format!(
            "PowerPC branch endpoints must be word-aligned: 0x{from:08X} -> 0x{to:08X}"
        ));
    }
    let displacement = i64::from(to) - i64::from(from);
    if !(-0x0200_0000..=0x01ff_fffc).contains(&displacement) {
        return Err(format!(
            "PowerPC branch 0x{from:08X} -> 0x{to:08X} is out of the signed 26-bit range"
        ));
    }
    Ok(0x4800_0000 | ((displacement as i32 as u32) & 0x03ff_fffc) | u32::from(link))
}

fn encode_bne(from: u32, to: u32) -> Result<u32, String> {
    let displacement = i64::from(to) - i64::from(from);
    if from & 3 != 0 || to & 3 != 0 || !(-0x8000..=0x7ffc).contains(&displacement) {
        return Err(format!(
            "PowerPC conditional branch 0x{from:08X} -> 0x{to:08X} is out of range or unaligned"
        ));
    }
    Ok(0x4082_0000 | ((displacement as i32 as u32) & 0x0000_fffc))
}

fn encode_beq(from: u32, to: u32) -> Result<u32, String> {
    let displacement = i64::from(to) - i64::from(from);
    if from & 3 != 0 || to & 3 != 0 || !(-0x8000..=0x7ffc).contains(&displacement) {
        return Err(format!(
            "PowerPC conditional branch 0x{from:08X} -> 0x{to:08X} is out of range or unaligned"
        ));
    }
    Ok(0x4182_0000 | ((displacement as i32 as u32) & 0x0000_fffc))
}

fn encode_bdnz(from: u32, to: u32) -> Result<u32, String> {
    let displacement = i64::from(to) - i64::from(from);
    if from & 3 != 0 || to & 3 != 0 || !(-0x8000..=0x7ffc).contains(&displacement) {
        return Err(format!(
            "PowerPC count branch 0x{from:08X} -> 0x{to:08X} is out of range or unaligned"
        ));
    }
    Ok(0x4200_0000 | ((displacement as i32 as u32) & 0x0000_fffc))
}

fn encode_cmplw(left_register: u8, right_register: u8) -> u32 {
    0x7c00_0040 | (u32::from(left_register) << 16) | (u32::from(right_register) << 11)
}

fn encode_mr_word(target_register: u8, source_register: u8) -> u32 {
    (31_u32 << 26)
        | (u32::from(source_register) << 21)
        | (u32::from(target_register) << 16)
        | (u32::from(source_register) << 11)
        | (444_u32 << 1)
}

fn encode_li(register: u8, immediate: i16) -> u32 {
    0x3800_0000 | (u32::from(register) << 21) | u32::from(immediate as u16)
}

fn encode_cmpwi(register: u8, immediate: i16) -> u32 {
    0x2c00_0000 | (u32::from(register) << 16) | u32::from(immediate as u16)
}

fn encode_u32(register: u8, value: u32) -> [u32; 2] {
    [
        encode_d_form(15, register, 0, (value >> 16) as u16 as i16),
        (24_u32 << 26)
            | (u32::from(register) << 21)
            | (u32::from(register) << 16)
            | u32::from(value as u16),
    ]
}

fn encode_cmplwi(register: u8, immediate: u16) -> u32 {
    0x2800_0000 | (u32::from(register) << 16) | u32::from(immediate)
}

fn encode_d_form(opcode: u8, register_t: u8, register_a: u8, immediate: i16) -> u32 {
    (u32::from(opcode) << 26)
        | (u32::from(register_t) << 21)
        | (u32::from(register_a) << 16)
        | u32::from(immediate as u16)
}

fn opcode(word: u32) -> u8 {
    (word >> 26) as u8
}

fn register_t(word: u32) -> u8 {
    ((word >> 21) & 0x1f) as u8
}

fn register_a(word: u32) -> u8 {
    ((word >> 16) & 0x1f) as u8
}

fn immediate_u16(word: u32) -> u16 {
    word as u16
}

fn immediate_i16(word: u32) -> i16 {
    word as u16 as i16
}

fn sign_extend(value: u32, bits: u32) -> i32 {
    let shift = 32 - bits;
    ((value << shift) as i32) >> shift
}

fn add_signed_address(address: u32, displacement: i32) -> Result<u32, String> {
    let result = i64::from(address) + i64::from(displacement);
    u32::try_from(result).map_err(|_| {
        format!(
            "Relative branch from 0x{address:08X} with displacement {displacement} overflows u32"
        )
    })
}

fn ranges_overlap(left_start: u32, left_end: u32, right_start: u32, right_end: u32) -> bool {
    left_start < right_end && right_start < left_end
}

fn align_up_usize(value: usize, alignment: usize) -> Result<usize, String> {
    if !alignment.is_power_of_two() {
        return Err(format!("Alignment 0x{alignment:X} is not a power of two"));
    }
    value
        .checked_add(alignment - 1)
        .map(|rounded| rounded & !(alignment - 1))
        .ok_or_else(|| format!("Aligning 0x{value:X} to 0x{alignment:X} overflows usize"))
}

fn align_up_u32(value: u32, alignment: u32) -> Result<u32, String> {
    if !alignment.is_power_of_two() {
        return Err(format!("Alignment 0x{alignment:X} is not a power of two"));
    }
    value
        .checked_add(alignment - 1)
        .map(|rounded| rounded & !(alignment - 1))
        .ok_or_else(|| format!("Aligning 0x{value:X} to 0x{alignment:X} overflows u32"))
}

fn read_be_u32(source: &[u8], offset: usize) -> Result<u32, String> {
    let end = offset
        .checked_add(4)
        .ok_or_else(|| "DOL header offset overflows usize".to_string())?;
    let bytes = source
        .get(offset..end)
        .ok_or_else(|| format!("DOL is truncated at header offset 0x{offset:X}"))?;
    Ok(u32::from_be_bytes(bytes.try_into().map_err(|_| {
        format!("DOL header word at 0x{offset:X} is truncated")
    })?))
}

fn write_be_u32(destination: &mut [u8], offset: usize, value: u32) -> Result<(), String> {
    let end = offset
        .checked_add(4)
        .ok_or_else(|| "DOL write offset overflows usize".to_string())?;
    let output = destination
        .get_mut(offset..end)
        .ok_or_else(|| format!("DOL write at 0x{offset:X} exceeds the output buffer"))?;
    output.copy_from_slice(&value.to_be_bytes());
    Ok(())
}

/// Spawn-side hook: the single `bl TPollutionManager::isPolluted` inside
/// `TConductor::genEnemyFromPollution`, and the callee it targets.
///
/// Addresses are US retail (GMSE01). They are never trusted blindly: the call
/// site is required to hold exactly the branch this expects, so a different
/// build fails loudly instead of being corrupted.
const GOOP_LAYER_HOOK_CALL: u32 = 0x8003_475C;
const GOOP_LAYER_IS_POLLUTED: u32 = 0x8019_DA6C;
const GOOP_LAYER_GET_MANAGER_BY_NAME: u32 = 0x8003_4F20;
const GOOP_LAYER_SPAWN_INTERVAL_BRANCH: u32 = 0x8003_45B8;
const GOOP_LAYER_SPAWN_INTERVAL_BNE: u32 = 0x4082_02C8;
const GOOP_LAYER_MARKER: &[u8] = b"GRAFFITO-LAYER-SPAWN-V1\0";
/// Keep the executable table aligned with the scene format's real capacity.
/// Non-Mare stages can author all 20 retail-supported pollution layers.
const GOOP_LAYER_MAX: usize = sms_scene::GOOP_MAX_LAYERS;
const GOOP_LAYER_VANILLA: u32 = 0;
const GOOP_LAYER_DEAD: u32 = 0xFFFF_FFFF;
const MIN_GOOP_LAYER_STACK_GAP: u32 = 0x100;

/// What a goop layer does to a spawn that lands inside it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum RuntimeGoopLayerBinding {
    /// Keep whichever pool the conductor already chose.
    Vanilla,
    /// Nothing spawns in this layer.
    Empty,
    /// Take the spawn from this manager's pool instead.
    Pool(String),
}

fn ppc_stw(rs: u32, d: i32, ra: u32) -> u32 {
    36 << 26 | rs << 21 | ra << 16 | (d as u32 & 0xFFFF)
}

fn ppc_stwu(rs: u32, d: i32, ra: u32) -> u32 {
    37 << 26 | rs << 21 | ra << 16 | (d as u32 & 0xFFFF)
}

fn ppc_lwz(rt: u32, d: i32, ra: u32) -> u32 {
    32 << 26 | rt << 21 | ra << 16 | (d as u32 & 0xFFFF)
}

fn ppc_lfs(ft: u32, d: i32, ra: u32) -> u32 {
    48 << 26 | ft << 21 | ra << 16 | (d as u32 & 0xFFFF)
}

fn ppc_stfs(fs: u32, d: i32, ra: u32) -> u32 {
    52 << 26 | fs << 21 | ra << 16 | (d as u32 & 0xFFFF)
}

fn ppc_addi(rt: u32, ra: u32, imm: i32) -> u32 {
    14 << 26 | rt << 21 | ra << 16 | (imm as u32 & 0xFFFF)
}

fn ppc_li(rt: u32, imm: i32) -> u32 {
    ppc_addi(rt, 0, imm)
}

fn ppc_lis(rt: u32, imm: u32) -> u32 {
    15 << 26 | rt << 21 | (imm & 0xFFFF)
}

fn ppc_ori(ra: u32, rs: u32, imm: u32) -> u32 {
    24 << 26 | rs << 21 | ra << 16 | (imm & 0xFFFF)
}

fn ppc_cmpwi(ra: u32, imm: i32) -> u32 {
    11 << 26 | ra << 16 | (imm as u32 & 0xFFFF)
}

fn ppc_cmplwi(ra: u32, imm: u32) -> u32 {
    10 << 26 | ra << 16 | (imm & 0xFFFF)
}

fn ppc_cmpw(ra: u32, rb: u32) -> u32 {
    31 << 26 | ra << 16 | rb << 11
}

fn ppc_lwzx(rt: u32, ra: u32, rb: u32) -> u32 {
    31 << 26 | rt << 21 | ra << 16 | rb << 11 | 23 << 1
}

fn ppc_rlwinm(ra: u32, rs: u32, sh: u32, mb: u32, me: u32, rc: u32) -> u32 {
    21 << 26 | rs << 21 | ra << 16 | sh << 11 | mb << 6 | me << 1 | rc
}

fn ppc_slwi(ra: u32, rs: u32, sh: u32) -> u32 {
    ppc_rlwinm(ra, rs, sh, 0, 31 - sh, 0)
}

fn ppc_mr(ra: u32, rs: u32) -> u32 {
    31 << 26 | rs << 21 | ra << 16 | rs << 11 | 444 << 1
}

const PPC_MFLR_R0: u32 = 0x7C08_02A6;
const PPC_MTLR_R0: u32 = 0x7C08_03A6;
const PPC_MTCTR_R12: u32 = 0x7D89_03A6;
const PPC_BCTRL: u32 = 0x4E80_0421;
const PPC_BLR_WORD: u32 = 0x4E80_0020;

/// Labels the layer-routing stub branches between.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum StubLabel {
    Loop,
    NotFound,
    Found,
    Vanilla,
    Dead,
    Epilogue,
}

#[derive(Debug, Clone)]
enum StubItem {
    Word(u32),
    Label(StubLabel),
    Branch { kind: BranchKind, to: StubLabel },
    Call(u32),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum BranchKind {
    Always,
    Equal,
    NotEqual,
    GreaterOrEqual,
}

/// Assembles the stub, resolving label branches once every address is known.
fn assemble_goop_layer_stub(base: u32, items: &[StubItem]) -> Result<Vec<u32>, String> {
    let mut addresses = BTreeMap::new();
    let mut cursor = base;
    for item in items {
        match item {
            StubItem::Label(label) => {
                addresses.insert(*label, cursor);
            }
            _ => {
                cursor = cursor
                    .checked_add(4)
                    .ok_or_else(|| "layer spawn stub overflows the address space".to_string())?;
            }
        }
    }

    let mut words = Vec::new();
    let mut cursor = base;
    for item in items {
        let word = match item {
            StubItem::Label(_) => continue,
            StubItem::Word(word) => *word,
            StubItem::Call(target) => encode_branch(cursor, *target, true)?,
            StubItem::Branch { kind, to } => {
                let target = *addresses
                    .get(to)
                    .ok_or_else(|| format!("layer spawn stub branches to an unset label {to:?}"))?;
                match kind {
                    BranchKind::Always => encode_branch(cursor, target, false)?,
                    _ => {
                        let (bo, bi) = match kind {
                            BranchKind::Equal => (12_u32, 2_u32),
                            BranchKind::NotEqual => (4, 2),
                            BranchKind::GreaterOrEqual => (4, 0),
                            BranchKind::Always => unreachable!(),
                        };
                        let offset = i64::from(target) - i64::from(cursor);
                        if !(-0x8000..0x8000).contains(&offset) {
                            return Err(
                                "layer spawn stub conditional branch is out of range".to_string()
                            );
                        }
                        16 << 26 | bo << 21 | bi << 16 | (offset as u32 & 0xFFFC)
                    }
                }
            }
        };
        words.push(word);
        cursor = cursor
            .checked_add(4)
            .ok_or_else(|| "layer spawn stub overflows the address space".to_string())?;
    }
    Ok(words)
}

/// The routing stub itself.
///
/// It replaces the conductor's `isPolluted` call, so it must answer the same
/// question. On the way it learns *which* layer answered yes, and when that
/// layer names a pool it swaps the caller's pending manager for that pool's.
///
/// Register contract, read out of the US binary rather than assumed:
///   in  `r3` = `TPollutionManager*`, `f1`/`f2`/`f3` = the rolled x/y/z
///   out `r3` = the polluted flag, exactly like the call it replaces
/// `r30` (the conductor) and `r31` (the pending manager) belong to the caller
/// and stay live across this call, so writing `r31` redirects the spawn.
fn goop_layer_stub_items(table_address: u32) -> Vec<StubItem> {
    use BranchKind::*;
    use StubItem::*;
    use StubLabel::*;
    vec![
        Word(PPC_MFLR_R0),
        Word(ppc_stw(0, 4, 1)),
        Word(ppc_stwu(1, -0x30, 1)),
        Word(ppc_stw(3, 0x18, 1)),
        Word(ppc_stfs(1, 0x8, 1)),
        Word(ppc_stfs(2, 0xC, 1)),
        Word(ppc_stfs(3, 0x10, 1)),
        Word(ppc_li(0, 0)),
        Word(ppc_stw(0, 0x14, 1)),
        // for (i = 0; i < manager->layerCount; ++i)
        Label(Loop),
        Word(ppc_lwz(5, 0x18, 1)),
        Word(ppc_lwz(0, 0x10, 5)),
        Word(ppc_lwz(4, 0x14, 1)),
        Word(ppc_cmpw(4, 0)),
        Branch {
            kind: GreaterOrEqual,
            to: NotFound,
        },
        Word(ppc_lwz(6, 0x14, 5)),
        Word(ppc_slwi(0, 4, 2)),
        Word(ppc_lwzx(3, 6, 0)),
        Word(ppc_lfs(1, 0x8, 1)),
        Word(ppc_lfs(2, 0xC, 1)),
        Word(ppc_lfs(3, 0x10, 1)),
        Word(ppc_lwz(12, 0, 3)),
        // virtual TPollutionLayer::isPolluted
        Word(ppc_lwz(12, 0x50, 12)),
        Word(PPC_MTCTR_R12),
        Word(PPC_BCTRL),
        Word(ppc_rlwinm(0, 3, 0, 24, 31, 1)),
        Branch {
            kind: NotEqual,
            to: Found,
        },
        Word(ppc_lwz(4, 0x14, 1)),
        Word(ppc_addi(4, 4, 1)),
        Word(ppc_stw(4, 0x14, 1)),
        Branch {
            kind: Always,
            to: Loop,
        },
        Label(NotFound),
        Word(ppc_li(3, 0)),
        Branch {
            kind: Always,
            to: Epilogue,
        },
        Label(Found),
        Word(ppc_lwz(4, 0x14, 1)),
        Word(ppc_cmplwi(4, GOOP_LAYER_MAX as u32)),
        Branch {
            kind: GreaterOrEqual,
            to: Vanilla,
        },
        Word(ppc_lis(5, table_address >> 16)),
        Word(ppc_ori(5, 5, table_address & 0xFFFF)),
        Word(ppc_slwi(0, 4, 2)),
        Word(ppc_lwzx(4, 5, 0)),
        Word(ppc_cmpwi(4, 0)),
        Branch {
            kind: Equal,
            to: Vanilla,
        },
        Word(ppc_cmpwi(4, -1)),
        Branch {
            kind: Equal,
            to: Dead,
        },
        Word(ppc_mr(3, 30)),
        Call(GOOP_LAYER_GET_MANAGER_BY_NAME),
        Word(ppc_cmplwi(3, 0)),
        Branch {
            kind: Equal,
            to: Vanilla,
        },
        Word(ppc_mr(31, 3)),
        Label(Vanilla),
        Word(ppc_li(3, 1)),
        Branch {
            kind: Always,
            to: Epilogue,
        },
        Label(Dead),
        Word(ppc_li(3, 0)),
        Label(Epilogue),
        Word(ppc_addi(1, 1, 0x30)),
        Word(ppc_lwz(0, 4, 1)),
        Word(PPC_MTLR_R0),
        Word(PPC_BLR_WORD),
    ]
}

/// Routes goop spawns by the layer they land in.
///
/// `TConductor::genEnemyFromPollution` chooses a pool *before* it rolls a
/// position, so a stage cannot bind an enemy to a goop layer through data
/// alone. This installs a stub on the pollution test it already performs:
/// the layer that answers becomes the pool selector, which is what lets two
/// layers spawn the same enemy wearing different baked stains.
///
/// `every_frame` removes the spawn-interval gate so results are immediately
/// visible while authoring; it is not something a shipped stage wants.
pub(super) fn patch_sms_goop_layer_spawn_dol(
    source: &[u8],
    bindings: &BTreeMap<usize, RuntimeGoopLayerBinding>,
    every_frame: bool,
) -> Result<Vec<u8>, String> {
    if bindings
        .iter()
        .all(|(_, binding)| *binding == RuntimeGoopLayerBinding::Vanilla)
    {
        return Ok(source.to_vec());
    }
    if let Some((layer, _)) = bindings.iter().find(|(layer, _)| **layer >= GOOP_LAYER_MAX) {
        return Err(format!(
            "goop layer {layer} is outside the {GOOP_LAYER_MAX} layers the runtime patch routes"
        ));
    }

    let image = parse_dol(source)?;
    let hook_offset = usize::try_from(dol_file_offset(&image, GOOP_LAYER_HOOK_CALL)?)
        .map_err(|_| "goop layer hook offset does not fit usize".to_string())?;
    let expected = encode_branch(GOOP_LAYER_HOOK_CALL, GOOP_LAYER_IS_POLLUTED, true)?;
    let found = read_be_u32(source, hook_offset)?;
    if found != expected {
        return Err(format!(
            "the goop spawn hook at 0x{GOOP_LAYER_HOOK_CALL:08X} is 0x{found:08X}, not the \
             expected 0x{expected:08X}; per-layer goop spawning supports the US retail \
             executable"
        ));
    }

    let text_slot = (0..DOL_TEXT_SECTION_COUNT)
        .find(|slot| {
            !image
                .sections
                .iter()
                .any(|section| section.text && section.slot == *slot)
        })
        .ok_or_else(|| {
            "The DOL has no unused text section for per-layer goop spawning".to_string()
        })?;
    let loaded_end = image
        .sections
        .iter()
        .map(|section| section.address_end())
        .chain(image.bss.map(|(_, end)| Ok(end)))
        .collect::<Result<Vec<_>, _>>()?
        .into_iter()
        .max()
        .ok_or_else(|| "The DOL has no loaded sections".to_string())?;
    let stub_address = align_up_u32(loaded_end, FILE_ALIGNMENT)?;
    let stack_top = find_stack_top(source, &image)?;

    // The table follows the code, and the names follow the table, so the code
    // is assembled twice: once to measure it, once knowing where it points.
    let probe = assemble_goop_layer_stub(stub_address, &goop_layer_stub_items(0))?;
    let table_address =
        stub_address
            .checked_add(u32::try_from(probe.len() * 4).map_err(|_| {
                "layer spawn stub size does not fit the DOL address space".to_string()
            })?)
            .ok_or_else(|| "layer spawn stub range overflows u32".to_string())?;
    let words = assemble_goop_layer_stub(stub_address, &goop_layer_stub_items(table_address))?;
    if words.len() != probe.len() {
        return Err("layer spawn stub changed size between assembly passes".to_string());
    }
    let names_address =
        table_address
            .checked_add(u32::try_from(GOOP_LAYER_MAX * 4).map_err(|_| {
                "layer spawn table size does not fit the DOL address space".to_string()
            })?)
            .ok_or_else(|| "layer spawn table range overflows u32".to_string())?;

    let mut names = Vec::new();
    let mut table = [GOOP_LAYER_VANILLA; GOOP_LAYER_MAX];
    for (layer, binding) in bindings {
        table[*layer] = match binding {
            RuntimeGoopLayerBinding::Vanilla => GOOP_LAYER_VANILLA,
            RuntimeGoopLayerBinding::Empty => GOOP_LAYER_DEAD,
            RuntimeGoopLayerBinding::Pool(manager) => {
                let encoded = sms_formats::encode_shift_jis_name(manager).ok_or_else(|| {
                    format!("manager name {manager:?} is not encodable for the runtime")
                })?;
                let address = names_address
                    .checked_add(u32::try_from(names.len()).map_err(|_| {
                        "layer spawn name table does not fit the DOL address space".to_string()
                    })?)
                    .ok_or_else(|| "layer spawn name range overflows u32".to_string())?;
                names.extend_from_slice(&encoded);
                names.push(0);
                address
            }
        };
    }

    let mut payload = Vec::new();
    for word in &words {
        payload.extend_from_slice(&word.to_be_bytes());
    }
    for entry in table {
        payload.extend_from_slice(&entry.to_be_bytes());
    }
    payload.extend_from_slice(&names);
    while payload.len() % 4 != 0 {
        payload.push(0);
    }
    payload.extend_from_slice(GOOP_LAYER_MARKER);
    let payload_size = align_up_usize(payload.len(), FILE_ALIGNMENT as usize)?;
    payload.resize(payload_size, 0);

    let stub_end = stub_address
        .checked_add(u32::try_from(payload_size).map_err(|_| {
            "layer spawn payload size does not fit the DOL address space".to_string()
        })?)
        .ok_or_else(|| "layer spawn payload range overflows u32".to_string())?;
    let safe_end = stub_end
        .checked_add(MIN_GOOP_LAYER_STACK_GAP)
        .ok_or_else(|| "layer spawn stack guard overflows u32".to_string())?;
    if safe_end > stack_top {
        return Err(format!(
            "per-layer goop spawn payload 0x{stub_address:08X}..0x{stub_end:08X} leaves less \
             than 0x{MIN_GOOP_LAYER_STACK_GAP:X} bytes below the original stack top \
             0x{stack_top:08X}"
        ));
    }
    reject_injected_range_overlap(&image, stub_address, stub_end)?;

    let file_offset = align_up_usize(source.len(), FILE_ALIGNMENT as usize)?;
    let mut bytes = source.to_vec();
    bytes.resize(file_offset, 0);
    bytes.extend_from_slice(&payload);

    write_be_u32(
        &mut bytes,
        DOL_TEXT_FILE_OFFSETS + text_slot * 4,
        u32::try_from(file_offset)
            .map_err(|_| "layer spawn file offset does not fit u32".to_string())?,
    )?;
    write_be_u32(&mut bytes, DOL_TEXT_ADDRESSES + text_slot * 4, stub_address)?;
    write_be_u32(
        &mut bytes,
        DOL_TEXT_SIZES + text_slot * 4,
        u32::try_from(payload_size)
            .map_err(|_| "layer spawn section size does not fit u32".to_string())?,
    )?;
    write_be_u32(
        &mut bytes,
        hook_offset,
        encode_branch(GOOP_LAYER_HOOK_CALL, stub_address, true)?,
    )?;

    if every_frame {
        let interval_offset =
            usize::try_from(dol_file_offset(&image, GOOP_LAYER_SPAWN_INTERVAL_BRANCH)?)
                .map_err(|_| "goop spawn interval offset does not fit usize".to_string())?;
        let found = read_be_u32(source, interval_offset)?;
        if found != GOOP_LAYER_SPAWN_INTERVAL_BNE {
            return Err(format!(
                "the goop spawn interval gate at 0x{GOOP_LAYER_SPAWN_INTERVAL_BRANCH:08X} is \
                 0x{found:08X}, not the expected 0x{GOOP_LAYER_SPAWN_INTERVAL_BNE:08X}"
            ));
        }
        write_be_u32(&mut bytes, interval_offset, PPC_NOP)?;
    }

    Ok(bytes)
}

/// File offset of a loaded address.
fn dol_file_offset(image: &DolImage, address: u32) -> Result<u32, String> {
    for section in &image.sections {
        if address >= section.address && address < section.address_end()? {
            return section
                .file_offset
                .checked_add(address - section.address)
                .ok_or_else(|| format!("DOL offset for 0x{address:08X} overflows"));
        }
    }
    Err(format!(
        "address 0x{address:08X} is not inside any loaded DOL section"
    ))
}

#[cfg(test)]
mod goop_layer_spawn_tests {
    use super::*;

    /// Local diagnostic against the real US executable:
    /// `SMS_BASE_ROOT=<extracted game> cargo test goop_layer_spawn_patches_the_local_retail_dol
    /// -- --ignored --nocapture`
    #[test]
    #[ignore = "requires SMS_BASE_ROOT with an extracted US retail game"]
    fn goop_layer_spawn_patches_the_local_retail_dol() {
        let path =
            std::path::PathBuf::from(std::env::var_os("SMS_BASE_ROOT").expect("SMS_BASE_ROOT"))
                .join("sys/main.dol");
        let source = std::fs::read(path).expect("read main.dol");
        let bindings = BTreeMap::from([
            (0, RuntimeGoopLayerBinding::Pool("poolA".to_string())),
            (1, RuntimeGoopLayerBinding::Empty),
        ]);
        let patched = patch_sms_goop_layer_spawn_dol(&source, &bindings, false).expect("patch");
        assert!(patched.len() > source.len(), "payload was appended");

        // The hook now calls the stub rather than isPolluted.
        let image = parse_dol(&patched).expect("parse patched");
        let offset =
            usize::try_from(dol_file_offset(&image, GOOP_LAYER_HOOK_CALL).unwrap()).unwrap();
        let word = read_be_u32(&patched, offset).unwrap();
        let vanilla = encode_branch(GOOP_LAYER_HOOK_CALL, GOOP_LAYER_IS_POLLUTED, true).unwrap();
        assert_ne!(word, vanilla, "the hook was redirected");
        assert!(
            patched
                .windows(GOOP_LAYER_MARKER.len())
                .any(|window| window == GOOP_LAYER_MARKER),
            "payload carries its marker"
        );

        // Patching an already-patched executable must refuse rather than
        // stack a second stub on a redirected call site.
        assert!(patch_sms_goop_layer_spawn_dol(&patched, &bindings, false).is_err());
    }

    /// With nothing bound the executable must come back byte-identical, so a
    /// stage that uses no per-layer pools ships a stock DOL.
    #[test]
    fn an_unbound_stage_leaves_the_executable_untouched() {
        let source = vec![0_u8; 0x120];
        let bindings = BTreeMap::new();
        let patched = patch_sms_goop_layer_spawn_dol(&source, &bindings, false).unwrap();
        assert_eq!(patched, source);

        let vanilla_only = BTreeMap::from([(0, RuntimeGoopLayerBinding::Vanilla)]);
        let patched = patch_sms_goop_layer_spawn_dol(&source, &vanilla_only, false).unwrap();
        assert_eq!(patched, source);
    }

    /// A layer index the stub's table cannot address is refused rather than
    /// silently dropped.
    #[test]
    fn a_layer_beyond_the_runtime_table_is_rejected() {
        let source = vec![0_u8; 0x120];
        let bindings = BTreeMap::from([(
            GOOP_LAYER_MAX,
            RuntimeGoopLayerBinding::Pool("pool".to_string()),
        )]);
        assert!(patch_sms_goop_layer_spawn_dol(&source, &bindings, false).is_err());
    }

    #[test]
    fn the_runtime_table_covers_every_authorable_goop_layer() {
        assert_eq!(GOOP_LAYER_MAX, sms_scene::GOOP_MAX_LAYERS);
        assert_eq!(GOOP_LAYER_MAX, 20);
    }

    /// Every branch in the stub has to land inside it.
    #[test]
    fn the_stub_branches_stay_inside_itself() {
        let base = 0x8041_7800;
        let words = assemble_goop_layer_stub(base, &goop_layer_stub_items(base + 0xDC)).unwrap();
        let end = base + (words.len() as u32) * 4;
        for (index, word) in words.iter().enumerate() {
            let address = base + (index as u32) * 4;
            let opcode = word >> 26;
            let target = if opcode == 18 {
                let mut offset = (word & 0x03FF_FFFC) as i32;
                if offset & 0x0200_0000 != 0 {
                    offset -= 0x0400_0000;
                }
                if word & 1 != 0 {
                    continue; // the call out to getManagerByName
                }
                address.wrapping_add(offset as u32)
            } else if opcode == 16 {
                let mut offset = (word & 0xFFFC) as i32;
                if offset & 0x8000 != 0 {
                    offset -= 0x1_0000;
                }
                address.wrapping_add(offset as u32)
            } else {
                continue;
            };
            assert!(
                (base..=end).contains(&target),
                "branch at 0x{address:08X} leaves the stub: 0x{target:08X}"
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::path::{Path, PathBuf};

    use super::*;

    const SYNTHETIC_ENTRY_OFFSET: usize = DOL_HEADER_SIZE;
    const SYNTHETIC_TEXT_OFFSET: usize = 0x120;
    const SYNTHETIC_TEXT_WORDS: usize = 0x200;

    #[derive(Clone, Copy)]
    struct SyntheticLayout {
        text_address: u32,
        hook_word: usize,
        movie_word: usize,
        setter_word: usize,
    }

    fn synthetic_dol(layout: SyntheticLayout) -> Vec<u8> {
        let mut words = vec![PPC_NOP; SYNTHETIC_TEXT_WORDS];
        let address = |word: usize| layout.text_address + u32::try_from(word * 4).unwrap();

        // Four zero-filled linker alignment gaps, each immediately after a
        // return and ending on the next 0x20-byte function boundary.
        install_alignment_cave(&mut words, 0x188, TRANSITION_CAVE_WORDS);
        install_alignment_cave(&mut words, 0x198, MOVIE_PRIMARY_CAVE_WORDS);
        install_alignment_cave(&mut words, 0x1a8, MOVIE_SECONDARY_CAVE_WORDS);
        install_alignment_cave(&mut words, 0x1b8, DEATH_RESTART_CAVE_WORDS);
        // The NLogo proc case constructs and sets up TGCLogoDir. Direct boot
        // bypasses the case body so the display remains black while the
        // required asynchronous game-data setup continues.
        let setup_word = 0x20;
        let setup_resume_word = 0x40;
        words[setup_word] = encode_d_form(32, 3, 31, 0x1c);
        words[setup_word + 1] =
            encode_branch(address(setup_word + 1), address(0x1c0), true).unwrap();
        words[setup_word + 2] = encode_li(3, 0x48);
        words[setup_word + 3] =
            encode_branch(address(setup_word + 3), address(0x1c1), true).unwrap();
        words[setup_word + 4] = encode_mr(28, 3) | 1;
        words[setup_word + 5] = 0x4182_000c; // beq +0xC
        words[setup_word + 6] = encode_mr(3, 28);
        words[setup_word + 7] =
            encode_branch(address(setup_word + 7), address(0x1c2), true).unwrap();
        words[setup_word + 8] = encode_d_form(36, 28, 31, 4);
        words[setup_word + 9] = encode_mr(3, 28);
        words[setup_word + 10] = encode_d_form(32, 4, 31, 0x1c);
        words[setup_word + 11] = encode_d_form(32, 5, 31, 0x20);
        words[setup_word + 12] =
            encode_branch(address(setup_word + 12), address(0x1c3), true).unwrap();
        words[setup_word + 13] =
            encode_branch(address(setup_word + 13), address(setup_resume_word), false).unwrap();

        // The NLogo game-loop path normally calls TGCLogoDir::direct until it
        // returns DONE. The bypass marks that visual half complete immediately
        // but retains the separate setup-thread completion check.
        let director_word = layout.hook_word - 0x20;
        let director_skip_word = director_word + 14;
        words[director_word] = encode_d_form(32, 0, 13, -0x7000);
        words[director_word + 1] = encode_li(29, 0);
        words[director_word + 2] = 0x5400_07ff; // clrlwi. r0, r0, 31
        words[director_word + 3] =
            encode_bne(address(director_word + 3), address(director_skip_word)).unwrap();
        words[director_word + 4] = encode_d_form(32, 3, 31, 4);
        words[director_word + 5] = encode_d_form(32, 12, 3, 0);
        words[director_word + 6] = encode_d_form(32, 12, 12, 0x64);
        words[director_word + 7] = 0x7d88_03a6; // mtctr r12
        words[director_word + 8] = 0x4e80_0021; // bctrl
        words[director_word + 9] = 0x2c03_0004; // cmpwi r3, DONE
        words[director_word + 10] =
            encode_bne(address(director_word + 10), address(director_skip_word)).unwrap();
        words[director_word + 11] = encode_d_form(32, 0, 13, -0x7000);
        words[director_word + 12] = 0x6000_0001; // ori r0, r0, 1
        words[director_word + 13] = encode_d_form(36, 0, 13, -0x7000);

        // Derive the application register from the nearby app-state 2/3
        // comparisons, independent of its absolute address.
        let state_word = layout.hook_word - 8;
        words[state_word] = encode_d_form(34, 0, 31, 8);
        words[state_word + 1] = encode_cmplwi(0, 2);
        words[state_word + 2] = encode_cmplwi(0, 3);

        // Semantic post-NLogo tail: sGameInit == 3 ? nextState = DONE : ...
        let transition_target = address(layout.hook_word + 5);
        words[layout.hook_word] = encode_d_form(32, 0, 13, -0x7000);
        words[layout.hook_word + 1] = 0x2c00_0003; // cmpwi r0, 3
        words[layout.hook_word + 2] =
            encode_bne(address(layout.hook_word + 2), transition_target).unwrap();
        words[layout.hook_word + 3] = encode_li(29, 4);
        words[layout.hook_word + 4] =
            encode_branch(address(layout.hook_word + 4), transition_target, false).unwrap();

        // Semantic gameplay movie call, followed later by the retail
        // mNextArea.set(15, 0, 0) case. The latter supplies field offsets.
        let original_movie_word = 0x1d0;
        words[layout.movie_word] = encode_mr(3, 31);
        words[layout.movie_word + 1] = encode_branch(
            address(layout.movie_word + 1),
            address(original_movie_word),
            true,
        )
        .unwrap();
        words[layout.movie_word + 2] = encode_clrlwi_dot_r0_r3_24();
        words[layout.movie_word + 3] = 0x4182_0008; // beq +8
        words[original_movie_word] = PPC_BLR;

        words[layout.setter_word] = encode_li(4, 15);
        words[layout.setter_word + 1] = encode_li(0, 0);
        words[layout.setter_word + 2] = encode_d_form(38, 4, 31, 0x12);
        words[layout.setter_word + 3] = encode_d_form(38, 0, 31, 0x13);
        words[layout.setter_word + 4] = encode_d_form(44, 0, 31, 0x14);

        // Ordinary-stage miss handling has a special-stage branch that copies
        // the current area directly. The other branch normally calls
        // decideNextStage and returns to Delfino Plaza. The direct-boot patch
        // replaces only that ordinary-stage call and preserves the surrounding
        // life/fade state machine.
        let death_word = layout.movie_word - 0x20;
        let special_word = death_word - 8;
        words[special_word] = encode_d_form(34, 0, 27, 0);
        words[special_word + 1] = encode_d_form(38, 0, 3, 0x12);
        words[special_word + 2] = encode_li(0, 0);
        words[special_word + 3] = encode_d_form(38, 0, 3, 0x13);
        words[special_word + 4] = encode_d_form(44, 0, 3, 0x14);
        words[special_word + 5] =
            encode_branch(address(special_word + 5), address(death_word + 1), false).unwrap();
        words[death_word] = encode_branch(address(death_word), address(0x1c8), true).unwrap();
        words[death_word + 1] = encode_d_form(40, 0, 28, 0x4c);
        words[death_word + 2] = encode_d_form(14, 3, 28, 0);
        words[death_word + 3] = 0x5400_04e0;
        words[death_word + 4] = encode_d_form(44, 0, 28, 0x4c);
        words[death_word + 5] =
            encode_branch(address(death_word + 5), address(0x1c9), true).unwrap();
        words[death_word + 6] = encode_li(0, 15);
        words[0x1c8] = PPC_BLR;
        words[0x1c9] = PPC_BLR;

        let text_size = u32::try_from(words.len() * 4).unwrap();
        let mut bytes = vec![0_u8; SYNTHETIC_TEXT_OFFSET + text_size as usize];
        write_be_u32(
            &mut bytes,
            DOL_TEXT_FILE_OFFSETS,
            SYNTHETIC_ENTRY_OFFSET as u32,
        )
        .unwrap();
        write_be_u32(&mut bytes, DOL_TEXT_ADDRESSES, 0x8000_3100).unwrap();
        write_be_u32(&mut bytes, DOL_TEXT_SIZES, 4).unwrap();
        write_be_u32(
            &mut bytes,
            DOL_TEXT_FILE_OFFSETS + 4,
            SYNTHETIC_TEXT_OFFSET as u32,
        )
        .unwrap();
        write_be_u32(&mut bytes, DOL_TEXT_ADDRESSES + 4, layout.text_address).unwrap();
        write_be_u32(&mut bytes, DOL_TEXT_SIZES + 4, text_size).unwrap();
        write_be_u32(&mut bytes, DOL_BSS_ADDRESS, 0x8030_0000).unwrap();
        write_be_u32(&mut bytes, DOL_BSS_SIZE, 0x1000).unwrap();
        write_be_u32(&mut bytes, DOL_ENTRY_POINT, 0x8000_3100).unwrap();
        write_be_u32(&mut bytes, SYNTHETIC_ENTRY_OFFSET, PPC_BLR).unwrap();
        for (index, word) in words.into_iter().enumerate() {
            write_be_u32(&mut bytes, SYNTHETIC_TEXT_OFFSET + index * 4, word).unwrap();
        }
        bytes
    }

    fn install_alignment_cave(words: &mut [u32], end_word: usize, word_count: usize) {
        let start = end_word - word_count;
        words[start - 1] = PPC_BLR;
        words[start..end_word].fill(0);
    }

    fn encode_mr(target: u8, source: u8) -> u32 {
        (31_u32 << 26)
            | (u32::from(source) << 21)
            | (u32::from(target) << 16)
            | (u32::from(source) << 11)
            | (444_u32 << 1)
    }

    fn encode_clrlwi_dot_r0_r3_24() -> u32 {
        (21_u32 << 26) | (3_u32 << 21) | (24_u32 << 6) | (31_u32 << 1) | 1
    }

    #[test]
    fn retail_nlogo_director_sequence_matches_semantic_helpers() {
        let sequence = [
            0x800d_9800,
            0x3ba0_0000,
            0x5400_07ff,
            0x4082_002c,
            0x807f_0004,
            0x8183_0000,
            0x818c_0064,
            0x7d88_03a6,
            0x4e80_0021,
            0x2c03_0004,
            0x4082_0010,
            0x800d_9800,
            0x6000_0001,
            0x900d_9800,
        ];
        assert_eq!(decode_lwz_from_r13(sequence[0]), Some(0));
        assert!(is_li(sequence[1], 0));
        assert!(is_low_bit_test(sequence[2], 0));
        assert_eq!(decode_d_form(sequence[4], 32), Some((3, 31, 4)));
        assert_eq!(decode_d_form(sequence[5], 32), Some((12, 3, 0)));
        assert_eq!(decode_d_form(sequence[6], 32), Some((12, 12, 0x64)));
        assert!(is_mtctr(sequence[7], 12));
        assert!(is_cmpwi(sequence[9], 3, 4));
        assert_eq!(decode_lwz_from_r13(sequence[11]), Some(0));
        assert!(is_ori(sequence[12], 0, 0, 1));
        assert_eq!(decode_d_form(sequence[13], 36), Some((0, 13, -0x6800)));
    }

    #[test]
    fn goop_manager_patches_remove_only_the_decomp_proven_runtime_gates() {
        let layout = SyntheticLayout {
            text_address: 0x8000_4000,
            hook_word: 0x80,
            movie_word: 0xc0,
            setter_word: 0xe0,
        };
        let mut source = synthetic_dol(layout);
        let poi_word = 0x100;
        let poi_sequence = [
            0x8803_000e,
            0x2800_0038,
            0x4082_002c,
            0x3860_01cc,
            0x4800_0001,
            0x7c7f_1b79,
            0x4182_0014,
        ];
        for (index, word) in poi_sequence.into_iter().enumerate() {
            write_be_u32(
                &mut source,
                SYNTHETIC_TEXT_OFFSET + (poi_word + index) * 4,
                word,
            )
            .unwrap();
        }
        let hino_word = 0x120;
        let hino_sequence = [
            0x2803_0000,
            0x4082_0058,
            0x8081_0068,
            0x387e_0114,
            0x83a4_0008,
            0x4800_0001,
            0x819d_0000,
            0x3883_0000,
            0x387d_0000,
            0x818c_001c,
            0x38be_0114,
            0x7d88_03a6,
            0x4e80_0021,
            0x2803_0000,
            0x4082_0024,
        ];
        for (index, word) in hino_sequence.into_iter().enumerate() {
            write_be_u32(
                &mut source,
                SYNTHETIC_TEXT_OFFSET + (hino_word + index) * 4,
                word,
            )
            .unwrap();
        }
        let mame_word = 0x150;
        let mame_sequence = [
            0x807d_01d4,
            0x801d_01d8,
            0x907d_0010,
            0x901d_0014,
            0x801d_01dc,
            0x901d_0018,
            0xc03d_0014,
            0xc002_92cc,
            0xec01_002a,
            0xd01d_0014,
        ];
        for (index, word) in mame_sequence.into_iter().enumerate() {
            write_be_u32(
                &mut source,
                SYNTHETIC_TEXT_OFFSET + (mame_word + index) * 4,
                word,
            )
            .unwrap();
        }

        let requested = BTreeSet::from([
            RuntimeGoopManagerPatch::PoiHanaAnyArea,
            RuntimeGoopManagerPatch::HinoKuri2Pool,
            RuntimeGoopManagerPatch::MameGessoRelocatableReset,
        ]);
        let patched = patch_sms_goop_enemy_managers_dol(&source, &requested).unwrap();
        assert_eq!(
            read_be_u32(&patched, SYNTHETIC_TEXT_OFFSET + (poi_word + 2) * 4).unwrap(),
            PPC_NOP
        );
        assert_eq!(
            read_be_u32(&patched, SYNTHETIC_TEXT_OFFSET + (hino_word + 1) * 4).unwrap(),
            PPC_NOP
        );
        for relative_word in [2usize, 3, 5] {
            assert_eq!(
                read_be_u32(
                    &patched,
                    SYNTHETIC_TEXT_OFFSET + (mame_word + relative_word) * 4,
                )
                .unwrap(),
                PPC_NOP
            );
        }
        assert_eq!(
            patch_sms_goop_enemy_managers_dol(&patched, &requested).unwrap(),
            patched
        );
    }

    #[test]
    fn telesa_pollution_pool_reset_clears_hidden_and_starts_fade_in() {
        let layout = SyntheticLayout {
            text_address: 0x8000_4000,
            hook_word: 0x80,
            movie_word: 0xc0,
            setter_word: 0xe0,
        };
        let mut source = synthetic_dol(layout);
        write_be_u32(&mut source, DOL_TEXT_SIZES, 0x20).unwrap();
        write_be_u32(
            &mut source,
            SYNTHETIC_ENTRY_OFFSET,
            encode_branch(0x8000_3100, 0x8000_3108, true).unwrap(),
        )
        .unwrap();
        write_be_u32(
            &mut source,
            SYNTHETIC_ENTRY_OFFSET + 8,
            encode_d_form(15, 1, 0, 0x8040_u16 as i16),
        )
        .unwrap();
        write_be_u32(
            &mut source,
            SYNTHETIC_ENTRY_OFFSET + 12,
            encode_d_form(14, 1, 1, 0),
        )
        .unwrap();
        write_be_u32(&mut source, SYNTHETIC_ENTRY_OFFSET + 16, PPC_BLR).unwrap();
        let telesa_word = 0x100;
        let telesa_prologue = [
            0x7c08_02a6,
            0x9001_0004,
            0x9421_ff90,
            0xdbe1_0068,
            0x93e1_0064,
            0x7c7f_1b78,
            0x93c1_0060,
            0x93a1_005c,
            0x9381_0058,
            0x4800_0001,
        ];
        for (index, word) in telesa_prologue.into_iter().enumerate() {
            write_be_u32(
                &mut source,
                SYNTHETIC_TEXT_OFFSET + (telesa_word - 10 + index) * 4,
                word,
            )
            .unwrap();
        }
        let telesa_sequence = [
            0x801f_00f0,
            0x6000_0008,
            0x901f_00f0,
            0x801f_00f0,
            0x6000_0002,
            0x901f_00f0,
            0x807f_0194,
            0x3bc3_0458,
            0xc023_045c,
            0xc003_0458,
            0xefe1_0028,
        ];
        for (index, word) in telesa_sequence.into_iter().enumerate() {
            write_be_u32(
                &mut source,
                SYNTHETIC_TEXT_OFFSET + (telesa_word + index) * 4,
                word,
            )
            .unwrap();
        }

        let patched = patch_sms_goop_enemy_managers_dol(
            &source,
            &BTreeSet::from([RuntimeGoopManagerPatch::TelesaVisibleReset]),
        )
        .unwrap();
        let hook_word = telesa_word + 4;
        let hook_address = layout.text_address + u32::try_from(hook_word * 4).unwrap();
        let hook = read_be_u32(&patched, SYNTHETIC_TEXT_OFFSET + hook_word * 4).unwrap();
        assert!(is_relative_bl(hook));
        let stub_address = decode_branch_target(hook, hook_address).unwrap();
        let image = parse_dol(&patched).unwrap();
        let stub_anchor = address_to_word_anchor(&image, stub_address)
            .unwrap()
            .expect("Telesa stub must be in an injected text section");
        let stub_words = section_words(&patched, stub_anchor.section).unwrap();
        assert_eq!(
            &stub_words[stub_anchor.word_index..stub_anchor.word_index + 7],
            &[
                0x5400_07fa,
                encode_d_form(14, 12, 0, 0),
                encode_d_form(36, 12, 31, 0x1c0),
                encode_d_form(36, 12, 31, 0x1d8),
                encode_d_form(14, 12, 0, 1),
                encode_d_form(36, 12, 31, 0x1dc),
                PPC_BLR,
            ]
        );
        assert_eq!(
            read_be_u32(&patched, SYNTHETIC_TEXT_OFFSET + (telesa_word + 5) * 4).unwrap(),
            0x901f_00f0
        );
    }

    #[test]
    #[ignore = "requires SMS_BASE_ROOT with an extracted JP or US retail game"]
    fn local_retail_goop_manager_patches_are_semantically_located() {
        let path = PathBuf::from(std::env::var_os("SMS_BASE_ROOT").expect("SMS_BASE_ROOT"))
            .join("sys/main.dol");
        let source = fs::read(path).unwrap();
        let requested = BTreeSet::from([
            RuntimeGoopManagerPatch::PoiHanaAnyArea,
            RuntimeGoopManagerPatch::HinoKuri2Pool,
            RuntimeGoopManagerPatch::MameGessoRelocatableReset,
            RuntimeGoopManagerPatch::PoiHanaRedPool,
            RuntimeGoopManagerPatch::TelesaVisibleReset,
        ]);
        let patched = patch_sms_goop_enemy_managers_dol(&source, &requested).unwrap();
        assert_ne!(patched, source);
        parse_dol(&patched).unwrap();
    }

    #[test]
    fn semantic_patch_injects_target_and_one_shot_movie_bypass() {
        let layout = SyntheticLayout {
            text_address: 0x8000_1000,
            hook_word: 0x80,
            movie_word: 0x120,
            setter_word: 0x160,
        };
        let source = synthetic_dol(layout);
        let target = RuntimeStageTarget {
            area_index: 7,
            scenario_index: 4,
            archive_name: "customModStage.arc".to_string(),
        };

        let patched = patch_sms_direct_boot_dol(&source, &target).unwrap();
        let setup_address = layout.text_address + 0x20 * 4;
        assert_eq!(patched.logo_bypass_address, setup_address);
        assert_eq!(
            decode_branch_target(
                read_be_u32(&patched.bytes, SYNTHETIC_TEXT_OFFSET + 0x20 * 4).unwrap(),
                setup_address,
            )
            .unwrap(),
            layout.text_address + 0x40 * 4
        );
        let director_branch_word = layout.hook_word - 0x20 + 3;
        let director_branch_address =
            layout.text_address + u32::try_from(director_branch_word * 4).unwrap();
        assert_eq!(
            decode_branch_target(
                read_be_u32(
                    &patched.bytes,
                    SYNTHETIC_TEXT_OFFSET + director_branch_word * 4,
                )
                .unwrap(),
                director_branch_address,
            )
            .unwrap(),
            layout.text_address + u32::try_from((layout.hook_word - 0x20 + 11) * 4).unwrap()
        );

        assert_eq!(
            patched.hook_address,
            layout.text_address + u32::try_from((layout.hook_word + 4) * 4).unwrap()
        );
        assert_eq!(
            patched.movie_hook_address,
            layout.text_address + u32::try_from((layout.movie_word + 1) * 4).unwrap()
        );
        let death_word = layout.movie_word - 0x20;
        assert_eq!(
            patched.death_hook_address,
            layout.text_address + u32::try_from(death_word * 4).unwrap()
        );
        assert_eq!(
            decode_branch_target(
                read_be_u32(
                    &patched.bytes,
                    SYNTHETIC_TEXT_OFFSET + (layout.hook_word + 4) * 4,
                )
                .unwrap(),
                patched.hook_address,
            )
            .unwrap(),
            patched.stub_address
        );
        assert_eq!(
            read_be_u32(
                &patched.bytes,
                SYNTHETIC_TEXT_OFFSET + (layout.hook_word + 3) * 4,
            )
            .unwrap(),
            encode_li(29, 7)
        );
        let wrapper_address = decode_branch_target(
            read_be_u32(
                &patched.bytes,
                SYNTHETIC_TEXT_OFFSET + (layout.movie_word + 1) * 4,
            )
            .unwrap(),
            patched.movie_hook_address,
        )
        .unwrap();
        assert_ne!(wrapper_address, patched.stub_address);
        assert!(address_is_in_text(
            &parse_dol(&patched.bytes).unwrap().sections,
            wrapper_address,
            4
        )
        .unwrap());

        let patched_image = parse_dol(&patched.bytes).unwrap();
        let cave_section = patched_image
            .sections
            .iter()
            .find(|section| {
                section.text
                    && patched.stub_address >= section.address
                    && patched.stub_address < section.address + section.size
            })
            .unwrap();
        let payload_offset =
            usize::try_from(cave_section.file_offset + patched.stub_address - cave_section.address)
                .unwrap();
        assert_eq!(
            read_be_u32(&patched.bytes, payload_offset).unwrap(),
            encode_d_form(38, 29, 31, 0x12)
        );
        assert_eq!(
            read_be_u32(&patched.bytes, payload_offset + 4).unwrap(),
            encode_li(29, 4)
        );
        assert_eq!(
            read_be_u32(&patched.bytes, payload_offset + 3 * 4).unwrap(),
            encode_li(0, DIRECT_BOOT_FLAG as i16)
        );
        assert_eq!(
            read_be_u32(&patched.bytes, payload_offset + 5 * 4).unwrap(),
            encode_li(29, POST_NLOGO_STATE)
        );
        let transition_target = decode_branch_target(
            read_be_u32(&patched.bytes, payload_offset + 6 * 4).unwrap(),
            patched.stub_address + 6 * 4,
        )
        .unwrap();
        assert_eq!(
            transition_target,
            layout.text_address + u32::try_from((layout.hook_word + 5) * 4).unwrap()
        );
        assert_eq!(
            decode_branch_target(
                read_be_u32(&patched.bytes, SYNTHETIC_TEXT_OFFSET + death_word * 4,).unwrap(),
                patched.death_hook_address,
            )
            .unwrap(),
            patched.death_stub_address
        );
        let death_stub = address_to_word_anchor(&patched_image, patched.death_stub_address)
            .unwrap()
            .unwrap();
        assert_eq!(
            &section_words(&patched.bytes, death_stub.section).unwrap()
                [death_stub.word_index..death_stub.word_index + DEATH_RESTART_CAVE_WORDS],
            &[
                encode_li(0, 7),
                encode_d_form(38, 0, 27, 4),
                encode_li(0, 4),
                encode_d_form(38, 0, 27, 5),
                encode_li(0, 0),
                encode_d_form(44, 0, 27, 6),
                PPC_BLR,
            ]
        );
        assert_eq!(patched.bytes.len(), source.len());
    }

    #[test]
    fn semantic_patch_tolerates_relocated_mod_like_layout() {
        let layout = SyntheticLayout {
            text_address: 0x8010_4000,
            hook_word: 0xa0,
            movie_word: 0x138,
            setter_word: 0x178,
        };
        let patched = patch_sms_direct_boot_dol(
            &synthetic_dol(layout),
            &RuntimeStageTarget {
                area_index: 42,
                scenario_index: 9,
                archive_name: "modded/entirelyCustomName.szs".to_string(),
            },
        )
        .unwrap();

        assert_eq!(
            patched.hook_address,
            layout.text_address + u32::try_from((layout.hook_word + 4) * 4).unwrap()
        );
        assert_eq!(patched.bytes.len(), synthetic_dol(layout).len());
    }

    #[test]
    fn sound_stage_hook_follows_an_existing_stage_music_dispatcher() {
        let layout = SyntheticLayout {
            text_address: 0x8000_1000,
            hook_word: 0x80,
            movie_word: 0x120,
            setter_word: 0x160,
        };
        let mut source = synthetic_dol(layout);
        let sound_hook_word = layout.hook_word + 7;
        let dispatch_word = layout.hook_word + 9;
        let dispatcher_word = 0x1b0;
        let init_sound_word = 0x1e0;
        let enter_stage_word = 0x1e1;
        let address = |word: usize| layout.text_address + u32::try_from(word * 4).unwrap();
        for (word, value) in [
            (sound_hook_word, encode_d_form(32, 3, 13, -0x6000)),
            (
                sound_hook_word + 1,
                encode_branch(address(sound_hook_word + 1), address(init_sound_word), true)
                    .unwrap(),
            ),
            (sound_hook_word + 2, encode_d_form(32, 4, 13, -0x5ff8)),
            (sound_hook_word + 3, encode_cmpwi(4, -1)),
            (sound_hook_word + 4, 0x4182_0014),
            (sound_hook_word + 5, encode_d_form(32, 3, 13, -0x6000)),
            (sound_hook_word + 6, encode_d_form(14, 5, 28, 0)),
            (sound_hook_word + 7, encode_d_form(14, 6, 29, 0)),
            (
                sound_hook_word + 8,
                encode_branch(
                    address(sound_hook_word + 8),
                    address(enter_stage_word),
                    true,
                )
                .unwrap(),
            ),
            (sound_hook_word + 9, PPC_BLR),
            (init_sound_word, PPC_BLR),
            (enter_stage_word, PPC_BLR),
        ] {
            write_be_u32(&mut source, SYNTHETIC_TEXT_OFFSET + word * 4, value).unwrap();
        }
        write_be_u32(
            &mut source,
            SYNTHETIC_TEXT_OFFSET + dispatch_word * 4,
            encode_branch(address(dispatch_word), address(dispatcher_word), false).unwrap(),
        )
        .unwrap();
        write_be_u32(
            &mut source,
            SYNTHETIC_TEXT_OFFSET + dispatcher_word * 4,
            encode_d_form(32, 4, 13, -0x5ff8),
        )
        .unwrap();
        write_be_u32(
            &mut source,
            SYNTHETIC_TEXT_OFFSET + (dispatcher_word + 1) * 4,
            encode_branch(
                address(dispatcher_word + 1),
                address(dispatch_word + 1),
                false,
            )
            .unwrap(),
        )
        .unwrap();

        let image = parse_dol(&source).unwrap();
        let hook = find_sound_stage_hook(&source, &image).unwrap();
        assert_eq!(hook.ms_stg_offset, -0x5ff8);
    }

    fn stage_music_test_hook() -> SoundStageHook {
        let section = DolSection {
            text: true,
            slot: 0,
            file_offset: 0,
            address: 0x8000_0000,
            size: 0x4000,
        };
        SoundStageHook {
            dispatch_anchor: WordAnchor {
                section,
                word_index: 0x100,
            },
            enter_stage_anchor: WordAnchor {
                section,
                word_index: 0x110,
            },
            enter_stage_target: 0x8000_0800,
            area_register: 28,
            scenario_register: 29,
            ms_stg_offset: -0x5ff8,
        }
    }

    #[derive(Debug)]
    struct StageMusicInterpreter {
        registers: [u32; 32],
        fields: [u8; 32],
        equal: bool,
        writes: Vec<(usize, usize)>,
    }

    impl StageMusicInterpreter {
        fn new(area: u8, scenario: u8) -> Self {
            let mut registers = [0_u32; 32];
            registers[28] = u32::from(area);
            registers[29] = u32::from(scenario);
            Self {
                registers,
                fields: [0; 32],
                equal: false,
                writes: Vec::new(),
            }
        }

        fn set_word(&mut self, offset: usize, value: u32) {
            self.fields[offset..offset + 4].copy_from_slice(&value.to_be_bytes());
        }

        fn word(&self, offset: usize) -> u32 {
            u32::from_be_bytes(self.fields[offset..offset + 4].try_into().unwrap())
        }

        fn run(
            &mut self,
            stub_address: u32,
            words: &[u32],
            hook: SoundStageHook,
        ) -> Result<(), String> {
            let end_address = stub_address + u32::try_from(words.len() * 4).unwrap();
            let mut pc = stub_address;
            for _ in 0..2_000 {
                if pc < stub_address || pc >= end_address {
                    return Ok(());
                }
                let index = usize::try_from((pc - stub_address) / 4).unwrap();
                let word = words[index];
                let mut next = pc + 4;
                match opcode(word) {
                    11 => {
                        self.equal = self.registers[usize::from(register_a(word))]
                            == immediate_i16(word) as i32 as u32;
                    }
                    14 => {
                        let source = register_a(word);
                        let base = if source == 0 {
                            0
                        } else {
                            self.registers[usize::from(source)]
                        };
                        self.registers[usize::from(register_t(word))] =
                            base.wrapping_add_signed(i32::from(immediate_i16(word)));
                    }
                    15 => {
                        let source = register_a(word);
                        let base = if source == 0 {
                            0
                        } else {
                            self.registers[usize::from(source)]
                        };
                        self.registers[usize::from(register_t(word))] = base
                            .wrapping_add_signed(i32::from(immediate_i16(word)).wrapping_shl(16));
                    }
                    16 => {
                        let take = (is_beq(word) && self.equal) || (is_bne(word) && !self.equal);
                        if take {
                            next = decode_conditional_branch_target(word, pc)?;
                        }
                    }
                    18 => {
                        next = decode_branch_target(word, pc)?;
                    }
                    24 => {
                        self.registers[usize::from(register_a(word))] = self.registers
                            [usize::from(register_t(word))]
                            | u32::from(immediate_u16(word));
                    }
                    31 if ((word >> 1) & 0x3ff) == 32 => {
                        let right = ((word >> 11) & 0x1f) as usize;
                        self.equal =
                            self.registers[usize::from(register_a(word))] == self.registers[right];
                    }
                    32 | 36 | 38 => {
                        if register_a(word) != 13 {
                            return Err(format!(
                                "test interpreter encountered unsupported D-form base in 0x{word:08X}"
                            ));
                        }
                        let relative =
                            i32::from(immediate_i16(word)) - i32::from(hook.ms_stg_offset);
                        let offset = usize::try_from(relative).map_err(|_| {
                            format!("test interpreter field offset {relative} is negative")
                        })?;
                        match opcode(word) {
                            32 => {
                                self.registers[usize::from(register_t(word))] = self.word(offset);
                            }
                            36 => {
                                let value = self.registers[usize::from(register_t(word))];
                                self.set_word(offset, value);
                                self.writes.push((offset, 4));
                            }
                            38 => {
                                self.fields[offset] =
                                    self.registers[usize::from(register_t(word))] as u8;
                                self.writes.push((offset, 1));
                            }
                            _ => unreachable!(),
                        }
                    }
                    _ => {
                        return Err(format!(
                            "test interpreter encountered unsupported instruction 0x{word:08X}"
                        ));
                    }
                }
                pc = next;
            }
            Err("test interpreter exceeded its instruction budget".to_string())
        }
    }

    fn default_runtime_music_override() -> RuntimeStageMusicOverride {
        RuntimeStageMusicOverride {
            area_index: 24,
            scenario_index: 0,
            main: RuntimeMusicRoleOverride::GameDefault,
            entrance: RuntimeMusicRoleOverride::GameDefault,
            inside: RuntimeMusicRoleOverride::GameDefault,
            switch_a: RuntimeMusicRoleOverride::GameDefault,
            switch_b: RuntimeMusicRoleOverride::GameDefault,
            transition: RuntimeStageMusicTransition::GameDefault,
        }
    }

    #[test]
    fn stage_music_dispatcher_preserves_aliases_and_explicit_role_precedence() {
        const STUB: u32 = 0x8000_2000;
        let hook = stage_music_test_hook();
        let mut override_ = default_runtime_music_override();
        override_.main = RuntimeMusicRoleOverride::Track {
            bgm_id: 0x8001_000d,
            wave_scene_id: Some(0x210),
        };
        override_.inside = RuntimeMusicRoleOverride::Silent;
        let stub = build_stage_music_stub(STUB, hook, 0x8000_0900, &[override_]).unwrap();
        let mut machine = StageMusicInterpreter::new(24, 0);
        machine.set_word(0, 0x204);
        machine.set_word(4, 0x8001_0021);
        machine.set_word(8, 0x8001_0021);
        machine.set_word(12, 0x8001_0021);
        machine.set_word(24, 0x8001_0008);
        machine.set_word(28, 0x8001_0009);
        machine
            .run(STUB, &stub.words[..stub.wrapper_word], hook)
            .unwrap();

        assert_eq!(machine.word(0), 0x210);
        assert_eq!(machine.word(8), 0x8001_000d);
        assert_eq!(machine.word(4), 0x8001_000d);
        assert_eq!(machine.word(12), 0xffff_fff0);
        assert_eq!(machine.word(24), 0x8001_0008);
        assert_eq!(machine.word(28), 0x8001_0009);

        let mut silent = default_runtime_music_override();
        silent.main = RuntimeMusicRoleOverride::Silent;
        let stub = build_stage_music_stub(STUB, hook, 0x8000_0900, &[silent]).unwrap();
        let mut machine = StageMusicInterpreter::new(24, 0);
        machine.set_word(0, 0x204);
        machine.set_word(4, 0x8001_0021);
        machine.set_word(8, 0x8001_0021);
        machine
            .run(STUB, &stub.words[..stub.wrapper_word], hook)
            .unwrap();
        assert_eq!(machine.word(0), 0x204);
        assert_eq!(machine.word(8), 0xffff_fff0);
        assert_eq!(machine.word(4), 0xffff_fff0);
    }

    #[test]
    fn stage_music_transition_modes_write_only_their_runtime_fields() {
        const STUB: u32 = 0x8000_2000;
        let hook = stage_music_test_hook();
        for (transition, expected_writes, expected_status, expected_fade) in [
            (
                RuntimeStageMusicTransition::GameDefault,
                Vec::<(usize, usize)>::new(),
                7,
                9,
            ),
            (RuntimeStageMusicTransition::Disabled, vec![(20, 1)], 7, 0),
            (
                RuntimeStageMusicTransition::InsideCrossfade,
                vec![(16, 1), (20, 1)],
                2,
                2,
            ),
            (
                RuntimeStageMusicTransition::SwitchRegion,
                vec![(20, 1)],
                7,
                3,
            ),
        ] {
            let mut override_ = default_runtime_music_override();
            override_.transition = transition;
            let stub = build_stage_music_stub(STUB, hook, 0x8000_0900, &[override_]).unwrap();
            let mut machine = StageMusicInterpreter::new(24, 0);
            machine.fields[16] = 7;
            machine.fields[20] = 9;
            machine
                .run(STUB, &stub.words[..stub.wrapper_word], hook)
                .unwrap();
            assert_eq!(machine.writes, expected_writes);
            assert_eq!(machine.fields[16], expected_status);
            assert_eq!(machine.fields[20], expected_fade);
        }
    }

    #[test]
    fn stage_music_wave_preloads_are_active_deduplicated_and_branch_valid() {
        const STUB: u32 = 0x8000_2000;
        const WAVE_LOADER: u32 = 0x8000_0900;
        let hook = stage_music_test_hook();
        let mut override_ = default_runtime_music_override();
        override_.main = RuntimeMusicRoleOverride::Track {
            bgm_id: 0x8001_0002,
            wave_scene_id: Some(0x202),
        };
        override_.entrance = RuntimeMusicRoleOverride::Track {
            bgm_id: 0x8001_0003,
            wave_scene_id: Some(0x203),
        };
        override_.inside = RuntimeMusicRoleOverride::Track {
            bgm_id: 0x8001_0004,
            wave_scene_id: Some(0x206),
        };
        override_.switch_a = RuntimeMusicRoleOverride::Track {
            bgm_id: 0x8001_0005,
            wave_scene_id: Some(0x203),
        };
        override_.switch_b = RuntimeMusicRoleOverride::Track {
            bgm_id: 0x8001_0006,
            wave_scene_id: Some(0x204),
        };
        override_.transition = RuntimeStageMusicTransition::SwitchRegion;
        let stub = build_stage_music_stub(STUB, hook, WAVE_LOADER, &[override_]).unwrap();
        let end = STUB + u32::try_from(stub.words.len() * 4).unwrap();
        let mut wave_calls = 0;
        for (index, word) in stub.words.iter().copied().enumerate() {
            let address = STUB + u32::try_from(index * 4).unwrap();
            if opcode(word) == 16 {
                let target = decode_conditional_branch_target(word, address).unwrap();
                assert!((STUB..=end).contains(&target));
            } else if opcode(word) == 18 {
                let target = decode_branch_target(word, address).unwrap();
                if word & 1 != 0 && target == WAVE_LOADER {
                    wave_calls += 1;
                }
            }
        }
        assert_eq!(wave_calls, 2);
    }

    #[test]
    fn empty_dialogue_and_balloon_overrides_are_byte_identical() {
        let source = b"not even a DOL because an empty patch must not inspect it";
        assert_eq!(patch_sms_dialogue_dol(source, &[], &[]).unwrap(), source);
    }

    #[test]
    fn dialogue_duplicate_keys_include_the_complete_runtime_actor_guard() {
        let first = RuntimeDialogueOverride {
            area_index: 1,
            scenario_index: 0,
            manager_instance_index: 0,
            original_message_id: 0x0003_0001,
            replacement_message_id: 0x0003_0002,
            factory_name: "NPCKinojii".to_string(),
            actor_type: 0x0400_0017,
            runtime_name_shift_jis: b"manager-a actor".to_vec(),
            reset_position_bits: [1.0_f32.to_bits(), 2.0_f32.to_bits(), 3.0_f32.to_bits()],
        };
        let mut second = first.clone();
        second.factory_name = "NPCMonteM".to_string();
        second.actor_type = 0x0400_0018;
        second.runtime_name_shift_jis = b"manager-b actor".to_vec();
        second.reset_position_bits[0] = 4.0_f32.to_bits();

        let mut talk_keys = std::collections::BTreeSet::new();
        assert!(talk_keys.insert(first.guard_key()));
        assert!(talk_keys.insert(second.guard_key()));

        for mutation in [
            {
                let mut override_ = first.clone();
                override_.actor_type += 1;
                override_
            },
            {
                let mut override_ = first.clone();
                override_.runtime_name_shift_jis.push(b'!');
                override_
            },
            {
                let mut override_ = first.clone();
                override_.reset_position_bits[2] ^= 1;
                override_
            },
        ] {
            assert_ne!(first.guard_key(), mutation.guard_key());
        }

        let mut same_runtime_guard = first.clone();
        same_runtime_guard.factory_name = "diagnostic-only-factory".to_string();
        same_runtime_guard.replacement_message_id += 1;
        assert!(!talk_keys.insert(same_runtime_guard.guard_key()));

        let first_balloon = RuntimeBalloonOverride {
            area_index: first.area_index,
            scenario_index: first.scenario_index,
            manager_instance_index: first.manager_instance_index,
            original_message_id: first.original_message_id,
            replacement_message_id: first.replacement_message_id,
            factory_name: first.factory_name,
            actor_type: first.actor_type,
            runtime_name_shift_jis: first.runtime_name_shift_jis,
            reset_position_bits: first.reset_position_bits,
        };
        let second_balloon = RuntimeBalloonOverride {
            area_index: second.area_index,
            scenario_index: second.scenario_index,
            manager_instance_index: second.manager_instance_index,
            original_message_id: second.original_message_id,
            replacement_message_id: second.replacement_message_id,
            factory_name: second.factory_name,
            actor_type: second.actor_type,
            runtime_name_shift_jis: second.runtime_name_shift_jis,
            reset_position_bits: second.reset_position_bits,
        };
        let balloon_keys = std::collections::BTreeSet::from([
            first_balloon.guard_key(),
            second_balloon.guard_key(),
        ]);
        assert_eq!(balloon_keys.len(), 2);
    }

    #[test]
    fn dialogue_guard_mismatches_bypass_the_only_replacement_store() {
        let section = DolSection {
            text: true,
            slot: 0,
            file_offset: 0x100,
            address: 0x8040_0000,
            size: 0x1000,
        };
        let talk = build_dialogue_stub(
            0x8050_0000,
            0x8060_0000,
            DialogueHook {
                replay_anchor: WordAnchor {
                    section,
                    word_index: 4,
                },
                replay_instruction: encode_d_form(32, 10, 11, 0xa0),
                this_register: 12,
                director_sda_offset: -0x7000,
            },
            1,
        )
        .unwrap();
        assert_guard_failures_skip_replacement(
            &talk,
            0x8050_0000,
            &[encode_d_form(32, 0, 3, 8), encode_d_form(36, 0, 12, 0x264)],
        );

        let balloon = build_balloon_dialogue_stub(
            0x8051_0000,
            0x8060_0000,
            BalloonDialogueHook {
                entry_anchor: WordAnchor {
                    section,
                    word_index: 8,
                },
                replay_instruction: 0x7c08_02a6,
                director_sda_offset: -0x7000,
            },
            1,
        )
        .unwrap();
        assert_guard_failures_skip_replacement(
            &balloon,
            0x8051_0000,
            &[encode_d_form(32, 0, 3, 8), encode_d_form(36, 0, 1, 20)],
        );
    }

    #[test]
    fn dialogue_binding_cfg_requires_real_convergence_not_proximity() {
        let layout = SyntheticLayout {
            text_address: 0x8000_1000,
            hook_word: 0x80,
            movie_word: 0x120,
            setter_word: 0x160,
        };
        let mut source = synthetic_dol(layout);
        let address = |word: usize| layout.text_address + u32::try_from(word * 4).unwrap();
        let mut write_word = |word: usize, value: u32| {
            write_be_u32(&mut source, SYNTHETIC_TEXT_OFFSET + word * 4, value).unwrap();
        };

        let converging_entry = 0x1b0;
        let converging_hook = 0x1b3;
        write_word(
            converging_entry,
            encode_beq(address(converging_entry), address(converging_entry + 2)).unwrap(),
        );
        write_word(
            converging_entry + 1,
            encode_branch(
                address(converging_entry + 1),
                address(converging_hook),
                false,
            )
            .unwrap(),
        );
        write_word(
            converging_entry + 2,
            encode_branch(
                address(converging_entry + 2),
                address(converging_hook),
                false,
            )
            .unwrap(),
        );
        write_word(converging_hook, PPC_NOP);

        let bypass_entry = 0x1b8;
        let bypass_hook = 0x1bb;
        write_word(
            bypass_entry,
            encode_beq(address(bypass_entry), address(bypass_hook)).unwrap(),
        );
        write_word(bypass_entry + 1, PPC_BLR);
        write_word(bypass_hook, PPC_NOP);

        let nearby_return = 0x1c0;
        let nearby_target = 0x1c2;
        write_word(nearby_return, PPC_BLR);
        write_word(nearby_target, PPC_NOP);

        let image = parse_dol(&source).unwrap();
        assert!(function_cfg_converges_at(
            &source,
            &image,
            address(converging_entry),
            address(converging_hook),
            0x40,
        )
        .unwrap());
        assert!(!function_cfg_converges_at(
            &source,
            &image,
            address(bypass_entry),
            address(bypass_hook),
            0x40,
        )
        .unwrap());
        assert!(!function_cfg_converges_at(
            &source,
            &image,
            address(nearby_return),
            address(nearby_target),
            0x40,
        )
        .unwrap());
    }

    #[test]
    fn dialogue_convergence_discovery_does_not_require_talk_binding() {
        let layout = SyntheticLayout {
            text_address: 0x8000_1000,
            hook_word: 0x80,
            movie_word: 0x120,
            setter_word: 0x140,
        };
        let mut source = synthetic_dol(layout);
        let director_word = 0x168;
        let convergence_word = 0x170;
        for (word, value) in [
            (director_word, encode_d_form(32, 8, 13, -0x7000)),
            (director_word + 1, encode_d_form(32, 7, 8, 0xa0)),
            (convergence_word, encode_d_form(36, 4, 12, 0x264)),
            (convergence_word + 1, encode_d_form(32, 0, 7, 0x4c)),
            (convergence_word + 2, encode_d_form(15, 6, 0, -0x400)),
            (convergence_word + 3, encode_cmplwi(6, 0x1d)),
        ] {
            write_be_u32(&mut source, SYNTHETIC_TEXT_OFFSET + word * 4, value).unwrap();
        }

        let image = parse_dol(&source).unwrap();
        let convergence = find_dialogue_convergence_hook(&source, &image).unwrap();
        assert_eq!(convergence.director_sda_offset, -0x7000);
        assert_eq!(convergence.this_register, 12);
        assert_eq!(
            convergence.replay_anchor.address().unwrap(),
            layout.text_address + u32::try_from((convergence_word + 1) * 4).unwrap()
        );

        let error = find_dialogue_hook(&source, &image).unwrap_err();
        assert!(error.contains("setTalkMsgID binding string"));
    }

    fn assert_guard_failures_skip_replacement(
        words: &[u32],
        stub_address: u32,
        replacement_sequence: &[u32; 2],
    ) {
        let next_index = words
            .iter()
            .position(|word| decode_d_form(*word, 14) == Some((3, 3, 36)))
            .expect("stub advances to the next 36-byte guard record");
        let next_address = dialogue_word_address(stub_address, next_index).unwrap();
        let mismatch_branches = words
            .iter()
            .enumerate()
            .filter(|(_, word)| **word & 0xffff_0003 == 0x4082_0000)
            .collect::<Vec<_>>();
        assert_eq!(mismatch_branches.len(), 10);
        for (index, word) in mismatch_branches {
            assert_eq!(
                decode_conditional_branch_target(
                    *word,
                    dialogue_word_address(stub_address, index).unwrap(),
                )
                .unwrap(),
                next_address,
                "every failed area/scenario/ordinal/id/type/name/reset guard must skip replacement"
            );
        }
        let replacement_index = words
            .windows(replacement_sequence.len())
            .position(|window| window == replacement_sequence)
            .expect("stub has one guarded replacement sequence");
        assert!(replacement_index < next_index);
        assert_eq!(
            words
                .windows(replacement_sequence.len())
                .filter(|window| *window == replacement_sequence)
                .count(),
            1
        );
        assert_eq!(words[next_index + 1] & 0xffff_0003, 0x4200_0000);
        assert!(next_index + 2 > replacement_index);
    }

    #[test]
    #[ignore = "requires SMS_BASE_ROOT with an extracted JP or US retail game"]
    fn local_retail_dialogue_and_balloon_hooks_are_semantically_located() {
        let path = std::env::var_os("SMS_DIALOGUE_DOL")
            .map(PathBuf::from)
            .unwrap_or_else(|| {
                PathBuf::from(std::env::var_os("SMS_BASE_ROOT").expect("SMS_BASE_ROOT"))
                    .join("sys/main.dol")
            });
        let source = fs::read(&path).unwrap();
        let source_image = parse_dol(&source).unwrap();
        let talk_hook = find_dialogue_hook(&source, &source_image).unwrap();
        let balloon_hook =
            find_balloon_dialogue_hook(&source, &source_image, talk_hook.director_sda_offset)
                .unwrap();
        let talk = RuntimeDialogueOverride {
            area_index: 1,
            scenario_index: 0,
            manager_instance_index: 0,
            original_message_id: 0x0003_0001,
            replacement_message_id: 0x0003_0002,
            factory_name: "NPCKinojii".to_string(),
            actor_type: 0x0400_0017,
            runtime_name_shift_jis: b"runtime-npc".to_vec(),
            reset_position_bits: [0, 0, 0],
        };
        let balloon = RuntimeBalloonOverride {
            area_index: talk.area_index,
            scenario_index: talk.scenario_index,
            manager_instance_index: talk.manager_instance_index,
            original_message_id: 3,
            replacement_message_id: 4,
            factory_name: talk.factory_name.clone(),
            actor_type: talk.actor_type,
            runtime_name_shift_jis: talk.runtime_name_shift_jis.clone(),
            reset_position_bits: talk.reset_position_bits,
        };
        let mut balloon_only_source = source.clone();
        let talk_binding = b"setTalkMsgID\0";
        let talk_binding_offset = balloon_only_source
            .windows(talk_binding.len())
            .position(|window| window == talk_binding)
            .expect("retail DOL contains the talk binding identity");
        balloon_only_source[talk_binding_offset] = b'X';
        let balloon_only_image = parse_dol(&balloon_only_source).unwrap();
        assert!(find_dialogue_hook(&balloon_only_source, &balloon_only_image).is_err());
        let balloon_only =
            patch_sms_dialogue_dol(&balloon_only_source, &[], std::slice::from_ref(&balloon))
                .unwrap_or_else(|error| panic!("{} balloon-only: {error}", path.display()));
        assert!(balloon_only
            .windows(BALLOON_DIALOGUE_MARKER.len())
            .any(|window| window == BALLOON_DIALOGUE_MARKER));
        assert!(!balloon_only
            .windows(DIALOGUE_MARKER.len())
            .any(|window| window == DIALOGUE_MARKER));
        let patched = patch_sms_dialogue_dol(&source, &[talk], &[balloon])
            .unwrap_or_else(|error| panic!("{}: {error}", path.display()));
        assert_ne!(patched, source);
        assert!(patched
            .windows(DIALOGUE_MARKER.len())
            .any(|window| window == DIALOGUE_MARKER));
        assert!(patched
            .windows(BALLOON_DIALOGUE_MARKER.len())
            .any(|window| window == BALLOON_DIALOGUE_MARKER));
        let patched_image = parse_dol(&patched).unwrap();
        for anchor in [talk_hook.replay_anchor, balloon_hook.entry_anchor] {
            let address = anchor.address().unwrap();
            let target = decode_branch_target(
                read_be_u32(&patched, anchor.file_offset().unwrap()).unwrap(),
                address,
            )
            .unwrap();
            assert!(patched_image.sections.iter().any(|section| {
                !section.text
                    && target >= section.address
                    && target < section.address + section.size
            }));
        }
    }

    #[test]
    fn ambiguous_semantic_transition_is_rejected_before_writing() {
        let layout = SyntheticLayout {
            text_address: 0x8000_1000,
            hook_word: 0x80,
            movie_word: 0x120,
            setter_word: 0x160,
        };
        let mut source = synthetic_dol(layout);
        let duplicate_word = 0xb0;
        let duplicate_address = layout.text_address + u32::try_from(duplicate_word * 4).unwrap();
        let duplicate_target = duplicate_address + 20;
        let sequence = [
            encode_d_form(32, 0, 13, -0x7000),
            0x2c00_0003,
            encode_bne(duplicate_address + 8, duplicate_target).unwrap(),
            encode_li(28, 4),
            encode_branch(duplicate_address + 16, duplicate_target, false).unwrap(),
        ];
        for (offset, word) in sequence.into_iter().enumerate() {
            write_be_u32(
                &mut source,
                SYNTHETIC_TEXT_OFFSET + (duplicate_word + offset) * 4,
                word,
            )
            .unwrap();
        }

        let error = patch_sms_direct_boot_dol(
            &source,
            &RuntimeStageTarget {
                area_index: 1,
                scenario_index: 0,
                archive_name: "stage.arc".to_string(),
            },
        )
        .unwrap_err();
        assert!(error.contains("Ambiguous semantic post-NLogo transition tail"));
    }

    #[test]
    #[ignore = "requires the adjacent local SMS retail and source-build artifacts"]
    fn local_retail_and_source_binaries_accept_the_same_semantic_patcher() {
        let sms_root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../..");
        let mut candidates = vec![
            sms_root.join("orig/GMSJ01/sys/main.dol"),
            sms_root.join("build/GMSJ01/mario.dol"),
        ];
        if let Some(path) = std::env::var_os("SMS_US_RETAIL_DOL") {
            candidates.push(PathBuf::from(path));
        }
        for path in candidates {
            assert!(
                path.is_file(),
                "missing local audit binary: {}",
                path.display()
            );
            audit_local_binary(&path);
        }
    }

    #[test]
    #[ignore = "requires SMS_US_RETAIL_DOL"]
    fn maximum_stage_music_profile_payload_fits_retail_stack_guard() {
        let path = PathBuf::from(std::env::var_os("SMS_US_RETAIL_DOL").expect("SMS_US_RETAIL_DOL"));
        let source = fs::read(&path).unwrap();
        let overrides = (0..MAX_STAGE_MUSIC_OVERRIDES)
            .map(|index| RuntimeStageMusicOverride {
                area_index: u8::try_from(index / 8).unwrap(),
                scenario_index: u8::try_from(index % 8).unwrap(),
                main: RuntimeMusicRoleOverride::Track {
                    bgm_id: 0x8001_0001,
                    wave_scene_id: Some(0x200),
                },
                entrance: RuntimeMusicRoleOverride::Track {
                    bgm_id: 0x8001_0002,
                    wave_scene_id: Some(0x201),
                },
                inside: RuntimeMusicRoleOverride::Track {
                    bgm_id: 0x8001_0003,
                    wave_scene_id: Some(0x202),
                },
                switch_a: RuntimeMusicRoleOverride::Track {
                    bgm_id: 0x8001_0004,
                    wave_scene_id: Some(0x203),
                },
                switch_b: RuntimeMusicRoleOverride::Track {
                    bgm_id: 0x8001_0005,
                    wave_scene_id: Some(0x204),
                },
                transition: RuntimeStageMusicTransition::GameDefault,
            })
            .collect::<Vec<_>>();
        let patched = patch_sms_stage_music_dol(&source, &overrides)
            .unwrap_or_else(|error| panic!("maximum retail stage-music payload: {error}"));
        let image = parse_dol(&patched.bytes).unwrap();
        let section = image
            .sections
            .iter()
            .find(|section| section.text && section.address == patched.stub_address)
            .unwrap();
        let stack_top = find_stack_top(&source, &parse_dol(&source).unwrap()).unwrap();
        assert!(
            section.address_end().unwrap() + MIN_STAGE_MUSIC_STACK_GAP <= stack_top,
            "maximum stage-music payload crossed the retail stack guard"
        );
    }

    #[test]
    #[ignore = "requires SMS_US_RETAIL_DOL"]
    fn packaged_sound_music_dialogue_balloon_and_direct_boot_compose_on_retail_dol() {
        let path = PathBuf::from(std::env::var_os("SMS_US_RETAIL_DOL").expect("SMS_US_RETAIL_DOL"));
        let source = fs::read(&path).unwrap();
        let sound = patch_sms_sound_assignments_dol(
            &source,
            &[
                RuntimeSoundAssignment {
                    kind: RuntimeSoundAssignmentKind::MapStatic,
                    source_name: "SoundObjRiver".to_string(),
                    original_sound_id: 0x500f,
                    sound_id: 0x5000,
                },
                RuntimeSoundAssignment {
                    kind: RuntimeSoundAssignmentKind::Graph,
                    source_name: "ms_sea".to_string(),
                    original_sound_id: 0x5000,
                    sound_id: 0x5003,
                },
            ],
        )
        .unwrap_or_else(|error| panic!("composed sound assignments: {error}"));
        let goop = patch_sms_goop_enemy_managers_dol(
            &sound,
            &BTreeSet::from([
                RuntimeGoopManagerPatch::PoiHanaAnyArea,
                RuntimeGoopManagerPatch::PoiHanaRedPool,
                RuntimeGoopManagerPatch::TelesaVisibleReset,
            ]),
        )
        .unwrap_or_else(|error| panic!("composed goop manager patches: {error}"));
        let music = patch_sms_stage_music_dol(
            &goop,
            &[
                RuntimeStageMusicOverride {
                    area_index: 1,
                    scenario_index: 0,
                    main: RuntimeMusicRoleOverride::Track {
                        bgm_id: 0x8001_0002,
                        wave_scene_id: Some(0x202),
                    },
                    entrance: RuntimeMusicRoleOverride::GameDefault,
                    inside: RuntimeMusicRoleOverride::Track {
                        bgm_id: 0x8001_0003,
                        wave_scene_id: Some(0x203),
                    },
                    switch_a: RuntimeMusicRoleOverride::GameDefault,
                    switch_b: RuntimeMusicRoleOverride::GameDefault,
                    transition: RuntimeStageMusicTransition::InsideCrossfade,
                },
                RuntimeStageMusicOverride {
                    area_index: 17,
                    scenario_index: 1,
                    main: RuntimeMusicRoleOverride::Track {
                        bgm_id: 0x8001_0001,
                        wave_scene_id: Some(0x201),
                    },
                    entrance: RuntimeMusicRoleOverride::GameDefault,
                    inside: RuntimeMusicRoleOverride::GameDefault,
                    switch_a: RuntimeMusicRoleOverride::GameDefault,
                    switch_b: RuntimeMusicRoleOverride::GameDefault,
                    transition: RuntimeStageMusicTransition::GameDefault,
                },
            ],
        )
        .unwrap_or_else(|error| panic!("{}: {error}", path.display()));
        assert!(music.bytes.len() > source.len());
        let source_image = parse_dol(&source).unwrap();
        let sound_hook = find_sound_stage_hook(&source, &source_image).unwrap();
        let original_stack_top = find_stack_top(&source, &source_image).unwrap();
        let music_image = parse_dol(&music.bytes).unwrap();
        let patched_stack_top = find_stack_top(&music.bytes, &music_image).unwrap();
        assert!(address_is_in_text(&music_image.sections, music.stub_address, 4).unwrap());
        let music_section = music_image
            .sections
            .iter()
            .find(|section| section.text && section.address == music.stub_address)
            .unwrap();
        assert_eq!(patched_stack_top, original_stack_top);
        assert_eq!(music.stub_address % FILE_ALIGNMENT, 0);
        assert!(
            music.stub_address
                >= source_image
                    .sections
                    .iter()
                    .map(|section| section.address_end().unwrap())
                    .max()
                    .unwrap()
        );
        assert!(
            music_section.address_end().unwrap() + MIN_STAGE_MUSIC_STACK_GAP <= original_stack_top
        );
        let original_init_call = WordAnchor {
            section: sound_hook.dispatch_anchor.section,
            word_index: sound_hook.dispatch_anchor.word_index - 1,
        };
        assert_eq!(
            section_word(&music.bytes, original_init_call, 0).unwrap(),
            section_word(&source, original_init_call, 0).unwrap()
        );
        assert_eq!(
            section_word(&music.bytes, sound_hook.dispatch_anchor, 0).unwrap(),
            encode_branch(music.hook_address, music.stub_address, false).unwrap()
        );

        let dialogue = patch_sms_dialogue_dol(
            &music.bytes,
            &[RuntimeDialogueOverride {
                area_index: 17,
                scenario_index: 1,
                manager_instance_index: 0,
                original_message_id: 0x0003_0001,
                replacement_message_id: 0x0003_0002,
                factory_name: "NPCKinojii".to_string(),
                actor_type: 0x0400_0017,
                runtime_name_shift_jis: b"runtime-npc".to_vec(),
                reset_position_bits: [0, 0, 0],
            }],
            &[RuntimeBalloonOverride {
                area_index: 17,
                scenario_index: 1,
                manager_instance_index: 0,
                original_message_id: 3,
                replacement_message_id: 4,
                factory_name: "NPCKinojii".to_string(),
                actor_type: 0x0400_0017,
                runtime_name_shift_jis: b"runtime-npc".to_vec(),
                reset_position_bits: [0, 0, 0],
            }],
        )
        .unwrap_or_else(|error| panic!("composed dialogue: {error}"));
        let direct = patch_sms_direct_boot_dol(
            &dialogue,
            &RuntimeStageTarget {
                area_index: 17,
                scenario_index: 1,
                archive_name: "custom.arc".to_string(),
            },
        )
        .unwrap_or_else(|error| panic!("composed direct boot: {error}"));
        parse_dol(&direct.bytes).unwrap();
        assert!(direct.bytes.len() >= dialogue.len());
    }

    #[test]
    #[ignore = "requires SMS_US_RETAIL_DOL"]
    fn packaged_sound_helper_assignments_patch_retail_tables_by_identity() {
        let path = PathBuf::from(std::env::var_os("SMS_US_RETAIL_DOL").expect("SMS_US_RETAIL_DOL"));
        let source = fs::read(&path).unwrap();
        let patched = patch_sms_sound_assignments_dol(
            &source,
            &[
                RuntimeSoundAssignment {
                    kind: RuntimeSoundAssignmentKind::MapStatic,
                    source_name: "SoundObjRiver".to_string(),
                    original_sound_id: 0x500f,
                    sound_id: 0x5000,
                },
                RuntimeSoundAssignment {
                    kind: RuntimeSoundAssignmentKind::Graph,
                    source_name: "ms_sea".to_string(),
                    original_sound_id: 0x5000,
                    sound_id: 0x5003,
                },
            ],
        )
        .unwrap_or_else(|error| panic!("{}: {error}", path.display()));
        assert_eq!(patched.len(), source.len());
        assert_ne!(patched, source);
        parse_dol(&patched).unwrap();
    }

    fn audit_local_binary(path: &Path) {
        let source = fs::read(path).unwrap();
        let source_image = parse_dol(&source).unwrap();
        let talk_hook = find_dialogue_hook(&source, &source_image)
            .unwrap_or_else(|error| panic!("{} dialogue hook: {error}", path.display()));
        find_balloon_dialogue_hook(&source, &source_image, talk_hook.director_sda_offset)
            .unwrap_or_else(|error| panic!("{} balloon hook: {error}", path.display()));
        let dialogue = patch_sms_dialogue_dol(
            &source,
            &[RuntimeDialogueOverride {
                area_index: 17,
                scenario_index: 1,
                manager_instance_index: 0,
                original_message_id: 0x0003_0001,
                replacement_message_id: 0x0003_0002,
                factory_name: "NPCKinojii".to_string(),
                actor_type: 0x0400_0017,
                runtime_name_shift_jis: b"runtime-npc".to_vec(),
                reset_position_bits: [0, 0, 0],
            }],
            &[RuntimeBalloonOverride {
                area_index: 17,
                scenario_index: 1,
                manager_instance_index: 0,
                original_message_id: 3,
                replacement_message_id: 4,
                factory_name: "NPCKinojii".to_string(),
                actor_type: 0x0400_0017,
                runtime_name_shift_jis: b"runtime-npc".to_vec(),
                reset_position_bits: [0, 0, 0],
            }],
        )
        .unwrap_or_else(|error| panic!("{} dialogue patch: {error}", path.display()));
        assert!(dialogue
            .windows(DIALOGUE_MARKER.len())
            .any(|window| window == DIALOGUE_MARKER));
        assert!(dialogue
            .windows(BALLOON_DIALOGUE_MARKER.len())
            .any(|window| window == BALLOON_DIALOGUE_MARKER));
        let patched = patch_sms_direct_boot_dol(
            &dialogue,
            &RuntimeStageTarget {
                area_index: 17,
                scenario_index: 1,
                archive_name: "smsEditorRuntimeTest.arc".to_string(),
            },
        )
        .unwrap_or_else(|error| panic!("{}: {error}", path.display()));
        eprintln!(
            "{}: hook=0x{:08X}, movie=0x{:08X}, death=0x{:08X}->0x{:08X}, stub=0x{:08X}, bytes={}",
            path.display(),
            patched.hook_address,
            patched.movie_hook_address,
            patched.death_hook_address,
            patched.death_stub_address,
            patched.stub_address,
            patched.bytes.len()
        );
        assert_eq!(patched.bytes.len(), dialogue.len());
        let image = parse_dol(&patched.bytes).unwrap();
        assert!(address_is_in_text(&image.sections, patched.stub_address, 4).unwrap());
        assert!(address_is_in_text(&image.sections, patched.death_stub_address, 4).unwrap());
    }
}
