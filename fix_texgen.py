import io

p = "apps/sms-editor/src/mask_tool.rs"
t = io.open(p, encoding="utf-8").read()

# 1. Shading mode, so the material program is opt-in rather than imposed.
old = '''/// Where a goop texture comes from.'''
assert t.count(old) == 1, "source enum anchor"
t = t.replace(old, '''/// How the preview shades a model.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum MaskShading {
    /// The combine the geometry already resolved, lit by the surface normal.
    /// Reliable, and close to how the model reads in the stage.
    Simple,
    /// The material's own TEV program. Correct for actors whose colour lives
    /// in a TEV register, but the preview generates only stored texture
    /// coordinates, so a material that generates its own -- a toon ramp keyed
    /// on the normal, say -- is approximated.
    MaterialTev,
}

/// Where a goop texture comes from.''', 1)

# 2. Sample stored coordinates per stage, and approximate generated ones.
old = '''                            evaluate_tev(material, raster, &|map, _coord| {
                                geometry
                                    .textures
                                    .get(map)
                                    .zip(tex_coords)
                                    .and_then(|(texture, set)| {
                                        let u = set[0][0] * w2 + set[1][0] * w1 + set[2][0] * w0;
                                        let v = set[0][1] * w2 + set[1][1] * w1 + set[2][1] * w0;
                                        sample_texture(texture, u, v)
                                    })
                                    .map(|sample| sample.map(|c| c as f32 / 255.0))
                                    .unwrap_or([1.0; 4])
                            })'''
assert t.count(old) == 1, "sample closure"
new = '''                            evaluate_tev(material, raster, &|map, coord| {
                                let Some(texture) = geometry.textures.get(map) else {
                                    return [1.0; 4];
                                };
                                // A stage names which coordinate set it reads.
                                // Materials that generate a coordinate from the
                                // normal -- toon ramps -- have no stored set,
                                // so the facing angle stands in for it, which
                                // is what such a ramp is keyed on.
                                let stored = coord
                                    .and_then(|index| triangle.tex_coord_sets.get(index).copied())
                                    .flatten()
                                    .or(tex_coords);
                                let (u, v) = match stored {
                                    Some(set) => (
                                        set[0][0] * w2 + set[1][0] * w1 + set[2][0] * w0,
                                        set[0][1] * w2 + set[1][1] * w1 + set[2][1] * w0,
                                    ),
                                    None => (facing.clamp(0.0, 1.0), 0.5),
                                };
                                sample_texture(texture, u, v)
                                    .map(|sample| sample.map(|c| c as f32 / 255.0))
                                    .unwrap_or([1.0; 4])
                            })'''
t = t.replace(old, new, 1)

# 3. Raster colour carries the lighting, not flat white.
old = '''                            let raster = vertex
                                .map(|colour| {
                                    colour.map(|channel| channel as f32 / 255.0)
                                })
                                .unwrap_or([1.0; 4]);'''
assert t.count(old) == 1, "raster"
new = '''                            // The raster colour is the lit vertex colour. With
                            // no stored colours, the surface's own lighting
                            // stands in, so stages that modulate by it are not
                            // handed flat white and blown out.
                            let raster = vertex
                                .map(|colour| colour.map(|channel| channel as f32 / 255.0))
                                .unwrap_or([shade, shade, shade, 1.0]);'''
t = t.replace(old, new, 1)

# 4. Facing term for generated coordinates.
old = '''                    let modulate = |a: [u8; 4], b: [u8; 4]| -> [u8; 4] {'''
assert t.count(old) == 1, "modulate anchor"
new = '''                    let facing = shade;
                    let modulate = |a: [u8; 4], b: [u8; 4]| -> [u8; 4] {'''
t = t.replace(old, new, 1)

# 5. Honour the shading mode.
old = '''                    let material = triangle
                        .texture_index'''
assert t.count(old) == 1, "material lookup"
new = '''                    let material = (self.mask_shading == MaskShading::MaterialTev)
                        .then_some(())
                        .and(triangle.texture_index)'''
t = t.replace(old, new, 1)
old = '''                        .and_then(|slot| preview.material_for_texture.get(slot).copied().flatten())
                        .or(triangle.material_index)
                        .and_then(|index| geometry.materials.get(index))
                        .filter(|material| !material.tev_stages.is_empty());'''
assert t.count(old) == 1, "material chain"
t = t.replace(old, '''                        .and_then(|slot| preview.material_for_texture.get(slot).copied().flatten())
                        .or_else(|| {
                            (self.mask_shading == MaskShading::MaterialTev)
                                .then_some(triangle.material_index)
                                .flatten()
                        })
                        .and_then(|index| geometry.materials.get(index))
                        .filter(|material| !material.tev_stages.is_empty());''', 1)

# 6. The toggle in the panel.
old = '''        ui.horizontal(|ui| {
            ui.label("UV layer:");'''
assert t.count(old) == 1, "uv layer row"
new = '''        ui.horizontal(|ui| {
            ui.label("Shading:");
            ui.selectable_value(&mut self.mask_shading, MaskShading::Simple, "Simple")
                .on_hover_text("The combine the geometry resolved, lit by the surface normal");
            ui.selectable_value(
                &mut self.mask_shading,
                MaskShading::MaterialTev,
                "Material TEV",
            )
            .on_hover_text(
                "Run the material's own TEV program -- right for actors whose colour lives in a \\
                 TEV register, approximate where a material generates its own coordinates",
            );
        });
        ui.horizontal(|ui| {
            ui.label("UV layer:");'''
t = t.replace(old, new, 1)

io.open(p, "w", encoding="utf-8").write(t)
print("texgen, raster lighting and shading toggle wired")
