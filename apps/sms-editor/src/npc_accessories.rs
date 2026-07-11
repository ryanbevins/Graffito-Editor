use super::*;

pub(super) fn apply_monte_accessory_material_color(
    material: &mut J3dMaterial,
    object: &SceneObject,
    asset_suffix: &str,
) {
    let reg0 = |material: &mut J3dMaterial, name: &str, colors: &[[i16; 4]], index: usize| {
        if material.name.eq_ignore_ascii_case(name) {
            if let Some(color) = colors.get(index) {
                material.tev_colors[0] = *color;
            }
        }
    };
    let reg12 = |material: &mut J3dMaterial,
                 name: &str,
                 colors1: &[[i16; 4]],
                 colors2: &[[i16; 4]],
                 index: usize| {
        if material.name.eq_ignore_ascii_case(name) {
            if let (Some(color1), Some(color2)) = (colors1.get(index), colors2.get(index)) {
                material.tev_colors[1] = *color1;
                material.tev_colors[2] = *color2;
            }
        }
    };
    let mat_color = |material: &mut J3dMaterial, name: &str, colors: &[[i16; 4]], index: usize| {
        if material.name.eq_ignore_ascii_case(name) {
            if let Some(color) = colors.get(index) {
                material.material_colors[0] = (*color).map(|value| value as u8);
            }
        }
    };

    if asset_suffix.ends_with("/montemcommon/hata_model.bmd") {
        let Some(index) = npc_parts_color_index(object, 0) else {
            return;
        };
        reg12(
            material,
            "_boushi_mat",
            &[
                [200, 200, 120, 255],
                [70, 70, 70, 255],
                [200, 200, 150, 255],
                [100, 200, 200, 255],
                [0, 30, 150, 255],
                [200, 220, 120, 255],
                [140, 50, 0, 255],
            ],
            &[
                [160, 130, 50, 255],
                [70, 70, 70, 255],
                [200, 200, 150, 255],
                [50, 120, 160, 255],
                [0, 30, 150, 255],
                [0, 130, 50, 255],
                [140, 50, 0, 255],
            ],
            index,
        );
        reg0(
            material,
            "_obi_mat",
            &[
                [100, 0, 0, 255],
                [0, 0, 0, 255],
                [0, 0, 0, 255],
                [0, 100, 300, 255],
                [0, 200, 150, 255],
                [0, 130, 130, 255],
                [100, 0, 0, 255],
            ],
            index,
        );
    } else if asset_suffix.ends_with("higea_model.bmd") {
        if let Some(index) = npc_parts_color_index(object, 1) {
            reg0(
                material,
                "_hige_mat",
                &[
                    [0, 0, 0, 255],
                    [255, 255, 150, 255],
                    [100, 0, 0, 255],
                    [255, 200, 0, 255],
                ],
                index,
            );
        }
    } else if asset_suffix.ends_with("glassesb_model.bmd") {
        if let Some(index) = npc_parts_color_index(object, 2) {
            mat_color(
                material,
                "_megane_mat",
                &[[255, 230, 50, 255], [255, 255, 255, 255], [0, 0, 0, 255]],
                index,
            );
        }
    } else if asset_suffix.ends_with("hatb_model.bmd") {
        if let Some(index) = npc_parts_color_index(object, 0) {
            mat_color(
                material,
                "_boushi_mat",
                &[[255, 255, 60, 255], [255, 130, 0, 255], [255, 255, 0, 255]],
                index,
            );
            reg0(
                material,
                "_obi_mat",
                &[[100, 70, 50, 255], [100, 70, 50, 255], [0, 100, 255, 255]],
                index,
            );
        }
    } else if asset_suffix.ends_with("hatd_model.bmd") {
        if let Some(index) = npc_parts_color_index(object, 0) {
            reg0(
                material,
                "_obi_mat",
                &[[0, 100, 230, 255], [400, 400, 350, 255]],
                index,
            );
        }
    } else if asset_suffix.ends_with("hate_model.bmd") {
        if let Some(index) = npc_parts_color_index(object, 0) {
            reg12(
                material,
                "_boushi_mat",
                &[[0, 70, 150, 255], [230, 230, 210, 255]],
                &[[230, 230, 210, 255], [0, 70, 150, 255]],
                index,
            );
        }
    } else if asset_suffix.ends_with("hatf_model.bmd") {
        if let Some(index) = npc_parts_color_index(object, 0) {
            reg12(
                material,
                "_boushi_mat",
                &[[50, 150, 130, 255], [60, 40, 0, 255]],
                &[[230, 230, 210, 255], [160, 150, 60, 255]],
                index,
            );
        }
    } else if asset_suffix.ends_with("hatg_model.bmd") {
        if let Some(index) = npc_parts_color_index(object, 0) {
            reg12(
                material,
                "_boushi_mat",
                &[
                    [0, 50, 120, 255],
                    [100, 120, 0, 255],
                    [170, 120, 0, 255],
                    [0, 100, 255, 255],
                    [140, 50, 0, 255],
                ],
                &[
                    [0, 50, 120, 255],
                    [100, 120, 0, 255],
                    [170, 120, 0, 255],
                    [0, 180, 255, 255],
                    [200, 120, 0, 255],
                ],
                index,
            );
        }
    } else if asset_suffix.ends_with("eria_model.bmd") {
        if let Some(index) = npc_parts_color_index(object, 1) {
            reg12(
                material,
                "_eri_mat",
                &[[0, 70, 150, 255], [255, 255, 230, 255]],
                &[[255, 255, 230, 255], [0, 70, 150, 255]],
                index,
            );
        }
    } else if asset_suffix.ends_with("tieb_model.bmd") {
        if let Some(index) = npc_parts_color_index(object, 0) {
            reg12(
                material,
                "_tie_mat",
                &[[100, 0, 0, 255]],
                &[[150, 130, 0, 255]],
                index,
            );
        }
    } else if asset_suffix.ends_with("flower_model.bmd") {
        if let Some(index) = npc_parts_color_index(object, 1) {
            reg0(material, "_naka_mat", &[[255, 255, 0, 255]; 3], index);
            mat_color(
                material,
                "_hana_mat",
                &[[220, 40, 120, 255], [220, 40, 0, 255], [200, 220, 0, 255]],
                index,
            );
        }
    } else if asset_suffix.ends_with("hwa_model.bmd") {
        if let Some(index) = npc_parts_color_index(object, 0) {
            reg12(
                material,
                "_boushi_mat",
                &[
                    [200, 200, 120, 255],
                    [160, 0, 0, 255],
                    [190, 150, 120, 255],
                    [200, 220, 220, 255],
                ],
                &[
                    [160, 130, 50, 255],
                    [160, 0, 0, 255],
                    [120, 70, 50, 255],
                    [100, 170, 200, 255],
                ],
                index,
            );
            reg0(
                material,
                "_obi_mat",
                &[
                    [300, 100, 100, 255],
                    [350, 100, 100, 255],
                    [100, 0, 0, 255],
                    [400, 400, 380, 255],
                ],
                index,
            );
        }
    } else if asset_suffix.ends_with("gwb_model.bmd") {
        if let Some(index) = npc_parts_color_index(object, 2) {
            mat_color(
                material,
                "_megane_mat",
                &[[150, 0, 0, 255], [0, 200, 0, 255], [200, 200, 0, 255]],
                index,
            );
        }
    }
}
