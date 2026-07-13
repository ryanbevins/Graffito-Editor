use super::*;

pub(super) fn push_object_preview_materials(
    materials: &mut Vec<J3dMaterial>,
    cached: &CachedObjectModelPreview,
    object: &SceneObject,
) -> usize {
    let nozzle_tev_reg1 = nozzle_box_tev_reg1_color(object);
    let has_npc_colors = monte_material_colors(object).is_some()
        || mare_body_color(object).is_some()
        || npc_pollution_k_color(object).is_some();
    if nozzle_tev_reg1.is_none() && !has_npc_colors {
        return cached.material_base;
    }
    let source_end = cached.material_base + cached.preview.materials.len();
    let source_materials = materials[cached.material_base..source_end].to_vec();
    let material_base = materials.len();
    for mut material in source_materials {
        material.material_index = materials.len();
        if let Some(tev_reg1) = nozzle_tev_reg1 {
            material.tev_colors[1] = tev_reg1;
        }
        if !material.name.eq_ignore_ascii_case("_eye_mat") {
            if let Some(color) = npc_pollution_k_color(object) {
                material.tev_k_colors[0] = color;
            }
        }
        apply_monte_material_color(&mut material, object);
        apply_mare_material_color(&mut material, object);
        materials.push(material);
    }
    material_base
}

pub(super) fn apply_mare_material_color(material: &mut J3dMaterial, object: &SceneObject) {
    if material.name.eq_ignore_ascii_case("_body") {
        if let Some(color) = mare_body_color(object) {
            material.tev_colors[0] = color;
        }
    }
}

pub(super) fn mare_body_color(object: &SceneObject) -> Option<[i16; 4]> {
    let factory = object.factory_name.to_ascii_lowercase();
    let colors = if factory.starts_with("npcmarem") {
        &MARE_M_BODY_COLORS
    } else if factory.starts_with("npcmarew") {
        &MARE_W_BODY_COLORS
    } else {
        return None;
    };
    let body_index = npc_color_index(object, "npc_body_color_index")?;
    colors.get(body_index).copied()
}

pub(super) fn npc_pollution_k_color(object: &SceneObject) -> Option<[u8; 4]> {
    if !npc_has_initial_pollution_color(object) {
        return None;
    }
    let amount = object
        .raw_params
        .get("npc_pollution_amount")?
        .parse::<i32>()
        .ok()?
        .clamp(0, 255) as u8;
    Some([255, 255, 255, amount])
}

pub(super) fn npc_has_initial_pollution_color(object: &SceneObject) -> bool {
    let factory = object.factory_name.to_ascii_lowercase();
    // TBaseNPC::getPtrInitPollutionColor is intentionally broader than
    // isPollutionNpc: normal and special Monte/Mare actors still install K0,
    // usually with alpha zero, so the authored dirty-layer default is hidden.
    factory.starts_with("npcmonte") || factory.starts_with("npcmare") || factory == "npckinopio"
}

#[derive(Clone, Copy)]
pub(super) struct MonteMaterialColors {
    pub(super) body_reg0: Option<[i16; 4]>,
    pub(super) cloth_reg0: Option<[i16; 4]>,
    pub(super) cloth_reg1_reg2: Option<[[i16; 4]; 2]>,
}

pub(super) fn apply_monte_material_color(material: &mut J3dMaterial, object: &SceneObject) {
    let Some(colors) = monte_material_colors(object) else {
        return;
    };
    if material.name.eq_ignore_ascii_case("_hand_mat") {
        if let Some(color) = colors.body_reg0 {
            material.tev_colors[0] = color;
        }
    } else if material.name.eq_ignore_ascii_case("_fuku_mat") {
        if let Some(color) = colors.cloth_reg0 {
            material.tev_colors[0] = color;
        }
        if let Some([reg1, reg2]) = colors.cloth_reg1_reg2 {
            material.tev_colors[1] = reg1;
            material.tev_colors[2] = reg2;
        }
    }
}

