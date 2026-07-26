use regex::Regex;

use super::{
    braced_body, parse_cpp_string, parse_cpp_u32, split_cpp_initializer_fields,
    ObjectPreviewDefinition, ObjectPreviewTevKColorAlphaOverride,
};

pub(super) struct MarioObjectPreviewSources<'a> {
    pub mario_draw: &'a str,
    pub mario_main_header: &'a str,
    pub mario_main_source: &'a str,
    pub mario_init: &'a str,
    pub application: &'a str,
    pub mar_director_direct: &'a str,
    pub provenance: [&'a str; 7],
}

pub(super) fn extract_mario_object_preview(
    factory_name: &str,
    class_name: &str,
    sources: MarioObjectPreviewSources<'_>,
) -> Result<ObjectPreviewDefinition, String> {
    if factory_name != "Mario" || class_name != "TMario" {
        return Err(format!(
            "registered Mario factory resolved to {factory_name} -> {class_name}, expected Mario -> TMario"
        ));
    }

    let wait_animation_id = extract_wait_animation_id(sources.mario_main_header)?;
    ensure_default_state_has_no_shirt(sources.mario_init)?;
    let hidden_shape_indices = vec![extract_default_hidden_shirt_shape(
        sources.mario_main_source,
    )?];
    let init_model = method_body(sources.mario_draw, "void", "TMario", "initModel")?;
    let (model_path, load_flags) = extract_body_model(init_model)?;
    let (initial_animation_id, initial_rate) = extract_initial_animation(init_model)?;
    if initial_animation_id != wait_animation_id {
        return Err(format!(
            "TMario::initModel starts animation {initial_animation_id:#x}, but ANIM_WAIT is {wait_animation_id:#x}"
        ));
    }

    let set_animation = method_body(sources.mario_draw, "f32", "TMario", "setAnimation")?;
    let playback_factor = extract_motion_playback_factor(set_animation)?;
    let fixed_step_scale = extract_mario_fixed_step_playback_scale(
        sources.mario_main_source,
        sources.mar_director_direct,
        sources.application,
    )?;
    let idle_playback_rate = multiply_ratios(initial_rate, playback_factor)?;
    let (idle_playback_rate_numerator, idle_playback_rate_denominator) =
        multiply_ratios(idle_playback_rate, fixed_step_scale)?;

    ensure_animation_members(set_animation)?;
    let (transform_index, texture_pattern_index) =
        extract_animation_data_row(sources.mario_draw, wait_animation_id)?;
    let animation_name = extract_animation_file_name(sources.mario_draw, transform_index)?;
    let animation_format = extract_animation_path_format(init_model)?;
    let idle_bck_path = expand_single_string_format(&animation_format, &animation_name)?;
    let idle_btp_path =
        extract_texture_pattern_path(sources.mario_draw, set_animation, texture_pattern_index)?;
    let runtime_archive_path = extract_runtime_archive_path(sources.application)?;
    let tev_k_color_alpha_overrides = vec![extract_clean_pollution_alpha_override(
        sources.mario_draw,
        sources.mario_init,
    )?];

    Ok(ObjectPreviewDefinition {
        factory_name: factory_name.to_string(),
        runtime_archive_path,
        model_path,
        load_flags,
        idle_bck_path,
        idle_btp_path,
        idle_playback_rate_numerator,
        idle_playback_rate_denominator,
        hidden_shape_indices,
        tev_k_color_alpha_overrides,
        source_files: sources
            .provenance
            .into_iter()
            .map(ToString::to_string)
            .collect(),
    })
}

fn extract_wait_animation_id(mario_main: &str) -> Result<u32, String> {
    let wait_re = Regex::new(r"\bANIM_WAIT\s*=\s*([^,\r\n}]+)").expect("valid ANIM_WAIT regex");
    let values = wait_re
        .captures_iter(mario_main)
        .map(|captures| {
            parse_cpp_u32(&captures[1])
                .ok_or_else(|| format!("ANIM_WAIT has non-numeric value {:?}", &captures[1]))
        })
        .collect::<Result<Vec<_>, _>>()?;
    match values.as_slice() {
        [value] => Ok(*value),
        _ => Err(format!(
            "MarioMain.hpp contains {} numeric ANIM_WAIT declarations; expected one",
            values.len()
        )),
    }
}

fn ensure_default_state_has_no_shirt(mario_init: &str) -> Result<(), String> {
    let load = method_body(mario_init, "void", "TMario", "load")?;
    let state_re =
        Regex::new(r"\bmState\s*=\s*([^;]+)\s*;").expect("valid Mario state-reset regex");
    let states = state_re.captures_iter(load).collect::<Vec<_>>();
    let [state] = states.as_slice() else {
        return Err(format!(
            "TMario::load contains {} recognizable mState resets; expected one",
            states.len()
        ));
    };
    let state = parse_cpp_u32(&state[1]).ok_or_else(|| {
        format!(
            "TMario::load initializes mState with non-numeric value {:?}",
            &state[1]
        )
    })?;
    if state != 0 {
        return Err(format!(
            "TMario::load initializes mState to {state:#x}; cannot derive the default no-shirt branch"
        ));
    }

    let init_values = method_body(mario_init, "void", "TMario", "initValues")?;
    let enables_shirt =
        Regex::new(r"\bmState\s*\|=\s*MARIO_FLAG_HAS_SHIRT\b").expect("valid shirt-state regex");
    if enables_shirt.is_match(load) || enables_shirt.is_match(init_values) {
        return Err(
            "Mario initialization explicitly enables MARIO_FLAG_HAS_SHIRT; the default no-shirt preview is no longer valid"
                .to_string(),
        );
    }
    Ok(())
}

