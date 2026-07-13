use super::*;

pub(super) fn apply_npc_accessory_material_color(
    material: &mut J3dMaterial,
    object: &SceneObject,
    spec: &NpcAccessorySpec,
) {
    let color_index = npc_parts_color_index(object, usize::from(spec.color_index_channel));
    for change in spec
        .color_changes
        .iter()
        .filter(|change| material.name.eq_ignore_ascii_case(&change.material_name))
    {
        let Some(index) = color_index else {
            continue;
        };
        match change.mode {
            0 => {
                if let Some(color) = change.colors0.get(index) {
                    material.material_colors[0] = color.map(|value| value as u8);
                }
            }
            1 => {
                if let Some(color) = change.colors0.get(index) {
                    material.tev_colors[0] = *color;
                }
            }
            2 => {
                if let Some(color) = change.colors0.get(index) {
                    material.tev_colors[1] = *color;
                }
                if let Some(color) = change.colors1.get(index) {
                    material.tev_colors[2] = *color;
                }
            }
            _ => {}
        }
    }

    if spec.uses_pollution {
        if let Some(color) = npc_pollution_k_color(object) {
            material.tev_k_colors[0] = color;
        }
    }
}