pub(super) fn monte_material_colors(object: &SceneObject) -> Option<MonteMaterialColors> {
    let factory = object.factory_name.to_ascii_lowercase();
    if !factory.starts_with("npcmonte") {
        return None;
    }
    let body_index = npc_color_index(object, "npc_body_color_index")?;
    let cloth_index = npc_color_index(object, "npc_cloth_color_index")?;
    let male_body = MONTE_M_BODY_COLORS.get(body_index).copied();
    let female_body = MONTE_W_BODY_COLORS.get(body_index).copied();
    let (body_reg0, cloth_reg0, cloth_reg1_reg2) = match factory.as_str() {
        "npcmontemb" => (
            MONTE_MB_BODY_COLORS.get(body_index).copied(),
            MONTE_MB_CLOTH_COLORS.get(cloth_index).copied(),
            None,
        ),
        "npcmontema" | "npcmontemh" => (
            male_body,
            None,
            paired_color(&MONTE_MA_CLOTH_REG1, &MONTE_MA_CLOTH_REG2, cloth_index),
        ),
        "npcmontemc" | "npcmontemg" => (
            male_body,
            None,
            paired_color(&MONTE_MC_CLOTH_REG1, &MONTE_MC_CLOTH_REG2, cloth_index),
        ),
        "npcmontemd" => (
            male_body,
            MONTE_MD_CLOTH_COLORS.get(cloth_index).copied(),
            None,
        ),
        "npcmontewa" => (
            female_body,
            MONTE_WA_CLOTH_COLORS.get(cloth_index).copied(),
            None,
        ),
        "npcmontewb" => (
            female_body,
            None,
            paired_color(&MONTE_WB_CLOTH_REG1, &MONTE_WB_CLOTH_REG2, cloth_index),
        ),
        "npcmontew" | "npcmontewc" => (female_body, None, None),
        "npcmonteme" => (None, None, None),
        _ => (male_body, None, None),
    };
    Some(MonteMaterialColors {
        body_reg0,
        cloth_reg0,
        cloth_reg1_reg2,
    })
}

pub(super) fn npc_color_index(object: &SceneObject, key: &str) -> Option<usize> {
    object
        .raw_params
        .get(key)?
        .parse::<i32>()
        .ok()?
        .try_into()
        .ok()
}

pub(super) fn paired_color(
    reg1: &[[i16; 4]],
    reg2: &[[i16; 4]],
    index: usize,
) -> Option<[[i16; 4]; 2]> {
    Some([*reg1.get(index)?, *reg2.get(index)?])
}