fn extract_default_hidden_shirt_shape(mario_main: &str) -> Result<u16, String> {
    let perform = method_body(mario_main, "void", "TMario", "perform")?;
    let branch_re = Regex::new(
        r"(?s)if\s*\(\s*checkFlag\s*\(\s*MARIO_FLAG_HAS_SHIRT\s*\)\s*\)\s*\{(.*?)\}\s*else\s*\{(.*?)\}",
    )
    .expect("valid Mario shirt-visibility regex");
    let branches = branch_re.captures_iter(perform).collect::<Vec<_>>();
    let [branches] = branches.as_slice() else {
        return Err(format!(
            "TMario::perform contains {} recognizable MARIO_FLAG_HAS_SHIRT visibility branches; expected one",
            branches.len()
        ));
    };

    let shirt_shape = extract_shape_flag_index(&branches[1], "offFlag")?;
    let no_shirt_shape = extract_shape_flag_index(&branches[2], "onFlag")?;
    if shirt_shape != no_shirt_shape {
        return Err(format!(
            "TMario::perform shows shirt shape {shirt_shape} but hides shape {no_shirt_shape}"
        ));
    }
    Ok(no_shirt_shape)
}

fn extract_shape_flag_index(branch: &str, operation: &str) -> Result<u16, String> {
    let shape_re = Regex::new(&format!(
        r"(?s)mShapeNodePointer\s*\[\s*([A-Za-z0-9_xX]+)\s*\]\s*;.*?\bshape\s*->\s*{}\s*\(\s*1\s*\)",
        regex::escape(operation)
    ))
    .expect("valid generated Mario shape-visibility regex");
    let shapes = shape_re.captures_iter(branch).collect::<Vec<_>>();
    let [shape] = shapes.as_slice() else {
        return Err(format!(
            "Mario shirt branch contains {} recognizable shape->{operation}(1) operations; expected one",
            shapes.len()
        ));
    };
    let index = parse_cpp_u32(&shape[1]).ok_or_else(|| {
        format!(
            "Mario shirt branch has non-numeric shape index {:?}",
            &shape[1]
        )
    })?;
    u16::try_from(index)
        .map_err(|_| format!("Mario shirt shape index {index} does not fit the schema"))
}

fn extract_clean_pollution_alpha_override(
    mario_draw: &str,
    mario_init: &str,
) -> Result<ObjectPreviewTevKColorAlphaOverride, String> {
    let add_dirty = method_body(mario_draw, "void", "TMario", "addDirty")?;
    let override_re = Regex::new(
        r"(?s)mBodyModelData->getMaterialNum\s*\(\s*\).*?getTevKColor\s*\(\s*([A-Za-z0-9_xX]+)\s*\).*?->color\.a\s*=\s*([A-Za-z_][A-Za-z0-9_]*)\s*;",
    )
    .expect("valid Mario pollution-alpha regex");
    let overrides = override_re.captures_iter(add_dirty).collect::<Vec<_>>();
    let [alpha_override] = overrides.as_slice() else {
        return Err(format!(
            "TMario::addDirty contains {} recognizable body TEV konst-alpha assignments; expected one",
            overrides.len()
        ));
    };
    let register = parse_cpp_u32(&alpha_override[1]).ok_or_else(|| {
        format!(
            "TMario::addDirty uses non-numeric TEV konst register {:?}",
            &alpha_override[1]
        )
    })?;
    let register = u8::try_from(register)
        .map_err(|_| format!("Mario TEV konst register {register} does not fit the schema"))?;

    let dirty_field = &alpha_override[2];
    let init_values = method_body(mario_init, "void", "TMario", "initValues")?;
    let initial_re = Regex::new(&format!(
        r"\b{}\s*=\s*([^;]+)\s*;",
        regex::escape(dirty_field)
    ))
    .expect("valid generated Mario clean-pollution regex");
    let initial_values = initial_re.captures_iter(init_values).collect::<Vec<_>>();
    let [initial] = initial_values.as_slice() else {
        return Err(format!(
            "TMario::initValues contains {} initializers for pollution field {dirty_field}; expected one",
            initial_values.len()
        ));
    };
    let alpha = parse_u8_decimal_literal(&initial[1]).ok_or_else(|| {
        format!(
            "TMario::initValues initializes {dirty_field} with non-integral alpha {:?}",
            &initial[1]
        )
    })?;

    Ok(ObjectPreviewTevKColorAlphaOverride { register, alpha })
}

fn parse_u8_decimal_literal(value: &str) -> Option<u8> {
    let value = value.trim().trim_end_matches(['f', 'F']);
    let (whole, fractional) = value.split_once('.').unwrap_or((value, ""));
    if whole.is_empty()
        || !whole.bytes().all(|byte| byte.is_ascii_digit())
        || !fractional.bytes().all(|byte| byte == b'0')
    {
        return None;
    }
    whole.parse().ok()
}

