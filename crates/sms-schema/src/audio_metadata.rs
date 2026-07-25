use std::collections::{BTreeMap, BTreeSet};
use std::sync::LazyLock;

use regex::Regex;

use crate::{
    BgmWaveSceneDefinition, DialogueVoiceDefinition, Result, SchemaError, StageAudioAreaDefinition,
    StageAudioScenarioDefinition, StageAudioStateDefinition,
};

const BGM_BASE: u32 = 0x8001_0000;
static BGM_ASSIGNMENT: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"base\s*\+\s*(0x[0-9A-Fa-f]+)").expect("static BGM assignment regex is valid")
});
static SCENARIO_COMPARISON: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"^\s*area\s*(==|!=)\s*(\d+)\s*$").expect("static scenario condition regex is valid")
});
static NUMERIC_CASE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?m)^\s*case\s+(\d+)\s*:").expect("static switch case regex is valid")
});

pub(crate) fn extract_bgm_wave_scenes(
    source: &str,
    source_file: &str,
) -> Result<Vec<BgmWaveSceneDefinition>> {
    let mapping = Regex::new(r"(?s)case\s+(0x[0-9A-Fa-f]+)\s*:\s*return\s+(0x[0-9A-Fa-f]+)\s*;")
        .expect("static BGM mapping regex is valid");
    let mut result = BTreeMap::new();
    for captures in mapping.captures_iter(source) {
        let bgm_id = parse_hex(&captures[1])?;
        let wave_scene_id = parse_hex(&captures[2])?;
        if result.insert(bgm_id, wave_scene_id).is_some() {
            return Err(SchemaError::RegistryInvariant {
                detail: format!("duplicate BGM-to-wave-scene mapping {bgm_id:#010x}"),
            });
        }
    }
    Ok(result
        .into_iter()
        .map(|(bgm_id, wave_scene_id)| BgmWaveSceneDefinition {
            bgm_id,
            wave_scene_id,
            source_file: source_file.to_string(),
        })
        .collect())
}

pub(crate) fn extract_dialogue_voices(
    source: &str,
    source_file: &str,
) -> Result<Vec<DialogueVoiceDefinition>> {
    let declaration =
        source
            .find("scTalkSoundList")
            .ok_or_else(|| SchemaError::RegistryInvariant {
                detail: "Talk2D2.cpp does not declare scTalkSoundList".to_string(),
            })?;
    let body_start = source[declaration..]
        .find('{')
        .map(|offset| declaration + offset + 1)
        .ok_or_else(|| SchemaError::RegistryInvariant {
            detail: "scTalkSoundList has no initializer".to_string(),
        })?;
    let body_end = source[body_start..]
        .find('}')
        .map(|offset| body_start + offset)
        .ok_or_else(|| SchemaError::RegistryInvariant {
            detail: "scTalkSoundList initializer is not terminated".to_string(),
        })?;
    let literal =
        Regex::new(r"0[xX]([0-9A-Fa-f]+)").expect("static dialogue voice literal regex is valid");
    literal
        .captures_iter(&source[body_start..body_end])
        .enumerate()
        .map(|(index, capture)| {
            let index = u8::try_from(index).map_err(|_| SchemaError::RegistryInvariant {
                detail: "scTalkSoundList contains more entries than its u8 index supports"
                    .to_string(),
            })?;
            let sound_id = u32::from_str_radix(&capture[1], 16).map_err(|error| {
                SchemaError::RegistryInvariant {
                    detail: format!("invalid scTalkSoundList value: {error}"),
                }
            })?;
            Ok(DialogueVoiceDefinition {
                index,
                sound_id,
                source_file: source_file.to_string(),
            })
        })
        .collect()
}

pub(crate) fn extract_stage_audio_areas(
    source: &str,
    source_file: &str,
) -> Result<Vec<StageAudioAreaDefinition>> {
    let function = stage_audio_function(source).ok_or_else(|| SchemaError::RegistryInvariant {
        detail: "MSoundMainSide.cpp has no complete setMSoundEnterStage body".to_string(),
    })?;
    let candidates = numeric_branch_values(function);
    if candidates.is_empty() {
        return Ok(Vec::new());
    }

    let mut areas = Vec::new();
    for area_index in candidates.iter().copied() {
        let default = evaluate_stage_audio_function(function, area_index, u32::MAX);
        let mut scenario_overrides = candidates
            .iter()
            .copied()
            .filter_map(|scenario_index| {
                let state = evaluate_stage_audio_function(function, area_index, scenario_index);
                (state != default).then_some(StageAudioScenarioDefinition {
                    scenario_index,
                    state,
                })
            })
            .collect::<Vec<_>>();
        scenario_overrides.sort_by_key(|definition| definition.scenario_index);
        if default != StageAudioStateDefinition::default() || !scenario_overrides.is_empty() {
            areas.push(StageAudioAreaDefinition {
                area_index,
                default,
                scenario_overrides,
                source_file: source_file.to_string(),
            });
        }
    }
    let fallback_default = evaluate_stage_audio_function(function, u32::MAX, u32::MAX);
    let mut fallback_overrides = candidates
        .iter()
        .copied()
        .filter_map(|scenario_index| {
            let state = evaluate_stage_audio_function(function, u32::MAX, scenario_index);
            (state != fallback_default).then_some(StageAudioScenarioDefinition {
                scenario_index,
                state,
            })
        })
        .collect::<Vec<_>>();
    fallback_overrides.sort_by_key(|definition| definition.scenario_index);
    areas.push(StageAudioAreaDefinition {
        area_index: u32::MAX,
        default: fallback_default,
        scenario_overrides: fallback_overrides,
        source_file: source_file.to_string(),
    });
    areas.sort_by_key(|definition| definition.area_index);
    Ok(areas)
}

