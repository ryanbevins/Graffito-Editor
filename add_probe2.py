import io

p = "apps/sms-editor/src/tests.rs"
t = io.open(p, encoding="utf-8").read()
anchor = "/// Lists the textures inside a pollution model"
assert t.count(anchor) == 1, "anchor"
add = r'''/// Reports what colour data an actor model's triangles actually carry, to see
/// why one renders untinted. `GRAFFITO_PROBE_SZS=<stage.szs>
/// GRAFFITO_PROBE_BMD=poihana.bmd GRAFFITO_PROBE_FLAGS=<u32>
/// cargo test probe_actor_colour -- --ignored --nocapture`
#[test]
#[ignore]
fn probe_actor_colour() {
    let Ok(path) = std::env::var("GRAFFITO_PROBE_SZS") else {
        return;
    };
    let bmd = std::env::var("GRAFFITO_PROBE_BMD").expect("GRAFFITO_PROBE_BMD");
    let assets = sms_formats::mount_scene_archive(std::path::Path::new(&path)).expect("mount");
    let asset = assets
        .iter()
        .find(|a| {
            a.path
                .to_string_lossy()
                .replace('\\', "/")
                .to_ascii_lowercase()
                .ends_with(&bmd.to_ascii_lowercase())
        })
        .unwrap_or_else(|| panic!("no asset ending {bmd}"));
    let bytes = sms_formats::read_stage_asset_bytes(&asset.path).expect("read");
    let model = sms_formats::J3dFile::parse(&bytes).expect("parse");

    for flags in [
        sms_formats::SMS_MAP_MODEL_LOAD_FLAGS,
        0x0102_0000,
        0x1102_0000,
        0x0001_0000,
    ] {
        let Ok(geometry) = model.geometry_preview_with_loader_flags(flags) else {
            println!("flags {flags:#010x}: geometry failed");
            continue;
        };
        let mut modes = std::collections::BTreeMap::new();
        let mut with_color = 0usize;
        let mut with_vertex = 0usize;
        let mut sample_color = None;
        for triangle in &geometry.triangles {
            *modes.entry(format!("{:?}", triangle.combine_mode)).or_insert(0usize) += 1;
            if let Some(color) = triangle.color {
                with_color += 1;
                if sample_color.is_none() {
                    sample_color = Some(color);
                }
            }
            if triangle.vertex_colors.is_some() || triangle.color_channels[0].is_some() {
                with_vertex += 1;
            }
        }
        println!(
            "flags {flags:#010x}: {} tris, modes {modes:?}, with color {with_color} (e.g. {sample_color:?}), with vertex {with_vertex}, materials {}",
            geometry.triangles.len(),
            geometry.materials.len()
        );
        for material in geometry.materials.iter().take(4) {
            println!(
                "   material [{}] colors {:?} tex {:?}",
                material.name,
                material.material_colors,
                material.texture_indices.iter().flatten().next()
            );
        }
    }
}

'''
t = t.replace(anchor, add + anchor, 1)
io.open(p, "w", encoding="utf-8").write(t)
print("colour probe added")