fn extract_body_model(init_model: &str) -> Result<(String, u32), String> {
    let model_re = Regex::new(
        r#"(?s)mBodyModelData\s*=\s*J3DModelLoaderDataBase::load\s*\(\s*JKRFileLoader::getGlbResource\s*\(\s*"([^"]+\.bmd)"\s*\)\s*,\s*([A-Za-z0-9_xX]+)\s*\)"#,
    )
    .expect("valid Mario body-model regex");
    let matches = model_re.captures_iter(init_model).collect::<Vec<_>>();
    let [model] = matches.as_slice() else {
        return Err(format!(
            "TMario::initModel contains {} recognizable body-model loads; expected one",
            matches.len()
        ));
    };
    let load_flags = parse_cpp_u32(&model[2]).ok_or_else(|| {
        format!(
            "Mario body model has non-numeric loader flags {:?}",
            &model[2]
        )
    })?;
    Ok((model[1].to_string(), load_flags))
}

fn extract_initial_animation(init_model: &str) -> Result<(u32, (u32, u32)), String> {
    let initial_re = Regex::new(
        r"(?s)mModel\s*=\s*modelMario\s*;\s*setAnimation\s*\(\s*([^,]+)\s*,\s*([^)]+)\s*\)\s*;",
    )
    .expect("valid Mario initial-animation regex");
    let matches = initial_re.captures_iter(init_model).collect::<Vec<_>>();
    let [initial] = matches.as_slice() else {
        return Err(format!(
            "TMario::initModel contains {} recognizable initial animation calls; expected one",
            matches.len()
        ));
    };
    let animation_id = parse_cpp_u32(&initial[1]).ok_or_else(|| {
        format!(
            "TMario::initModel has non-numeric initial animation {:?}",
            &initial[1]
        )
    })?;
    let rate = parse_positive_decimal_ratio(&initial[2])?;
    Ok((animation_id, rate))
}

fn extract_motion_playback_factor(set_animation: &str) -> Result<(u32, u32), String> {
    let motion_rate_re = Regex::new(
        r"getMotionFrameCtrl\s*\(\s*\)\.setRate\s*\(\s*param_2\s*\*\s*([0-9]+(?:\.[0-9]+)?[fF]?)\s*\)",
    )
    .expect("valid Mario motion-rate regex");
    let motion_rates = motion_rate_re
        .captures_iter(set_animation)
        .collect::<Vec<_>>();
    let [motion_rate] = motion_rates.as_slice() else {
        return Err(format!(
            "TMario::setAnimation contains {} recognizable motion playback factors; expected one",
            motion_rates.len()
        ));
    };
    let motion_rate = parse_positive_decimal_ratio(&motion_rate[1])?;

    let texture_rate_re = Regex::new(
        r"mModel->getFrameCtrl\s*\(\s*2\s*\)\.setRate\s*\(\s*param_2\s*\*\s*([0-9]+(?:\.[0-9]+)?[fF]?)\s*\)",
    )
    .expect("valid Mario texture-pattern rate regex");
    let texture_rates = texture_rate_re
        .captures_iter(set_animation)
        .collect::<Vec<_>>();
    let [texture_rate] = texture_rates.as_slice() else {
        return Err(format!(
            "TMario::setAnimation contains {} recognizable texture-pattern playback factors; expected one",
            texture_rates.len()
        ));
    };
    let texture_rate = parse_positive_decimal_ratio(&texture_rate[1])?;
    if motion_rate != texture_rate {
        return Err(
            "TMario::setAnimation applies different motion and texture-pattern playback factors"
                .to_string(),
        );
    }
    Ok(motion_rate)
}