fn stage_audio_function(source: &str) -> Option<&str> {
    let function = source.find("void MSMainProc::setMSoundEnterStage")?;
    let open = source[function..].find('{')? + function;
    let close = matching_delimiter(source, open, b'{', b'}')?;
    Some(&source[open + 1..close])
}

fn numeric_branch_values(source: &str) -> BTreeSet<u32> {
    let case = Regex::new(r"\bcase\s+(\d+)\s*:").expect("static numeric case regex is valid");
    let comparison = Regex::new(r"\b(?:map|area)\s*(?:==|!=)\s*(\d+)")
        .expect("static stage condition regex is valid");
    case.captures_iter(source)
        .chain(comparison.captures_iter(source))
        .filter_map(|captures| captures[1].parse().ok())
        .collect()
}

fn evaluate_stage_audio_function(
    function: &str,
    area_index: u32,
    scenario_index: u32,
) -> StageAudioStateDefinition {
    let mut state = StageAudioStateDefinition::default();
    evaluate_audio_block(function, area_index, scenario_index, &mut state);
    state
}

fn evaluate_audio_block(
    block: &str,
    area_index: u32,
    scenario_index: u32,
    state: &mut StageAudioStateDefinition,
) -> bool {
    let mut cursor = 0;
    while cursor < block.len() {
        cursor = skip_cpp_space_and_comments(block, cursor);
        if cursor >= block.len() {
            break;
        }
        if keyword_at(block, cursor, "break") {
            return true;
        }
        if keyword_at(block, cursor, "case") || keyword_at(block, cursor, "default") {
            cursor = block[cursor..]
                .find(':')
                .map_or(block.len(), |offset| cursor + offset + 1);
            continue;
        }
        if keyword_at(block, cursor, "switch") {
            let Some(paren) = block[cursor..].find('(').map(|offset| cursor + offset) else {
                break;
            };
            let Some(paren_end) = matching_delimiter(block, paren, b'(', b')') else {
                break;
            };
            let selector = block[paren + 1..paren_end].trim();
            let body_start = skip_cpp_space_and_comments(block, paren_end + 1);
            if block.as_bytes().get(body_start) != Some(&b'{') {
                cursor = paren_end + 1;
                continue;
            }
            let Some(body_end) = matching_delimiter(block, body_start, b'{', b'}') else {
                break;
            };
            let value = match selector {
                "map" => Some(area_index),
                "area" => Some(scenario_index),
                _ => None,
            };
            if let Some(value) = value {
                if let Some(case_start) = switch_case_start(&block[body_start + 1..body_end], value)
                {
                    evaluate_audio_block(
                        &block[body_start + 1 + case_start..body_end],
                        area_index,
                        scenario_index,
                        state,
                    );
                }
            }
            cursor = body_end + 1;
            continue;
        }
        if keyword_at(block, cursor, "if") {
            let Some(paren) = block[cursor..].find('(').map(|offset| cursor + offset) else {
                break;
            };
            let Some(paren_end) = matching_delimiter(block, paren, b'(', b')') else {
                break;
            };
            let mut condition =
                evaluate_scenario_condition(&block[paren + 1..paren_end], scenario_index);
            let mut branch_start = skip_cpp_space_and_comments(block, paren_end + 1);
            let Some((branch, mut after_branch)) = cpp_statement(block, branch_start) else {
                break;
            };
            let mut selected = false;
            if condition == Some(true) {
                evaluate_audio_block(branch, area_index, scenario_index, state);
                selected = true;
            }
            loop {
                let else_at = skip_cpp_space_and_comments(block, after_branch);
                if !keyword_at(block, else_at, "else") {
                    cursor = after_branch;
                    break;
                }
                branch_start = skip_cpp_space_and_comments(block, else_at + 4);
                if keyword_at(block, branch_start, "if") {
                    let Some(next_paren) = block[branch_start..]
                        .find('(')
                        .map(|offset| branch_start + offset)
                    else {
                        cursor = branch_start + 2;
                        break;
                    };
                    let Some(next_paren_end) = matching_delimiter(block, next_paren, b'(', b')')
                    else {
                        cursor = next_paren + 1;
                        break;
                    };
                    condition = evaluate_scenario_condition(
                        &block[next_paren + 1..next_paren_end],
                        scenario_index,
                    );
                    branch_start = skip_cpp_space_and_comments(block, next_paren_end + 1);
                } else {
                    condition = Some(true);
                }
                let Some((next_branch, next_after)) = cpp_statement(block, branch_start) else {
                    cursor = branch_start;
                    break;
                };
                if !selected && condition == Some(true) {
                    evaluate_audio_block(next_branch, area_index, scenario_index, state);
                    selected = true;
                }
                after_branch = next_after;
            }
            continue;
        }
        let end = block[cursor..]
            .find(';')
            .map_or(block.len(), |offset| cursor + offset + 1);
        apply_audio_statement(&block[cursor..end], state);
        cursor = end;
    }
    false
}

