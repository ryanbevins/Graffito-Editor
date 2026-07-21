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

#[cfg(test)]
const PPC_NOP: u32 = 0x6000_0000;
const PPC_BLR: u32 = 0x4e80_0020;
const DIRECT_BOOT_FLAG: u16 = 0x534d;
const POST_NLOGO_STATE: i16 = 5;
const FILE_ALIGNMENT: u32 = 0x20;
const MIN_STACK_GAP: u32 = 0x100;
const THIS_SEARCH_WORDS: usize = 0x100;
const STATE_COMPARE_SEARCH_WORDS: usize = 0x40;
const NLOGO_DIRECT_SEARCH_WORDS: usize = 0x40;
const MOVIE_SEARCH_WORDS: usize = 0xc0;
const ENTRY_BL_SEARCH_WORDS: usize = 0x40;
const INIT_REGISTER_SEARCH_WORDS: usize = 0x40;
const TRANSITION_CAVE_WORDS: usize = 7;
const MOVIE_PRIMARY_CAVE_WORDS: usize = 7;
const MOVIE_SECONDARY_CAVE_WORDS: usize = 3;
const PROGRESSION_HEAD_CAVE_WORDS: usize = 7;
const PROGRESSION_COUNTS_HEAD_CAVE_WORDS: usize = 6;
const PROGRESSION_COUNTS_TAIL_CAVE_WORDS: usize = 5;
const FULL_CARD_BOOL_BYTES: i16 = 119;
const LIVES_OFFSET: i16 = 124;
const SHINE_COUNT_OFFSET: i16 = 208;
const BLUE_COIN_COUNT_OFFSET: i16 = 212;
const FULL_LIVES: i16 = 99;
const FULL_SHINE_COUNT: i16 = 120;
const FULL_BLUE_COIN_COUNT: i16 = 240;
const TRANSITION_WORD_COUNT: u32 = 8;
const MOVIE_WRAPPER_WORD_COUNT: u32 = 9;
const DIRECT_BOOT_MARKER: &[u8] = b"SMS_EDITOR_DIRECT_BOOT_V1\0";

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct RuntimeStageTarget {
    pub(super) area_index: u8,
    pub(super) scenario_index: u8,
    pub(super) archive_name: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct DirectBootDol {
    pub(super) bytes: Vec<u8>,
    pub(super) logo_bypass_address: u32,
    pub(super) hook_address: u32,
    pub(super) movie_hook_address: u32,
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
    progression_head: CodeCave,
    progression_counts_head: CodeCave,
    progression_counts_tail: CodeCave,
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
    let flag_manager_sda_offset = find_flag_manager_sda_offset(source, &image)?;
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
    let original_transition_target =
        decode_branch_target(section_word(source, hook.anchor, 4)?, hook_address)?;
    let caves = choose_direct_boot_caves(
        find_zero_alignment_code_caves(source, &image)?,
        hook_address,
        movie_hook_address,
        original_transition_target,
        movie.original_target,
    )?;
    let transition_address = caves.transition.anchor.address()?;
    let movie_primary_address = caves.movie_primary.anchor.address()?;
    let movie_secondary_address = caves.movie_secondary.anchor.address()?;
    let progression_addresses = [
        caves.progression_head.anchor.address()?,
        caves.progression_counts_head.anchor.address()?,
        caves.progression_counts_tail.anchor.address()?,
    ];

    let transition_words = build_transition_cave(
        transition_address,
        progression_addresses[0],
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
    let progression_words = build_full_progression_caves(
        progression_addresses,
        original_transition_target,
        flag_manager_sda_offset,
    )?;

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
    write_words(&mut bytes, caves.transition.anchor, &transition_words)?;
    write_words(&mut bytes, caves.movie_primary.anchor, &movie_primary_words)?;
    write_words(
        &mut bytes,
        caves.movie_secondary.anchor,
        &movie_secondary_words,
    )?;
    for (cave, words) in [
        (caves.progression_head, &progression_words[0]),
        (caves.progression_counts_head, &progression_words[1]),
        (caves.progression_counts_tail, &progression_words[2]),
    ] {
        write_words(&mut bytes, cave.anchor, words)?;
    }
    parse_dol(&bytes)?;

    Ok(DirectBootDol {
        bytes,
        logo_bypass_address: setup_case_address,
        hook_address,
        movie_hook_address,
        stub_address: transition_address,
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

fn find_flag_manager_sda_offset(source: &[u8], image: &DolImage) -> Result<i16, String> {
    let mut offsets = std::collections::BTreeSet::new();
    for section in image
        .sections
        .iter()
        .copied()
        .filter(|section| section.text)
    {
        let words = section_words(source, section)?;
        for sequence in words.windows(4) {
            let Some((flag_register, zero_register, upper)) = decode_d_form(sequence[0], 15) else {
                continue;
            };
            let Some(instance_register) = decode_lwz_from_r13(sequence[1]) else {
                continue;
            };
            if zero_register != 0
                || upper != 1
                || decode_d_form(sequence[2], 14) != Some((flag_register, flag_register, 0x0386))
                || !is_relative_bl(sequence[3])
                || instance_register == flag_register
            {
                continue;
            }
            offsets.insert(immediate_i16(sequence[1]));
        }
    }
    match offsets.into_iter().collect::<Vec<_>>().as_slice() {
        [offset] => Ok(*offset),
        [] => Err(
            "Could not derive TFlagManager::smInstance from the 0x10386 getBool call".to_string(),
        ),
        offsets => Err(format!(
            "Ambiguous TFlagManager::smInstance SDA offset: found {} candidates",
            offsets.len()
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

#[allow(dead_code)]
fn derive_stack_top(source: &[u8], image: &DolImage) -> Result<u32, String> {
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

#[allow(dead_code)]
fn derive_safe_upper_boundary(
    source: &[u8],
    image: &DolImage,
    stack_top: u32,
) -> Result<u32, String> {
    let mut upper_boundary = None;
    for section in image
        .sections
        .iter()
        .copied()
        .filter(|section| section.text)
    {
        let words = section_words(source, section)?;
        for pair in words.windows(2) {
            if let Some(value) = decode_materialized_address(pair[0], pair[1]) {
                if value > stack_top {
                    upper_boundary =
                        Some(upper_boundary.map_or(value, |current: u32| current.min(value)));
                }
            }
        }
    }
    let upper_boundary = upper_boundary.ok_or_else(|| {
        format!("Could not infer a safe address boundary above stack top 0x{stack_top:08X}")
    })?;
    let gap = upper_boundary - stack_top;
    if gap < MIN_STACK_GAP {
        return Err(format!(
            "Inferred stack gap is only 0x{gap:X} bytes (0x{stack_top:08X}..0x{upper_boundary:08X}); direct boot requires at least 0x{MIN_STACK_GAP:X}"
        ));
    }
    Ok(upper_boundary)
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
    progression_address: u32,
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
        encode_branch(return_address, progression_address, false)?,
    ];
    debug_assert_eq!(words.len(), TRANSITION_CAVE_WORDS);
    Ok(words)
}

fn build_full_progression_caves(
    addresses: [u32; 3],
    original_transition_target: u32,
    flag_manager_sda_offset: i16,
) -> Result<[Vec<u32>; 3], String> {
    let head_tail = addresses[0]
        .checked_add(24)
        .ok_or_else(|| "Progression head tail address overflows u32".to_string())?;
    let card_bool_loop_branch = addresses[1]
        .checked_add(4)
        .ok_or_else(|| "Card-bool loop branch address overflows u32".to_string())?;
    let counts_head_tail = addresses[1]
        .checked_add(20)
        .ok_or_else(|| "Progression-count head tail address overflows u32".to_string())?;
    let counts_tail_return = addresses[2]
        .checked_add(16)
        .ok_or_else(|| "Progression-count tail return address overflows u32".to_string())?;

    let head = vec![
        encode_mfctr(11),
        encode_d_form(32, 7, 13, flag_manager_sda_offset),
        encode_li(8, -1),
        encode_li(9, FULL_CARD_BOOL_BYTES),
        encode_mtctr(9),
        encode_d_form(14, 10, 7, -1),
        encode_branch(head_tail, addresses[1], false)?,
    ];
    let counts_head = vec![
        encode_d_form(39, 8, 10, 1),
        encode_bdnz(card_bool_loop_branch, addresses[1])?,
        encode_li(8, FULL_LIVES),
        encode_d_form(36, 8, 7, LIVES_OFFSET),
        encode_li(8, FULL_SHINE_COUNT),
        encode_branch(counts_head_tail, addresses[2], false)?,
    ];
    let counts_tail = vec![
        encode_d_form(36, 8, 7, SHINE_COUNT_OFFSET),
        encode_li(8, FULL_BLUE_COIN_COUNT),
        encode_d_form(36, 8, 7, BLUE_COIN_COUNT_OFFSET),
        encode_mtctr(11),
        encode_branch(counts_tail_return, original_transition_target, false)?,
    ];
    debug_assert_eq!(head.len(), PROGRESSION_HEAD_CAVE_WORDS);
    debug_assert_eq!(counts_head.len(), PROGRESSION_COUNTS_HEAD_CAVE_WORDS);
    debug_assert_eq!(counts_tail.len(), PROGRESSION_COUNTS_TAIL_CAVE_WORDS);
    Ok([head, counts_head, counts_tail])
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
    original_transition_target: u32,
    original_movie_target: u32,
) -> Result<DirectBootCaves, String> {
    const REQUIREMENTS: [usize; 6] = [
        TRANSITION_CAVE_WORDS,
        MOVIE_PRIMARY_CAVE_WORDS,
        MOVIE_SECONDARY_CAVE_WORDS,
        PROGRESSION_HEAD_CAVE_WORDS,
        PROGRESSION_COUNTS_HEAD_CAVE_WORDS,
        PROGRESSION_COUNTS_TAIL_CAVE_WORDS,
    ];
    let mut selected = Vec::with_capacity(REQUIREMENTS.len());
    if let Some(selection) =
        select_direct_boot_caves(&caves, &REQUIREMENTS, &mut selected, |selection| {
            direct_boot_cave_branches_are_encodable(
                selection,
                hook_address,
                movie_hook_address,
                original_transition_target,
                original_movie_target,
            )
        })
    {
        return Ok(DirectBootCaves {
            transition: selection[0],
            movie_primary: selection[1],
            movie_secondary: selection[2],
            progression_head: selection[3],
            progression_counts_head: selection[4],
            progression_counts_tail: selection[5],
        });
    }
    Err(format!(
        "Could not find six safe executable alignment caves for direct boot and full Delfino progression (word requirements: {REQUIREMENTS:?})"
    ))
}

fn select_direct_boot_caves<F>(
    caves: &[CodeCave],
    requirements: &[usize],
    selected: &mut Vec<CodeCave>,
    accept: F,
) -> Option<Vec<CodeCave>>
where
    F: Fn(&[CodeCave]) -> bool + Copy,
{
    let Some((&required_words, remaining)) = requirements.split_first() else {
        return accept(selected).then(|| selected.clone());
    };
    for cave in caves
        .iter()
        .copied()
        .filter(|cave| cave.word_count >= required_words)
    {
        if selected
            .iter()
            .any(|selected_cave| selected_cave.anchor == cave.anchor)
        {
            continue;
        }
        selected.push(cave);
        if let Some(selection) = select_direct_boot_caves(caves, remaining, selected, accept) {
            return Some(selection);
        }
        selected.pop();
    }
    None
}

fn direct_boot_cave_branches_are_encodable(
    caves: &[CodeCave],
    hook_address: u32,
    movie_hook_address: u32,
    original_transition_target: u32,
    original_movie_target: u32,
) -> bool {
    let addresses = caves
        .iter()
        .map(|cave| cave.anchor.address())
        .collect::<Result<Vec<_>, _>>();
    let Ok(addresses) = addresses else {
        return false;
    };
    let transition = addresses[0];
    let movie_primary = addresses[1];
    let movie_secondary = addresses[2];
    let progression_head = addresses[3];
    let progression_counts_head = addresses[4];
    let progression_counts_tail = addresses[5];
    let movie_secondary_tail = movie_secondary + 8;
    encode_branch(hook_address, transition, false).is_ok()
        && encode_branch(transition + 24, progression_head, false).is_ok()
        && encode_branch(progression_head + 24, progression_counts_head, false).is_ok()
        && encode_branch(progression_counts_head + 20, progression_counts_tail, false).is_ok()
        && encode_branch(
            progression_counts_tail + 16,
            original_transition_target,
            false,
        )
        .is_ok()
        && encode_branch(movie_hook_address, movie_primary, true).is_ok()
        && encode_bne(movie_primary + 8, movie_secondary_tail).is_ok()
        && encode_branch(movie_primary + 24, movie_secondary, false).is_ok()
        && encode_branch(movie_secondary_tail, original_movie_target, false).is_ok()
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

fn encode_bdnz(from: u32, to: u32) -> Result<u32, String> {
    let displacement = i64::from(to) - i64::from(from);
    if from & 3 != 0 || to & 3 != 0 || !(-0x8000..=0x7ffc).contains(&displacement) {
        return Err(format!(
            "PowerPC count branch 0x{from:08X} -> 0x{to:08X} is out of range or unaligned"
        ));
    }
    Ok(0x4200_0000 | ((displacement as i32 as u32) & 0x0000_fffc))
}

fn encode_mfctr(register: u8) -> u32 {
    0x7c09_02a6 | (u32::from(register) << 21)
}

fn encode_mtctr(register: u8) -> u32 {
    0x7c08_03a6 | (u32::from(register) << 21)
}

fn encode_li(register: u8, immediate: i16) -> u32 {
    0x3800_0000 | (u32::from(register) << 21) | u32::from(immediate as u16)
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

        // Six zero-filled linker alignment gaps, each immediately after a
        // return and ending on the next 0x20-byte function boundary.
        install_alignment_cave(&mut words, 0x108, PROGRESSION_HEAD_CAVE_WORDS);
        install_alignment_cave(&mut words, 0x148, PROGRESSION_COUNTS_HEAD_CAVE_WORDS);
        install_alignment_cave(&mut words, 0x158, PROGRESSION_COUNTS_TAIL_CAVE_WORDS);
        install_alignment_cave(&mut words, 0x188, TRANSITION_CAVE_WORDS);
        install_alignment_cave(&mut words, 0x198, MOVIE_PRIMARY_CAVE_WORDS);
        install_alignment_cave(&mut words, 0x1a8, MOVIE_SECONDARY_CAVE_WORDS);
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

        // A stable, semantically identifiable TFlagManager::smInstance use:
        // getBool(0x10386), one of Delfino Plaza's progression gates.
        let flag_word = 0x1b0;
        words[flag_word] = encode_d_form(15, 4, 0, 1);
        words[flag_word + 1] = encode_d_form(32, 3, 13, -0x6800);
        words[flag_word + 2] = encode_d_form(14, 4, 4, 0x0386);
        words[flag_word + 3] = encode_branch(address(flag_word + 3), address(0x1c4), true).unwrap();

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
    fn full_progression_caves_encode_the_declared_delfino_state() {
        let addresses = [0x8000_1000, 0x8000_1100, 0x8000_1200];
        let original_target = 0x8000_2000;
        let words = build_full_progression_caves(addresses, original_target, -0x6800).unwrap();

        assert_eq!(words[0][1], encode_d_form(32, 7, 13, -0x6800));
        assert_eq!(words[0][2], encode_li(8, -1));
        assert_eq!(words[0][3], encode_li(9, 119));
        assert_eq!(words[1][0], encode_d_form(39, 8, 10, 1));
        assert_eq!(words[1][2], encode_li(8, 99));
        assert_eq!(words[1][3], encode_d_form(36, 8, 7, 124));
        assert_eq!(words[1][4], encode_li(8, 120));
        assert_eq!(words[2][0], encode_d_form(36, 8, 7, 208));
        assert_eq!(words[2][1], encode_li(8, 240));
        assert_eq!(words[2][2], encode_d_form(36, 8, 7, 212));
        assert_eq!(
            decode_branch_target(words[2][4], addresses[2] + 16).unwrap(),
            original_target
        );
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
        let progression_address = decode_branch_target(
            read_be_u32(&patched.bytes, payload_offset + 6 * 4).unwrap(),
            patched.stub_address + 6 * 4,
        )
        .unwrap();
        let progression_offset =
            usize::try_from(cave_section.file_offset + progression_address - cave_section.address)
                .unwrap();
        assert_eq!(
            read_be_u32(&patched.bytes, progression_offset).unwrap(),
            encode_mfctr(11)
        );
        assert_eq!(
            read_be_u32(&patched.bytes, progression_offset + 4).unwrap(),
            encode_d_form(32, 7, 13, -0x6800)
        );
        assert_eq!(
            read_be_u32(&patched.bytes, progression_offset + 2 * 4).unwrap(),
            encode_li(8, -1)
        );
        assert_eq!(
            read_be_u32(&patched.bytes, progression_offset + 3 * 4).unwrap(),
            encode_li(9, FULL_CARD_BOOL_BYTES)
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

    fn audit_local_binary(path: &Path) {
        let source = fs::read(path).unwrap();
        let patched = patch_sms_direct_boot_dol(
            &source,
            &RuntimeStageTarget {
                area_index: 17,
                scenario_index: 1,
                archive_name: "smsEditorRuntimeTest.arc".to_string(),
            },
        )
        .unwrap_or_else(|error| panic!("{}: {error}", path.display()));
        eprintln!(
            "{}: hook=0x{:08X}, movie=0x{:08X}, stub=0x{:08X}, bytes={}",
            path.display(),
            patched.hook_address,
            patched.movie_hook_address,
            patched.stub_address,
            patched.bytes.len()
        );
        assert_eq!(patched.bytes.len(), source.len());
        let image = parse_dol(&patched.bytes).unwrap();
        assert!(address_is_in_text(&image.sections, patched.stub_address, 4).unwrap());
    }
}