fn extract_mario_fixed_step_playback_scale(
    mario_main: &str,
    mar_director_direct: &str,
    application: &str,
) -> Result<(u32, u32), String> {
    let perform = method_body(mario_main, "void", "TMario", "perform")?;
    let movement_flag_re = Regex::new(r"\bu32\s+doMovement\s*=\s*flags\s*&\s*1\s*;")
        .expect("valid Mario movement-flag regex");
    if !movement_flag_re.is_match(perform) {
        return Err(
            "TMario::perform no longer derives doMovement from the fixed-step movement flag"
                .to_string(),
        );
    }
    let animation_update_re = Regex::new(r"\bcalcAnim\s*\(\s*2\s*,\s*gfx\s*\)\s*;")
        .expect("valid Mario animation-update regex");
    if animation_update_re.find_iter(perform).count() != 1 {
        return Err(
            "TMario::perform no longer contains exactly one fixed-step calcAnim update".to_string(),
        );
    }

    let scheduler_rate_re = Regex::new(
        r"\bint\s+vsyncRate\s*=\s*([0-9]+)\s*/\s*\(int\)\s*SMSGetVSyncTimesPerSec\s*\(\s*\)\s*;",
    )
    .expect("valid Mario scheduler-rate regex");
    let scheduler_rates = scheduler_rate_re
        .captures_iter(mar_director_direct)
        .collect::<Vec<_>>();
    let [scheduler_rate] = scheduler_rates.as_slice() else {
        return Err(format!(
            "TMarDirector::direct contains {} recognizable fixed-step clock rates; expected one",
            scheduler_rates.len()
        ));
    };
    let scheduler_units_per_second = parse_cpp_u32(&scheduler_rate[1]).ok_or_else(|| {
        format!(
            "TMarDirector::direct has non-numeric fixed-step clock rate {:?}",
            &scheduler_rate[1]
        )
    })?;
    let scheduler_accumulator_re = Regex::new(r"\bunk54\s*\+=\s*vsyncRate\s*;")
        .expect("valid Mario scheduler-accumulator regex");
    if scheduler_accumulator_re
        .find_iter(mar_director_direct)
        .count()
        != 1
    {
        return Err(
            "TMarDirector::direct no longer advances its fixed-step accumulator exactly once"
                .to_string(),
        );
    }
    let scheduler_step_re =
        Regex::new(r"\bunk54\s*-=\s*([0-9]+)\s*;").expect("valid Mario scheduler-step regex");
    let scheduler_steps = scheduler_step_re
        .captures_iter(mar_director_direct)
        .collect::<Vec<_>>();
    let [scheduler_step] = scheduler_steps.as_slice() else {
        return Err(format!(
            "TMarDirector::direct contains {} recognizable fixed-step decrements; expected one",
            scheduler_steps.len()
        ));
    };
    let scheduler_step = parse_cpp_u32(&scheduler_step[1]).ok_or_else(|| {
        format!(
            "TMarDirector::direct has non-numeric fixed-step decrement {:?}",
            &scheduler_step[1]
        )
    })?;

    let authored_rate_re = Regex::new(
        r"\bSMSGetAnmFrameRate\s*\(\s*\)\s*\{\s*return\s*([0-9]+(?:\.[0-9]+)?[fF]?)\s*/\s*SMSGetVSyncTimesPerSec\s*\(\s*\)\s*;\s*\}",
    )
    .expect("valid authored animation-rate regex");
    let authored_rates = authored_rate_re
        .captures_iter(application)
        .collect::<Vec<_>>();
    let [authored_rate] = authored_rates.as_slice() else {
        return Err(format!(
            "Application.cpp contains {} recognizable authored animation clock rates; expected one",
            authored_rates.len()
        ));
    };
    let authored_frames_per_second = parse_positive_decimal_ratio(&authored_rate[1])?;

    let numerator = u64::from(scheduler_units_per_second)
        .checked_mul(u64::from(authored_frames_per_second.1))
        .ok_or_else(|| "Mario fixed-step playback numerator overflowed".to_string())?;
    let denominator = u64::from(scheduler_step)
        .checked_mul(u64::from(authored_frames_per_second.0))
        .ok_or_else(|| "Mario fixed-step playback denominator overflowed".to_string())?;
    reduce_positive_ratio(numerator, denominator)
}

fn ensure_animation_members(set_animation: &str) -> Result<(), String> {
    let transform_re = Regex::new(r"gMarioAnimeData\s*\[\s*param_1\s*\]\.unk0")
        .expect("valid Mario transform-member regex");
    if !transform_re.is_match(set_animation) {
        return Err(
            "TMario::setAnimation no longer selects its transform through gMarioAnimeData[param_1].unk0"
                .to_string(),
        );
    }
    let texture_re = Regex::new(r"gMarioAnimeData\s*\[\s*param_1\s*\]\.unk4")
        .expect("valid Mario texture-pattern member regex");
    if !texture_re.is_match(set_animation) {
        return Err(
            "TMario::setAnimation no longer selects its texture pattern through gMarioAnimeData[param_1].unk4"
                .to_string(),
        );
    }
    Ok(())
}

fn extract_animation_data_row(mario_draw: &str, animation_id: u32) -> Result<(u32, u32), String> {
    let (declared_count, body) = array_body(mario_draw, "gMarioAnimeData")?;
    let rows = initializer_rows(body);
    ensure_array_count("gMarioAnimeData", declared_count, rows.len())?;
    let row = rows
        .get(usize::try_from(animation_id).map_err(|_| {
            format!("ANIM_WAIT value {animation_id:#x} does not fit a host array index")
        })?)
        .ok_or_else(|| {
            format!(
                "ANIM_WAIT value {animation_id:#x} is outside gMarioAnimeData's {} rows",
                rows.len()
            )
        })?;
    let fields = split_cpp_initializer_fields(row);
    if fields.len() != 6 {
        return Err(format!(
            "gMarioAnimeData[{animation_id:#x}] has {} fields; expected six",
            fields.len()
        ));
    }
    let transform_index = parse_cpp_u32(fields[0]).ok_or_else(|| {
        format!(
            "gMarioAnimeData[{animation_id:#x}] has non-numeric transform index {:?}",
            fields[0]
        )
    })?;
    let texture_pattern_index = parse_cpp_u32(fields[2]).ok_or_else(|| {
        format!(
            "gMarioAnimeData[{animation_id:#x}] has non-numeric texture-pattern index {:?}",
            fields[2]
        )
    })?;
    Ok((transform_index, texture_pattern_index))
}