fn apply_audio_statement(statement: &str, state: &mut StageAudioStateDefinition) {
    for (field, destination) in [
        (
            "stageBgmSilent",
            &mut state.secondary_bgm_id as &mut Option<u32>,
        ),
        ("demoBgm", &mut state.entrance_bgm_id),
        ("switchBgm2", &mut state.switch_bgm2_id),
        ("switchBgm", &mut state.switch_bgm_id),
        ("stageBgm", &mut state.primary_bgm_id),
    ] {
        let marker = format!("MSStageInfo::{field}");
        let Some(start) = statement.find(&marker) else {
            continue;
        };
        if statement
            .as_bytes()
            .get(start + marker.len())
            .is_some_and(|byte| byte.is_ascii_alphanumeric() || *byte == b'_')
        {
            continue;
        }
        let Some(equal) = statement[start + marker.len()..].find('=') else {
            continue;
        };
        let rhs = statement[start + marker.len() + equal + 1..].trim();
        *destination = if rhs.starts_with("cMSBgmNone") {
            None
        } else {
            BGM_ASSIGNMENT
                .captures(rhs)
                .and_then(|captures| parse_hex(&captures[1]).ok())
                .map(|offset| BGM_BASE + offset)
        };
        return;
    }
    for (field, destination) in [
        (
            "stageBgmSilentStartStatus",
            &mut state.secondary_start_status,
        ),
        ("flags", &mut state.flags),
        ("fadeEvent", &mut state.fade_event),
    ] {
        if let Some(start) = statement.find(&format!("MSStageInfo::{field}")) {
            if let Some(equal) = statement[start..].find('=') {
                let rhs = statement[start + equal + 1..]
                    .trim()
                    .trim_end_matches(';')
                    .trim();
                if let Ok(value) = rhs.parse::<u8>() {
                    *destination = value;
                }
            }
        }
    }
}

fn evaluate_scenario_condition(condition: &str, scenario_index: u32) -> Option<bool> {
    let captures = SCENARIO_COMPARISON.captures(condition)?;
    let expected = captures[2].parse::<u32>().ok()?;
    Some(if &captures[1] == "==" {
        scenario_index == expected
    } else {
        scenario_index != expected
    })
}

fn cpp_statement(source: &str, start: usize) -> Option<(&str, usize)> {
    if source.as_bytes().get(start) == Some(&b'{') {
        let end = matching_delimiter(source, start, b'{', b'}')?;
        Some((&source[start + 1..end], end + 1))
    } else {
        let end = source[start..].find(';')? + start;
        Some((&source[start..end + 1], end + 1))
    }
}

fn switch_case_start(block: &str, value: u32) -> Option<usize> {
    for captures in NUMERIC_CASE.captures_iter(block) {
        let matched = captures.get(0)?;
        let mut depth = 0_i32;
        for byte in &block.as_bytes()[..matched.start()] {
            match byte {
                b'{' => depth += 1,
                b'}' => depth -= 1,
                _ => {}
            }
        }
        if depth == 0 && captures[1].parse::<u32>().ok()? == value {
            return Some(matched.end());
        }
    }
    None
}

fn keyword_at(source: &str, offset: usize, keyword: &str) -> bool {
    source.get(offset..offset + keyword.len()) == Some(keyword)
        && source
            .as_bytes()
            .get(offset + keyword.len())
            .is_none_or(|byte| !byte.is_ascii_alphanumeric() && *byte != b'_')
}