const MONTE_M_BODY_COLORS: [[i16; 4]; 10] = [
    [100, 255, 300, 255],
    [120, 120, 300, 255],
    [350, 300, 0, 255],
    [200, 70, 0, 255],
    [300, 130, 255, 255],
    [255, 350, 0, 255],
    [400, 255, 255, 255],
    [320, 140, 0, 255],
    [200, 255, 400, 255],
    [400, 250, 100, 255],
];
const MONTE_MA_CLOTH_REG1: [[i16; 4]; 11] = [
    [255, 255, 255, 255],
    [255, 255, 255, 255],
    [255, 255, 255, 255],
    [200, 200, 170, 255],
    [50, 50, 50, 255],
    [150, 200, 255, 255],
    [0, 70, 150, 255],
    [400, 300, 200, 255],
    [255, 255, 255, 255],
    [255, 255, 255, 255],
    [255, 255, 150, 255],
];
const MONTE_MA_CLOTH_REG2: [[i16; 4]; 11] = [
    [250, 130, 50, 255],
    [50, 130, 100, 255],
    [150, 180, 20, 255],
    [200, 200, 170, 255],
    [50, 50, 50, 255],
    [150, 200, 255, 255],
    [0, 70, 150, 255],
    [230, 150, 100, 255],
    [60, 150, 230, 255],
    [180, 150, 200, 255],
    [100, 220, 300, 255],
];
const MONTE_MB_BODY_COLORS: [[i16; 4]; 4] = [
    [160, 200, 300, 255],
    [255, 160, 150, 255],
    [300, 200, 80, 255],
    [200, 300, 100, 255],
];
const MONTE_MB_CLOTH_COLORS: [[i16; 4]; 6] = [
    [70, 130, 200, 255],
    [200, 20, 20, 255],
    [130, 30, 80, 255],
    [130, 200, 80, 255],
    [230, 200, 80, 255],
    [50, 100, 150, 255],
];
const MONTE_MC_CLOTH_REG1: [[i16; 4]; 11] = [
    [230, 230, 210, 255],
    [150, 70, 0, 255],
    [230, 230, 210, 255],
    [0, 70, 150, 255],
    [50, 150, 130, 255],
    [60, 40, 0, 255],
    [0, 100, 100, 255],
    [0, 150, 200, 255],
    [0, 50, 100, 255],
    [100, 100, 0, 255],
    [100, 0, 0, 255],
];
const MONTE_MC_CLOTH_REG2: [[i16; 4]; 11] = [
    [230, 230, 210, 255],
    [150, 70, 0, 255],
    [0, 70, 150, 255],
    [230, 230, 210, 255],
    [230, 230, 210, 255],
    [160, 150, 60, 255],
    [0, 100, 100, 255],
    [0, 150, 200, 255],
    [0, 50, 100, 255],
    [0, 0, 0, 255],
    [0, 0, 0, 255],
];
const MONTE_MD_CLOTH_COLORS: [[i16; 4]; 5] = [
    [350, 360, 340, 255],
    [50, 100, 0, 255],
    [150, 0, 0, 255],
    [0, 300, 350, 255],
    [0, 100, 250, 255],
];
const MONTE_W_BODY_COLORS: [[i16; 4]; 6] = [
    [300, 100, 200, 255],
    [400, 150, 0, 255],
    [300, 330, 0, 255],
    [400, 330, 0, 255],
    [330, 40, 0, 255],
    [400, 200, 255, 255],
];
const MONTE_WA_CLOTH_COLORS: [[i16; 4]; 6] = [
    [380, 330, 150, 255],
    [300, 100, 200, 255],
    [360, 350, 300, 255],
    [300, 50, 0, 255],
    [400, 150, 100, 255],
    [120, 150, 300, 255],
];
const MONTE_WB_CLOTH_REG1: [[i16; 4]; 9] = [
    [220, 200, 220, 255],
    [200, 220, 220, 255],
    [255, 255, 255, 255],
    [255, 255, 255, 255],
    [220, 230, 220, 255],
    [180, 100, 110, 255],
    [200, 100, 0, 255],
    [0, 100, 150, 255],
    [255, 200, 100, 255],
];
const MONTE_WB_CLOTH_REG2: [[i16; 4]; 9] = [
    [100, 80, 200, 255],
    [100, 170, 200, 255],
    [150, 0, 60, 255],
    [180, 120, 200, 255],
    [140, 180, 300, 255],
    [180, 100, 110, 255],
    [200, 100, 0, 255],
    [0, 100, 150, 255],
    [255, 200, 100, 255],
];

// NpcInitData.cpp: sMareM_BodyColor and sMareW_BodyColor. The instance color is
// installed into TEV register 0 by SMS_InitChangeNpcColor.
const MARE_M_BODY_COLORS: [[i16; 4]; 6] = [
    [370, 290, 170, 255],
    [350, 340, 120, 255],
    [260, 180, 100, 255],
    [300, 150, 130, 255],
    [300, 200, 60, 255],
    [240, 160, 230, 255],
];
const MARE_W_BODY_COLORS: [[i16; 4]; 6] = [
    [410, 255, 280, 255],
    [430, 330, 150, 255],
    [440, 330, 255, 255],
    [300, 200, 130, 255],
    [300, 190, 220, 255],
    [400, 250, 90, 255],
];

pub(super) fn nozzle_box_tev_reg1_color(object: &SceneObject) -> Option<[i16; 4]> {
    let is_nozzle_box = object.factory_name.eq_ignore_ascii_case("NozzleBox")
        || object
            .class_name
            .as_deref()
            .is_some_and(|name| name.eq_ignore_ascii_case("NozzleBox"));
    if !is_nozzle_box {
        return None;
    }

    let nozzle_item = object
        .raw_params
        .values()
        .map(|value| value.to_ascii_lowercase())
        .find(|value| value.ends_with("_nozzle_item"));
    Some(match nozzle_item.as_deref() {
        Some("normal_nozzle_item") => [0, 0, 255, 100],
        Some("rocket_nozzle_item") => [255, 0, 0, 100],
        Some("back_nozzle_item") => [90, 90, 120, 100],
        _ => [255, 255, 255, 100],
    })
}