fn extract_animation_file_name(mario_draw: &str, transform_index: u32) -> Result<String, String> {
    let (declared_count, body) = array_body(mario_draw, "marioAnimeFiles")?;
    let rows = initializer_rows(body);
    ensure_array_count("marioAnimeFiles", declared_count, rows.len())?;
    let row = rows
        .get(usize::try_from(transform_index).map_err(|_| {
            format!("Mario transform index {transform_index:#x} does not fit a host array index")
        })?)
        .ok_or_else(|| {
            format!(
                "Mario transform index {transform_index:#x} is outside marioAnimeFiles' {} rows",
                rows.len()
            )
        })?;
    let fields = split_cpp_initializer_fields(row);
    if fields.len() != 2 {
        return Err(format!(
            "marioAnimeFiles[{transform_index:#x}] has {} fields; expected two",
            fields.len()
        ));
    }
    parse_cpp_string(fields[1]).ok_or_else(|| {
        format!(
            "marioAnimeFiles[{transform_index:#x}] has no string animation name in {:?}",
            fields[1]
        )
    })
}

fn extract_animation_path_format(init_model: &str) -> Result<String, String> {
    let format_re = Regex::new(
        r#"(?s)snprintf\s*\([^;]*?"([^"]*%s[^"]*\.bck)"\s*,\s*marioAnimeFiles\s*\[[^\]]+\]\.unk4\s*\)\s*;"#,
    )
    .expect("valid Mario BCK format regex");
    let formats = format_re
        .captures_iter(init_model)
        .map(|captures| captures[1].to_string())
        .collect::<Vec<_>>();
    match formats.as_slice() {
        [format] => Ok(format.clone()),
        _ => Err(format!(
            "TMario::initModel contains {} recognizable marioAnimeFiles BCK formats; expected one",
            formats.len()
        )),
    }
}