fn skip_cpp_space_and_comments(source: &str, mut offset: usize) -> usize {
    loop {
        while source
            .as_bytes()
            .get(offset)
            .is_some_and(|byte| byte.is_ascii_whitespace())
        {
            offset += 1;
        }
        if source
            .get(offset..)
            .is_some_and(|tail| tail.starts_with("//"))
        {
            offset = source[offset..]
                .find('\n')
                .map_or(source.len(), |end| offset + end + 1);
            continue;
        }
        if source
            .get(offset..)
            .is_some_and(|tail| tail.starts_with("/*"))
        {
            offset = source[offset + 2..]
                .find("*/")
                .map_or(source.len(), |end| offset + 2 + end + 2);
            continue;
        }
        return offset;
    }
}

fn matching_delimiter(source: &str, open: usize, open_byte: u8, close_byte: u8) -> Option<usize> {
    let mut depth = 0_u32;
    for (relative, byte) in source.as_bytes()[open..].iter().copied().enumerate() {
        if byte == open_byte {
            depth += 1;
        } else if byte == close_byte {
            depth = depth.checked_sub(1)?;
            if depth == 0 {
                return Some(open + relative);
            }
        }
    }
    None
}

fn parse_hex(value: &str) -> Result<u32> {
    u32::from_str_radix(
        value
            .strip_prefix("0x")
            .or_else(|| value.strip_prefix("0X"))
            .unwrap_or(value),
        16,
    )
    .map_err(|error| SchemaError::RegistryInvariant {
        detail: format!("invalid hexadecimal audio metadata value {value}: {error}"),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extracts_decomp_bgm_wave_scene_pairs() {
        let source = "case 0x80010001:\n return 0x201;\ncase 0x80010002: return 0x202;";
        let result = extract_bgm_wave_scenes(source, "MSoundBGM.cpp").unwrap();
        assert_eq!(result[0].bgm_id, 0x8001_0001);
        assert_eq!(result[0].wave_scene_id, 0x201);
        assert_eq!(result[1].bgm_id, 0x8001_0002);
        assert_eq!(result[1].wave_scene_id, 0x202);
    }

    #[test]
    fn extracts_every_stage_audio_role_and_scenario_override() {
        let source = r#"
            void MSMainProc::setMSoundEnterStage(unsigned char map, unsigned char area) {
                unsigned long base = 0x80010000;
                switch (map) {
                case 24:
                    MSStageInfo::stageBgm = base + 0x21;
                    MSStageInfo::demoBgm = base + 0x21;
                    MSStageInfo::stageBgmSilent = base + 0x23;
                    MSStageInfo::stageBgmSilentStartStatus = 2;
                    MSStageInfo::switchBgm = base + 0x08;
                    MSStageInfo::switchBgm2 = base + 0x09;
                    MSStageInfo::flags = 0;
                    MSStageInfo::fadeEvent = 3;
                    if (area == 2) {
                        MSStageInfo::stageBgm = base + 0x16;
                    }
                    break;
                }
            }
        "#;
        let areas = extract_stage_audio_areas(source, "MSoundMainSide.cpp").unwrap();
        assert_eq!(areas.len(), 2);
        let area = areas
            .iter()
            .find(|definition| definition.area_index == 24)
            .unwrap();
        assert_eq!(area.area_index, 24);
        assert_eq!(area.default.primary_bgm_id, Some(0x8001_0021));
        assert_eq!(area.default.entrance_bgm_id, Some(0x8001_0021));
        assert_eq!(area.default.secondary_bgm_id, Some(0x8001_0023));
        assert_eq!(area.default.secondary_start_status, 2);
        assert_eq!(area.default.switch_bgm_id, Some(0x8001_0008));
        assert_eq!(area.default.switch_bgm2_id, Some(0x8001_0009));
        assert_eq!(area.default.flags, 0);
        assert_eq!(area.default.fade_event, 3);
        assert_eq!(area.scenario_overrides.len(), 1);
        assert_eq!(area.scenario_overrides[0].scenario_index, 2);
        assert_eq!(
            area.scenario_overrides[0].state.primary_bgm_id,
            Some(0x8001_0016)
        );
        assert!(areas
            .iter()
            .any(|definition| definition.area_index == u32::MAX));
    }

    #[test]
    fn extracts_dialogue_voice_order() {
        let source = r#"
            static const u32 scTalkSoundList[] = {
                0x00008850, 0xFFFFFFFF,
                0X80010025,
            };
            static const u32 unrelated[] = { 0xDEADBEEF };
        "#;
        let voices = extract_dialogue_voices(source, "Talk2D2.cpp").unwrap();
        assert_eq!(
            voices
                .iter()
                .map(|voice| voice.sound_id)
                .collect::<Vec<_>>(),
            [0x8850, u32::MAX, 0x8001_0025]
        );
        assert_eq!(voices[2].index, 2);
    }
}