fn extract_texture_pattern_path(
    mario_draw: &str,
    set_animation: &str,
    texture_pattern_index: u32,
) -> Result<Option<String>, String> {
    let assignment_re = Regex::new(
        r"s8\s+([A-Za-z_][A-Za-z0-9_]*)\s*=\s*gMarioAnimeData\s*\[\s*param_1\s*\]\.unk4\s*;",
    )
    .expect("valid Mario texture-pattern assignment regex");
    let assignments = assignment_re
        .captures_iter(set_animation)
        .collect::<Vec<_>>();
    let [assignment] = assignments.as_slice() else {
        return Err(format!(
            "TMario::setAnimation contains {} recognizable texture-pattern index assignments; expected one",
            assignments.len()
        ));
    };
    let variable = &assignment[1];
    let selection_re = Regex::new(&format!(
        r"(?s)if\s*\(\s*{}\s*<\s*([A-Za-z0-9_xX]+)\s*\).*?changeAnmTexPattern\s*\(\s*0\s*,\s*{}\s*\)",
        regex::escape(variable),
        regex::escape(variable)
    ))
    .expect("valid generated Mario texture-pattern selection regex");
    let selections = selection_re
        .captures_iter(set_animation)
        .collect::<Vec<_>>();
    let [selection] = selections.as_slice() else {
        return Err(format!(
            "TMario::setAnimation contains {} recognizable texture-pattern range checks; expected one",
            selections.len()
        ));
    };
    let runtime_count = parse_cpp_u32(&selection[1]).ok_or_else(|| {
        format!(
            "TMario::setAnimation has non-numeric texture-pattern count {:?}",
            &selection[1]
        )
    })?;

    let (declared_count, body) = array_body(mario_draw, "marioAnimeTexPatternFilenames")?;
    if runtime_count != declared_count {
        return Err(format!(
            "TMario::setAnimation accepts {runtime_count} texture patterns, but marioAnimeTexPatternFilenames declares {declared_count}"
        ));
    }
    let string_re =
        Regex::new(r#""([^"\\]*(?:\\.[^"\\]*)*)""#).expect("valid C++ string-literal regex");
    let paths = string_re
        .captures_iter(body)
        .map(|captures| captures[1].to_string())
        .collect::<Vec<_>>();
    ensure_array_count("marioAnimeTexPatternFilenames", declared_count, paths.len())?;

    if texture_pattern_index >= runtime_count {
        return Ok(None);
    }
    paths
        .get(usize::try_from(texture_pattern_index).map_err(|_| {
            format!(
                "Mario texture-pattern index {texture_pattern_index:#x} does not fit a host array index"
            )
        })?)
        .cloned()
        .map(Some)
        .ok_or_else(|| {
            format!(
                "Mario texture-pattern index {texture_pattern_index:#x} is outside marioAnimeTexPatternFilenames"
            )
        })
}

fn extract_runtime_archive_path(application: &str) -> Result<String, String> {
    let archive_re = Regex::new(r#"(?s)\barcBufMario\s*=\s*SMSLoadArchive\s*\(\s*"([^"]+\.arc)""#)
        .expect("valid Mario archive regex");
    let archives = archive_re
        .captures_iter(application)
        .map(|captures| captures[1].to_string())
        .collect::<Vec<_>>();
    match archives.as_slice() {
        [archive] => Ok(archive.clone()),
        _ => Err(format!(
            "Application.cpp contains {} recognizable Mario archive mounts; expected one",
            archives.len()
        )),
    }
}

fn array_body<'a>(text: &'a str, array_name: &str) -> Result<(u32, &'a str), String> {
    let declaration_re = Regex::new(&format!(
        r"\b{}\s*\[\s*([0-9]+|0[xX][0-9A-Fa-f]+)\s*\]\s*=\s*\{{",
        regex::escape(array_name)
    ))
    .expect("valid generated array declaration regex");
    let declarations = declaration_re.captures_iter(text).collect::<Vec<_>>();
    let [declaration] = declarations.as_slice() else {
        return Err(format!(
            "MarioDraw.cpp contains {} declarations of {array_name}; expected one",
            declarations.len()
        ));
    };
    let count = parse_cpp_u32(&declaration[1])
        .ok_or_else(|| format!("{array_name} has a non-numeric declared count"))?;
    let whole = declaration
        .get(0)
        .ok_or_else(|| format!("{array_name} declaration has no complete match"))?;
    let body = braced_body(text, whole.end() - 1)
        .ok_or_else(|| format!("{array_name} has an unterminated initializer"))?;
    Ok((count, body))
}

fn initializer_rows(body: &str) -> Vec<&str> {
    let row_re = Regex::new(r"\{([^{}]*)\}").expect("valid initializer-row regex");
    row_re
        .captures_iter(body)
        .filter_map(|captures| captures.get(1).map(|row| row.as_str()))
        .collect()
}

fn ensure_array_count(name: &str, declared: u32, actual: usize) -> Result<(), String> {
    if usize::try_from(declared).ok() == Some(actual) {
        Ok(())
    } else {
        Err(format!(
            "{name} declares {declared} entries but contains {actual}"
        ))
    }
}

fn expand_single_string_format(format: &str, value: &str) -> Result<String, String> {
    if format.matches("%s").count() != 1 {
        return Err(format!(
            "Mario BCK format {format:?} does not contain exactly one %s placeholder"
        ));
    }
    Ok(format.replacen("%s", value, 1))
}

fn parse_positive_decimal_ratio(value: &str) -> Result<(u32, u32), String> {
    let value = value.trim().trim_end_matches(['f', 'F']);
    let (whole, fractional) = value.split_once('.').unwrap_or((value, ""));
    if whole.is_empty()
        || !whole.bytes().all(|byte| byte.is_ascii_digit())
        || !fractional.bytes().all(|byte| byte.is_ascii_digit())
    {
        return Err(format!(
            "animation playback value {value:?} is not a positive decimal literal"
        ));
    }
    let denominator = 10_u64
        .checked_pow(
            u32::try_from(fractional.len())
                .map_err(|_| format!("animation playback value {value:?} is too precise"))?,
        )
        .ok_or_else(|| format!("animation playback value {value:?} is too precise"))?;
    let whole = whole
        .parse::<u64>()
        .map_err(|_| format!("animation playback value {value:?} is too large"))?;
    let fractional = if fractional.is_empty() {
        0
    } else {
        fractional
            .parse::<u64>()
            .map_err(|_| format!("animation playback value {value:?} is too large"))?
    };
    let numerator = whole
        .checked_mul(denominator)
        .and_then(|value| value.checked_add(fractional))
        .ok_or_else(|| format!("animation playback value {value:?} is too large"))?;
    reduce_positive_ratio(numerator, denominator)
}

fn multiply_ratios(left: (u32, u32), right: (u32, u32)) -> Result<(u32, u32), String> {
    let numerator = u64::from(left.0)
        .checked_mul(u64::from(right.0))
        .ok_or_else(|| "Mario idle playback numerator overflowed".to_string())?;
    let denominator = u64::from(left.1)
        .checked_mul(u64::from(right.1))
        .ok_or_else(|| "Mario idle playback denominator overflowed".to_string())?;
    reduce_positive_ratio(numerator, denominator)
}

fn reduce_positive_ratio(numerator: u64, denominator: u64) -> Result<(u32, u32), String> {
    if numerator == 0 || denominator == 0 {
        return Err("Mario idle playback ratio must be positive".to_string());
    }
    let divisor = greatest_common_divisor(numerator, denominator);
    let numerator = u32::try_from(numerator / divisor)
        .map_err(|_| "Mario idle playback numerator does not fit u32".to_string())?;
    let denominator = u32::try_from(denominator / divisor)
        .map_err(|_| "Mario idle playback denominator does not fit u32".to_string())?;
    Ok((numerator, denominator))
}

fn greatest_common_divisor(mut left: u64, mut right: u64) -> u64 {
    while right != 0 {
        let remainder = left % right;
        left = right;
        right = remainder;
    }
    left
}

fn method_body<'a>(
    text: &'a str,
    return_type: &str,
    class_name: &str,
    method_name: &str,
) -> Result<&'a str, String> {
    let method_re = Regex::new(&format!(
        r"\b{}\s+{}::{}\s*\([^)]*\)\s*\{{",
        regex::escape(return_type),
        regex::escape(class_name),
        regex::escape(method_name)
    ))
    .expect("valid generated Mario method regex");
    let methods = method_re.find_iter(text).collect::<Vec<_>>();
    let [method] = methods.as_slice() else {
        return Err(format!(
            "MarioDraw.cpp contains {} definitions of {class_name}::{method_name}; expected one",
            methods.len()
        ));
    };
    braced_body(text, method.end() - 1)
        .ok_or_else(|| format!("{class_name}::{method_name} has an unterminated body"))
}

#[cfg(test)]
mod tests {
    use super::*;

    const MARIO_MAIN_HEADER: &str = r#"
        class TMario {
        public:
            enum {
                ANIM_JUMP = 0x1,
                ANIM_WAIT = 0x2,
            };
        };
    "#;

    const MARIO_MAIN_SOURCE: &str = r#"
        void TMario::perform(u32 flags, JDrama::TGraphics* gfx) {
            u32 doMovement = flags & 1;
            if (checkFlag(MARIO_FLAG_HAS_SHIRT)) {
                J3DShape* shape =
                    mModel->unk8->mModelData->mShapeNodePointer[10];
                shape->offFlag(1);
            } else {
                J3DShape* shape =
                    mModel->unk8->mModelData->mShapeNodePointer[10];
                shape->onFlag(1);
            }
            if (doMovement && unk14E <= 0) {
                calcAnim(2, gfx);
            }
        }
    "#;

    const MARIO_INIT: &str = r#"
        void TMario::initValues() {
            mHealth = mDeParams.mHpMax.get();
            unk134 = 0.0f;
        }

        void TMario::load(JSUMemoryInputStream& stream) {
            JDrama::TActor::load(stream);
            mState = 0;
            if (flags & 1) {
                mState &= ~MARIO_FLAG_HAS_FLUDD;
            } else {
                mState |= MARIO_FLAG_HAS_FLUDD;
            }
            initValues();
        }
    "#;

    const MARIO_DRAW: &str = r#"
        static unkTMarioAnimeFilesStruct marioAnimeFiles[3] = {
            { 0x00000001, "jump" },
            { 0x00000000, "fixture_wait" },
            { 0x00000000, "unused" },
        };

        TMarioAnimeData gMarioAnimeData[3] = {
            { 0x0000, 0x00C8, 0x02, 0x01, 0x04, 0x16 },
            { 0x0000, 0x00C8, 0x02, 0x01, 0x04, 0x16 },
            { 0x0001, 0x0044, 0x00, 0x00, 0x06, 0x16 },
        };

        static char* marioAnimeTexPatternFilenames[2] = {
            "/mario/btp/fixture_blink.btp",
            "/mario/btp/fixture_other.btp",
        };

        f32 TMario::setAnimation(int param_1, f32 param_2) {
            mModel->changeMtxCalcSIAnmBQAnmTransform(
                0, 0, gMarioAnimeData[param_1].unk0);
            s8 texture = gMarioAnimeData[param_1].unk4;
            if (texture < 0x2) {
                mModel->changeAnmTexPattern(0, texture);
            }
            getMotionFrameCtrl().setRate(param_2 * 0.5f);
            mModel->getFrameCtrl(2).setRate(param_2 * 0.5f);
            return 0.0f;
        }

        void TMario::initModel() {
            mBodyModelData = J3DModelLoaderDataBase::load(
                JKRFileLoader::getGlbResource("/mario/bmd/fixture_body.bmd"),
                0x10100000);
            char buffer[0x10C];
            for (int i = 0; i < 3; ++i) {
                snprintf(buffer, 0xff, "/mario/bck/ma_%s.bck",
                         marioAnimeFiles[i].unk4);
                loadAnm(&animations[i], buffer);
            }
            mModel = modelMario;
            setAnimation(0x2, 1.0f);
        }

        void TMario::addDirty() {
            for (u16 i = 0; i < mBodyModelData->getMaterialNum(); ++i) {
                J3DGXColor* konstColor =
                    mBodyModelData->getMaterialNodePointer(i)
                        ->getTevBlock()
                        ->getTevKColor(0);
                konstColor->color.a = unk134;
            }
        }
    "#;

    const APPLICATION: &str = r#"
        f32 SMSGetAnmFrameRate() {
            return 60.0f / SMSGetVSyncTimesPerSec();
        }

        void* TApplication::setupThreadFuncLogo() {
            arcBufMario =
                SMSLoadArchive("/data/mario.arc", nullptr, 0, JKRGetRootHeap());
            return nullptr;
        }
    "#;

    const MAR_DIRECTOR_DIRECT: &str = r#"
        int TMarDirector::direct() {
            int vsyncRate = 600 / (int)SMSGetVSyncTimesPerSec();
            unk54 += vsyncRate;
            for (;;) {
                unk54 -= 5;
                if (unk54 < 5)
                    unk4C |= 0x4000;
            }
        }
    "#;

    const SOURCES: [&str; 7] = [
        "src/System/MarNameRefGen.cpp",
        "src/Player/MarioDraw.cpp",
        "include/Player/MarioMain.hpp",
        "src/Player/MarioMain.cpp",
        "src/Player/MarioInit.cpp",
        "src/System/Application.cpp",
        "src/System/MarDirectorDirect.cpp",
    ];

    fn fixture_sources<'a>(
        mario_draw: &'a str,
        mario_main_source: &'a str,
        mario_init: &'a str,
    ) -> MarioObjectPreviewSources<'a> {
        MarioObjectPreviewSources {
            mario_draw,
            mario_main_header: MARIO_MAIN_HEADER,
            mario_main_source,
            mario_init,
            application: APPLICATION,
            mar_director_direct: MAR_DIRECTOR_DIRECT,
            provenance: SOURCES,
        }
    }

    #[test]
    fn extracts_linked_mario_idle_preview_without_a_resource_name_table() {
        let definition = extract_mario_object_preview(
            "Mario",
            "TMario",
            fixture_sources(MARIO_DRAW, MARIO_MAIN_SOURCE, MARIO_INIT),
        )
        .expect("extract Mario preview");

        assert_eq!(definition.factory_name, "Mario");
        assert_eq!(definition.runtime_archive_path, "/data/mario.arc");
        assert_eq!(definition.model_path, "/mario/bmd/fixture_body.bmd");
        assert_eq!(definition.load_flags, 0x1010_0000);
        assert_eq!(definition.idle_bck_path, "/mario/bck/ma_fixture_wait.bck");
        assert_eq!(
            definition.idle_btp_path.as_deref(),
            Some("/mario/btp/fixture_blink.btp")
        );
        assert_eq!(definition.idle_playback_rate_numerator, 1);
        assert_eq!(definition.idle_playback_rate_denominator, 1);
        assert_eq!(definition.hidden_shape_indices, [10]);
        assert_eq!(
            definition.tev_k_color_alpha_overrides,
            [ObjectPreviewTevKColorAlphaOverride {
                register: 0,
                alpha: 0,
            }]
        );
        assert_eq!(definition.source_files, SOURCES);
    }

    #[test]
    fn omits_texture_pattern_when_the_animation_row_uses_the_runtime_sentinel() {
        let mario_draw = MARIO_DRAW.replace(
            "{ 0x0001, 0x0044, 0x00, 0x00, 0x06, 0x16 }",
            "{ 0x0001, 0x0044, 0x02, 0x00, 0x06, 0x16 }",
        );
        let definition = extract_mario_object_preview(
            "Mario",
            "TMario",
            fixture_sources(&mario_draw, MARIO_MAIN_SOURCE, MARIO_INIT),
        )
        .expect("extract Mario preview without a BTP");

        assert_eq!(definition.idle_btp_path, None);
    }

    #[test]
    fn rejects_an_initial_animation_that_is_not_anim_wait() {
        let mario_draw = MARIO_DRAW.replace("setAnimation(0x2, 1.0f)", "setAnimation(0x1, 1.0f)");
        let error = extract_mario_object_preview(
            "Mario",
            "TMario",
            fixture_sources(&mario_draw, MARIO_MAIN_SOURCE, MARIO_INIT),
        )
        .unwrap_err();

        assert!(error.contains("but ANIM_WAIT is"));
    }

    #[test]
    fn rejects_mismatched_motion_and_texture_pattern_playback_rates() {
        let mario_draw = MARIO_DRAW.replace(
            "mModel->getFrameCtrl(2).setRate(param_2 * 0.5f)",
            "mModel->getFrameCtrl(2).setRate(param_2 * 0.25f)",
        );
        let error = extract_mario_object_preview(
            "Mario",
            "TMario",
            fixture_sources(&mario_draw, MARIO_MAIN_SOURCE, MARIO_INIT),
        )
        .unwrap_err();

        assert!(error.contains("different motion and texture-pattern playback factors"));
    }

    #[test]
    fn rejects_a_default_shirt_state_or_reversed_shape_visibility() {
        let mario_init = MARIO_INIT.replace("mState = 0;", "mState = 1;");
        let error = extract_mario_object_preview(
            "Mario",
            "TMario",
            fixture_sources(MARIO_DRAW, MARIO_MAIN_SOURCE, &mario_init),
        )
        .unwrap_err();
        assert!(error.contains("default no-shirt branch"));

        let mario_main = MARIO_MAIN_SOURCE.replacen("shape->onFlag(1);", "shape->offFlag(1);", 1);
        let error = extract_mario_object_preview(
            "Mario",
            "TMario",
            fixture_sources(MARIO_DRAW, &mario_main, MARIO_INIT),
        )
        .unwrap_err();
        assert!(error.contains("shape->onFlag(1)"));
    }

    #[test]
    fn rejects_mismatched_shirt_shapes() {
        let mario_main = MARIO_MAIN_SOURCE.replacen(
            "mShapeNodePointer[10];\n                shape->onFlag",
            "mShapeNodePointer[11];\n                shape->onFlag",
            1,
        );
        let error = extract_mario_object_preview(
            "Mario",
            "TMario",
            fixture_sources(MARIO_DRAW, &mario_main, MARIO_INIT),
        )
        .unwrap_err();

        assert!(error.contains("shows shirt shape 10 but hides shape 11"));
    }

    #[test]
    fn derives_the_pollution_register_and_clean_initial_alpha() {
        let mario_draw = MARIO_DRAW.replace("getTevKColor(0)", "getTevKColor(2)");
        let mario_init = MARIO_INIT.replace("unk134 = 0.0f;", "unk134 = 7.0f;");
        let definition = extract_mario_object_preview(
            "Mario",
            "TMario",
            fixture_sources(&mario_draw, MARIO_MAIN_SOURCE, &mario_init),
        )
        .expect("derive nonzero fixture alpha");

        assert_eq!(
            definition.tev_k_color_alpha_overrides,
            [ObjectPreviewTevKColorAlphaOverride {
                register: 2,
                alpha: 7,
            }]
        );
    }
}
