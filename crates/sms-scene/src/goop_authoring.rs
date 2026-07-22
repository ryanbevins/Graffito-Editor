use serde::{Deserialize, Serialize};
use sms_formats::{
    compile_static_bmd3, compile_texture_section, BmpFile, GxEncodedTexture, GxMaterial,
    GxPaletteFormat, GxTextureEncodeOptions, GxTextureEncoding, GxTextureFormat,
    J3dRebuildDocument, J3dRebuildSectionData, RgbaImage, StaticModel, StaticModelMesh,
    StaticModelVertex, YmpDocument, YmpLayer,
};

use crate::{Result, SceneError, StageDocument, StageResourceDocument};

pub const GOOP_RESOURCE_PATH: &[u8] = b"map/ymap.ymp";
pub const GOOP_CELL_SIZE: f32 = 40.0;
pub const GOOP_MAX_LAYERS: usize = 20;
pub const GOOP_MAX_DIMENSION: usize = 1024;
pub const GOOP_AUTHORING_FORMAT_VERSION: u32 = 4;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GoopPlane {
    Floor,
    WallPlusX,
    WallMinusX,
    WallPlusZ,
    WallMinusZ,
    Wave,
    Retail(u16),
}

impl GoopPlane {
    pub fn runtime_code(self) -> u16 {
        match self {
            Self::Floor => 0,
            Self::WallPlusX => 2,
            Self::WallMinusX => 3,
            Self::WallPlusZ => 4,
            Self::WallMinusZ => 5,
            Self::Wave => 6,
            Self::Retail(code) => code,
        }
    }

    pub fn from_runtime_code(code: u16) -> Option<Self> {
        match code {
            0 | 1 => Some(Self::Floor),
            2 => Some(Self::WallPlusX),
            3 => Some(Self::WallMinusX),
            4 => Some(Self::WallPlusZ),
            5 => Some(Self::WallMinusZ),
            6 => Some(Self::Wave),
            code => Some(Self::Retail(code)),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GoopBehavior {
    Normal,
    Fire,
    Slippery,
    Barrier,
    Electric,
    Retail(u16),
}

impl GoopBehavior {
    pub fn runtime_code(self) -> u16 {
        match self {
            Self::Normal => 0,
            Self::Fire => 1,
            Self::Slippery => 2,
            Self::Barrier => 3,
            Self::Electric => 4,
            Self::Retail(code) => code,
        }
    }

    pub fn from_runtime_code(code: u16) -> Self {
        match code {
            0 => Self::Normal,
            1 => Self::Fire,
            2 => Self::Slippery,
            3 => Self::Barrier,
            4 => Self::Electric,
            code => Self::Retail(code),
        }
    }

    pub fn label(self) -> String {
        match self {
            Self::Normal => "Normal".to_string(),
            Self::Fire => "Fire".to_string(),
            Self::Slippery => "Slippery".to_string(),
            Self::Barrier => "Barrier".to_string(),
            Self::Electric => "Electric".to_string(),
            Self::Retail(code) => format!("Retail type {code}"),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GoopStyleSource {
    pub stage_id: String,
    pub layer_index: usize,
    pub display_name: String,
    #[serde(default)]
    pub behavior_code: u16,
    #[serde(default)]
    pub forced_incompatible: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct GoopRegion {
    pub min_x: f32,
    pub min_z: f32,
    pub max_x: f32,
    pub max_z: f32,
}

impl GoopRegion {
    pub fn contains(self, x: f32, z: f32) -> bool {
        x >= self.min_x && x < self.max_x && z >= self.min_z && z < self.max_z
    }

    pub fn overlaps(self, other: Self) -> bool {
        self.min_x < other.max_x
            && other.min_x < self.max_x
            && self.min_z < other.max_z
            && other.min_z < self.max_z
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GoopLayerOrigin {
    Imported,
    Generated,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct GoopLayerAuthoring {
    pub id: String,
    pub runtime_index: usize,
    pub origin: GoopLayerOrigin,
    pub plane: GoopPlane,
    pub behavior: GoopBehavior,
    #[serde(default = "default_true")]
    pub visible: bool,
    pub region: GoopRegion,
    pub runtime: YmpLayer,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bitmap: Option<BmpFile>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub generated_model: Option<J3dRebuildDocument>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub style_source: Option<GoopStyleSource>,
    #[serde(default)]
    pub resource_stem: String,
    #[serde(default)]
    pub metadata_dirty: bool,
}

fn default_true() -> bool {
    true
}

impl GoopLayerAuthoring {
    pub fn editable(&self) -> bool {
        self.plane == GoopPlane::Floor && self.bitmap.is_some()
    }

    pub fn dimensions(&self) -> Result<(usize, usize)> {
        Ok(self.runtime.dimensions()?)
    }

    pub fn valid_cell(&self, x: usize, y: usize) -> bool {
        self.runtime.depth_at(x, y).is_ok_and(|depth| depth != 0xff)
    }

    pub fn mask(&self) -> Result<Vec<u8>> {
        self.bitmap
            .as_ref()
            .ok_or_else(|| {
                SceneError::StageExport(format!("goop layer {} has no bitmap", self.id))
            })?
            .top_down_indices()
            .map_err(Into::into)
    }

    pub fn set_mask(&mut self, mask: &[u8]) -> Result<()> {
        self.bitmap
            .as_mut()
            .ok_or_else(|| {
                SceneError::StageExport(format!("goop layer {} has no bitmap", self.id))
            })?
            .set_top_down_indices(mask)?;
        Ok(())
    }

    pub fn world_to_cell(&self, x: f32, z: f32) -> Option<(usize, usize)> {
        if !self.region.contains(x, z) {
            return None;
        }
        let (width, height) = self.runtime.dimensions().ok()?;
        let cell_size = self.runtime.vertical_scale;
        if !cell_size.is_finite() || cell_size <= 0.0 {
            return None;
        }
        let cell_x = ((x - self.region.min_x) / cell_size) as usize;
        let cell_y = ((z - self.region.min_z) / cell_size) as usize;
        (cell_x < width && cell_y < height).then_some((cell_x, cell_y))
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct GoopAuthoringDocument {
    pub format_version: u32,
    pub layers: Vec<GoopLayerAuthoring>,
    #[serde(default)]
    pub terrain_fingerprint: u64,
    #[serde(default)]
    pub stale: bool,
}

impl Default for GoopAuthoringDocument {
    fn default() -> Self {
        Self {
            format_version: GOOP_AUTHORING_FORMAT_VERSION,
            layers: Vec::new(),
            terrain_fingerprint: 0,
            stale: false,
        }
    }
}

impl GoopAuthoringDocument {
    pub fn requires_generator_upgrade(&self) -> bool {
        self.format_version < GOOP_AUTHORING_FORMAT_VERSION
            && self
                .layers
                .iter()
                .any(|layer| layer.origin == GoopLayerOrigin::Generated)
    }

    pub fn validate(&self) -> Result<()> {
        if self.requires_generator_upgrade() {
            return Err(SceneError::StageExport(format!(
                "generated goop uses authoring format version {}, but this editor requires version {GOOP_AUTHORING_FORMAT_VERSION}; open the Goop tool and rebuild the generated layers",
                self.format_version
            )));
        }
        if self.layers.len() > GOOP_MAX_LAYERS {
            return Err(SceneError::StageExport(format!(
                "Sunshine supports at most {GOOP_MAX_LAYERS} goop layers"
            )));
        }
        for (index, layer) in self.layers.iter().enumerate() {
            if layer.runtime_index != index {
                return Err(SceneError::StageExport(format!(
                    "goop layer {} has runtime index {}, expected {index}",
                    layer.id, layer.runtime_index
                )));
            }
            let (width, height) = layer.dimensions()?;
            if width > GOOP_MAX_DIMENSION || height > GOOP_MAX_DIMENSION {
                return Err(SceneError::StageExport(format!(
                    "goop layer {} is {width}x{height}, exceeding 1024x1024",
                    layer.id
                )));
            }
            if layer.plane == GoopPlane::Floor {
                layer.runtime.validate_floor_runtime()?;
            }
            if let Some(bitmap) = &layer.bitmap {
                if bitmap.width.unsigned_abs() as usize != width
                    || bitmap.height.unsigned_abs() as usize != height
                {
                    return Err(SceneError::StageExport(format!(
                        "goop layer {} bitmap dimensions do not match YMP",
                        layer.id
                    )));
                }
            }
            if layer.origin == GoopLayerOrigin::Generated {
                if (layer.runtime.vertical_scale - GOOP_CELL_SIZE).abs() > f32::EPSILON {
                    return Err(SceneError::StageExport(format!(
                        "generated goop layer {} must use the canonical {GOOP_CELL_SIZE}-unit scale, got {}",
                        layer.id, layer.runtime.vertical_scale
                    )));
                }
                if layer.bitmap.is_none() || layer.generated_model.is_none() {
                    return Err(SceneError::StageExport(format!(
                        "generated goop layer {} is missing its bitmap or pollution model",
                        layer.id
                    )));
                }
                if layer.style_source.is_none() {
                    return Err(SceneError::StageExport(format!(
                        "generated goop layer {} has no retail style provenance",
                        layer.id
                    )));
                }
            }
            if let Some(model) = &layer.generated_model {
                let first_texture = model
                    .sections
                    .iter()
                    .find_map(|section| match &section.data {
                        J3dRebuildSectionData::Textures(textures) => textures.textures.first(),
                        _ => None,
                    });
                let Some(texture) = first_texture else {
                    return Err(SceneError::StageExport(format!(
                        "goop layer {} model has no first texture",
                        layer.id
                    )));
                };
                if texture.format != GxTextureFormat::I8 as u8
                    || usize::from(texture.width) != width
                    || usize::from(texture.height) != height
                {
                    return Err(SceneError::StageExport(format!(
                        "goop layer {} model texture zero is not a matching I8 mask",
                        layer.id
                    )));
                }
            }
        }
        for left in 0..self.layers.len() {
            for right in left + 1..self.layers.len() {
                if (self.layers[left].origin == GoopLayerOrigin::Generated
                    || self.layers[right].origin == GoopLayerOrigin::Generated)
                    && self.layers[left].plane == GoopPlane::Floor
                    && self.layers[right].plane == GoopPlane::Floor
                    && self.layers[left].region.overlaps(self.layers[right].region)
                {
                    return Err(SceneError::StageExport(format!(
                        "goop regions {} and {} overlap",
                        self.layers[left].id, self.layers[right].id
                    )));
                }
            }
        }
        if self.stale {
            return Err(SceneError::StageExport(
                "generated goopmaps are stale; rebuild them before export".to_string(),
            ));
        }
        Ok(())
    }

    pub fn compiled_ymp(&self) -> Result<YmpDocument> {
        self.validate()?;
        Ok(YmpDocument::canonical(
            self.layers
                .iter()
                .map(|layer| layer.runtime.clone())
                .collect(),
        )?)
    }

    pub fn compiled_ymp_preserving(&self, base: &YmpDocument) -> Result<YmpDocument> {
        self.validate()?;
        let allocation_compatible = base.layers.len() == self.layers.len()
            && base
                .layers
                .iter()
                .zip(&self.layers)
                .all(|(old, authored)| old.depth_map.len() == authored.runtime.depth_map.len());
        if !allocation_compatible {
            return self.compiled_ymp();
        }
        let mut document = base.clone();
        for (target, authored) in document.layers.iter_mut().zip(&self.layers) {
            let map_offset = target.map_offset;
            *target = authored.runtime.clone();
            target.map_offset = map_offset;
        }
        // Encoding performs checked bounds validation while retaining the
        // imported allocation, padding styles, and unrelated layer bytes.
        document.encode()?;
        Ok(document)
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct GoopTerrainTriangle {
    pub vertices: [[f32; 3]; 3],
}

/// One triangle decoded from the finalized map-render BMD. Unlike collision
/// triangles, its GX winding and optional vertex normals retain which side is
/// the visible upward surface.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct GoopRenderTriangle {
    pub vertices: [[f32; 3]; 3],
    pub normals: Option<[[f32; 3]; 3]>,
}

pub fn whole_terrain_region(triangles: &[GoopTerrainTriangle]) -> Result<(GoopRegion, u16, u16)> {
    let mut min_x = f32::INFINITY;
    let mut min_z = f32::INFINITY;
    let mut max_x = f32::NEG_INFINITY;
    let mut max_z = f32::NEG_INFINITY;
    for vertex in triangles.iter().flat_map(|triangle| triangle.vertices) {
        if !vertex.iter().all(|value| value.is_finite()) {
            continue;
        }
        min_x = min_x.min(vertex[0]);
        min_z = min_z.min(vertex[2]);
        max_x = max_x.max(vertex[0]);
        max_z = max_z.max(vertex[2]);
    }
    if !min_x.is_finite() || !min_z.is_finite() {
        return Err(SceneError::StageExport(
            "terrain has no finite collision triangles".to_string(),
        ));
    }
    min_x = (min_x / GOOP_CELL_SIZE).floor() * GOOP_CELL_SIZE;
    min_z = (min_z / GOOP_CELL_SIZE).floor() * GOOP_CELL_SIZE;
    let cells_x = (((max_x - min_x) / GOOP_CELL_SIZE).ceil() as usize)
        .max(8)
        .next_power_of_two();
    let cells_z = (((max_z - min_z) / GOOP_CELL_SIZE).ceil() as usize)
        .max(4)
        .next_power_of_two();
    if cells_x > GOOP_MAX_DIMENSION || cells_z > GOOP_MAX_DIMENSION {
        return Err(SceneError::StageExport(format!(
            "terrain requires a {cells_x}x{cells_z} goopmap; create smaller regions"
        )));
    }
    Ok((
        GoopRegion {
            min_x,
            min_z,
            max_x: min_x + cells_x as f32 * GOOP_CELL_SIZE,
            max_z: min_z + cells_z as f32 * GOOP_CELL_SIZE,
        },
        cells_x.trailing_zeros() as u16,
        cells_z.trailing_zeros() as u16,
    ))
}

pub fn generate_floor_depth_map(
    triangles: &[GoopTerrainTriangle],
    region: GoopRegion,
    width_log2: u16,
    height_log2: u16,
) -> Result<(f32, Vec<u8>)> {
    let width = 1usize << width_log2;
    let height = 1usize << height_log2;
    if width < 8 || height < 4 || width > GOOP_MAX_DIMENSION || height > GOOP_MAX_DIMENSION {
        return Err(SceneError::StageExport(format!(
            "invalid goopmap dimensions {width}x{height}"
        )));
    }
    let mut samples = vec![None; width * height];
    let mut minimum = f32::INFINITY;
    let offsets = [
        [-5.0, -5.0],
        [GOOP_CELL_SIZE + 5.0, -5.0],
        [-5.0, GOOP_CELL_SIZE + 5.0],
        [GOOP_CELL_SIZE + 5.0, GOOP_CELL_SIZE + 5.0],
    ];
    for y in 0..height {
        for x in 0..width {
            let world_x = region.min_x + x as f32 * GOOP_CELL_SIZE;
            let world_z = region.min_z + y as f32 * GOOP_CELL_SIZE;
            let corners = offsets
                .map(|offset| topmost_height(triangles, world_x + offset[0], world_z + offset[1]));
            let [Some(a), Some(b), Some(c), Some(d)] = corners else {
                continue;
            };
            // Matches the decomp's chained is_near(ground0, ground2,
            // ground1, ground3) test.
            if (a - c).abs() > 30.0 || (c - b).abs() > 30.0 || (b - d).abs() > 30.0 {
                continue;
            }
            let Some(center) = topmost_height(
                triangles,
                world_x + GOOP_CELL_SIZE * 0.5,
                world_z + GOOP_CELL_SIZE * 0.5,
            ) else {
                continue;
            };
            samples[y * width + x] = Some(center);
            minimum = minimum.min(center);
        }
    }
    if !minimum.is_finite() {
        return Err(SceneError::StageExport(
            "the goop region contains no representable floor cells".to_string(),
        ));
    }
    let vertical_offset = (minimum / GOOP_CELL_SIZE).floor() * GOOP_CELL_SIZE;
    let mut layer = YmpLayer {
        layer_type: 0,
        subtype: 0,
        flags: 0,
        reserved: 0,
        vertical_offset,
        vertical_scale: GOOP_CELL_SIZE,
        min_x: region.min_x,
        min_z: region.min_z,
        max_x: region.max_x,
        max_z: region.max_z,
        width_log2,
        height_log2,
        user_value: 0,
        map_offset: 0,
        depth_map: vec![0xff; width * height],
    };
    for y in 0..height {
        for x in 0..width {
            let Some(value) = samples[y * width + x] else {
                continue;
            };
            let depth = ((value - vertical_offset) * 0.025).trunc();
            if !(0.0..=254.0).contains(&depth) {
                return Err(SceneError::StageExport(format!(
                    "terrain height {value} in cell ({x}, {y}) exceeds the YMP 8-bit vertical span from offset {vertical_offset}"
                )));
            }
            layer.set_depth(x, y, depth as u8)?;
        }
    }
    Ok((vertical_offset, layer.depth_map))
}

fn topmost_height(triangles: &[GoopTerrainTriangle], x: f32, z: f32) -> Option<f32> {
    triangles
        .iter()
        .filter_map(|triangle| triangle_height_at(triangle.vertices, x, z))
        .max_by(f32::total_cmp)
}

fn triangle_height_at(vertices: [[f32; 3]; 3], x: f32, z: f32) -> Option<f32> {
    let [a, b, c] = vertices;
    let denominator = (b[2] - c[2]) * (a[0] - c[0]) + (c[0] - b[0]) * (a[2] - c[2]);
    if denominator.abs() <= f32::EPSILON {
        return None;
    }
    let wa = ((b[2] - c[2]) * (x - c[0]) + (c[0] - b[0]) * (z - c[2])) / denominator;
    let wb = ((c[2] - a[2]) * (x - c[0]) + (a[0] - c[0]) * (z - c[2])) / denominator;
    let wc = 1.0 - wa - wb;
    (wa >= -0.0001 && wb >= -0.0001 && wc >= -0.0001).then_some(wa * a[1] + wb * b[1] + wc * c[1])
}

pub fn terrain_fingerprint(model: &[u8], collision: &[u8]) -> u64 {
    model
        .iter()
        .chain(collision)
        .fold(0xcbf2_9ce4_8422_2325, |hash, byte| {
            (hash ^ u64::from(*byte)).wrapping_mul(0x0000_0100_0000_01b3)
        })
}

/// Builds a source-free floor pollution model while retaining the template's
/// complete MAT3 state and every TEX1 texture after texture zero.
pub fn generate_floor_pollution_model(
    template: &J3dRebuildDocument,
    triangles: &[GoopRenderTriangle],
    region: GoopRegion,
    width: u16,
    height: u16,
    allow_incompatible_template: bool,
) -> Result<J3dRebuildDocument> {
    let material_section = template
        .sections
        .iter()
        .find(|section| matches!(section.data, J3dRebuildSectionData::Materials(_)))
        .cloned()
        .ok_or_else(|| SceneError::StageExport("goop template has no MAT3 section".to_string()))?;
    let texture_section = template
        .sections
        .iter()
        .find_map(|section| match &section.data {
            J3dRebuildSectionData::Textures(textures) => Some(textures),
            _ => None,
        })
        .ok_or_else(|| SceneError::StageExport("goop template has no TEX1 section".to_string()))?;
    let first = texture_section.textures.first().ok_or_else(|| {
        SceneError::StageExport("goop template TEX1 has no first texture".to_string())
    })?;
    if !allow_incompatible_template && first.format != GxTextureFormat::I8 as u8 {
        return Err(SceneError::StageExport(format!(
            "goop template texture zero is GX format {}, expected mutable I8",
            first.format
        )));
    }

    let mut textures = texture_section
        .textures
        .iter()
        .enumerate()
        .map(|(index, record)| {
            let name = texture_section
                .names
                .entries
                .get(index)
                .map_or_else(|| format!("texture{index}"), |entry| entry.name.clone());
            GxEncodedTexture::from_j3d_record(name, record).map_err(SceneError::from)
        })
        .collect::<Result<Vec<_>>>()?;
    let mask_name = textures
        .first()
        .map_or_else(|| "pollution".to_string(), |texture| texture.name.clone());
    let mask = RgbaImage {
        width,
        height,
        pixels: vec![0; usize::from(width) * usize::from(height) * 4],
    };
    textures[0] = GxEncodedTexture::encode_rgba(
        mask_name,
        &mask,
        GxTextureEncodeOptions {
            encoding: GxTextureEncoding::Exact(GxTextureFormat::I8),
            palette_format: GxPaletteFormat::Ia8,
            mip_count: 1,
            sampler: textures[0].sampler,
        },
    )?;

    let mut vertices = Vec::new();
    let mut indices = Vec::new();
    for triangle in triangles {
        let Some(surface_vertices) = lifted_upward_render_vertices(*triangle) else {
            continue;
        };
        let mut polygon = clip_triangle_to_region(surface_vertices, region);
        if polygon.len() < 3 {
            continue;
        }
        // `compile_static_bmd3` accepts conventional geometric winding and
        // reverses it when emitting GX's clockwise display-list winding.
        // J3D preview exposes GX's clockwise runtime winding. Convert it back
        // to conventional authoring winding before the static compiler emits
        // the final clockwise display list.
        if triangle_normal_y(surface_vertices) < 0.0 {
            polygon.reverse();
        }
        let start = u32::try_from(vertices.len()).map_err(|_| {
            SceneError::StageExport("generated goop mesh has too many vertices".to_string())
        })?;
        for position in polygon.iter().copied() {
            let mut vertex = StaticModelVertex::new(position, [0.0, 1.0, 0.0]);
            vertex.tex_coords[0] = Some([
                (position[0] - region.min_x) / (region.max_x - region.min_x),
                (position[2] - region.min_z) / (region.max_z - region.min_z),
            ]);
            vertices.push(vertex);
        }
        for index in 1..polygon.len() - 1 {
            indices.push([start, start + index as u32, start + index as u32 + 1]);
        }
    }
    if indices.is_empty() {
        return Err(SceneError::StageExport(
            "goop region has no upward-facing terrain mesh".to_string(),
        ));
    }
    let mut generated = compile_static_bmd3(&StaticModel {
        root_joint_name: "pollution".to_string(),
        meshes: vec![StaticModelMesh {
            name: "pollution".to_string(),
            material_index: 0,
            vertices,
            triangles: indices,
        }],
        materials: vec![GxMaterial::default()],
        textures: vec![textures[0].clone()],
    })?;
    let textures = compile_texture_section(&textures)?;
    for section in &mut generated.sections {
        match section.data {
            J3dRebuildSectionData::Materials(_) => *section = material_section.clone(),
            J3dRebuildSectionData::Textures(_) => *section = textures.clone(),
            _ => {}
        }
    }
    // Reparse the encoded model so offset, count, and section agreement is
    // checked before it enters the semantic archive.
    let bytes = generated.to_bytes()?;
    J3dRebuildDocument::parse(&bytes)?;
    Ok(generated)
}

fn triangle_normal_y(vertices: [[f32; 3]; 3]) -> f32 {
    let ab = [
        vertices[1][0] - vertices[0][0],
        vertices[1][1] - vertices[0][1],
        vertices[1][2] - vertices[0][2],
    ];
    let ac = [
        vertices[2][0] - vertices[0][0],
        vertices[2][1] - vertices[0][1],
        vertices[2][2] - vertices[0][2],
    ];
    ab[2] * ac[0] - ab[0] * ac[2]
}

fn lifted_upward_render_vertices(triangle: GoopRenderTriangle) -> Option<[[f32; 3]; 3]> {
    const MIN_UPWARD_COMPONENT: f32 = 0.1;
    // A retail census found a modal +2 world-Y separation between map floors
    // and their pollution meshes (Bianco is about +1..2, Delfino/Monte/Ricco
    // are about +2, and Airport is higher). The runtime adds no transform, so
    // reproduce the conservative retail offset in the generated geometry.
    const SURFACE_Y_OFFSET: f32 = 2.0;

    let upward_component = if let Some(normals) = triangle.normals {
        let normalized = normals.map(normalize_vector);
        let average = normalized.iter().fold([0.0; 3], |mut average, normal| {
            for axis in 0..3 {
                average[axis] += normal[axis];
            }
            average
        });
        normalize_vector(average)[1]
    } else {
        let [a, b, c] = triangle.vertices;
        let ab = [b[0] - a[0], b[1] - a[1], b[2] - a[2]];
        let ac = [c[0] - a[0], c[1] - a[1], c[2] - a[2]];
        let gx_normal = normalize_vector([
            ab[1] * ac[2] - ab[2] * ac[1],
            ab[2] * ac[0] - ab[0] * ac[2],
            ab[0] * ac[1] - ab[1] * ac[0],
        ]);
        // Effective BMD previews retain GX's clockwise winding, so the
        // geometric normal points opposite the visible vertex normal.
        -gx_normal[1]
    };
    if upward_component <= MIN_UPWARD_COMPONENT {
        return None;
    }

    let mut vertices = triangle.vertices;
    for vertex in &mut vertices {
        vertex[1] += SURFACE_Y_OFFSET;
    }
    Some(vertices)
}

fn normalize_vector(vector: [f32; 3]) -> [f32; 3] {
    let length = vector_length(vector);
    if !length.is_finite() || length <= f32::EPSILON {
        [0.0; 3]
    } else {
        vector.map(|component| component / length)
    }
}

fn vector_length(vector: [f32; 3]) -> f32 {
    (vector[0] * vector[0] + vector[1] * vector[1] + vector[2] * vector[2]).sqrt()
}

fn clip_triangle_to_region(vertices: [[f32; 3]; 3], region: GoopRegion) -> Vec<[f32; 3]> {
    let mut polygon = vertices.to_vec();
    for (axis, boundary, keep_greater) in [
        (0, region.min_x, true),
        (0, region.max_x, false),
        (2, region.min_z, true),
        (2, region.max_z, false),
    ] {
        if polygon.is_empty() {
            break;
        }
        let input = std::mem::take(&mut polygon);
        let mut previous = *input.last().expect("non-empty clipped polygon");
        let mut previous_inside = if keep_greater {
            previous[axis] >= boundary
        } else {
            previous[axis] <= boundary
        };
        for current in input {
            let current_inside = if keep_greater {
                current[axis] >= boundary
            } else {
                current[axis] <= boundary
            };
            if current_inside != previous_inside {
                let denominator = current[axis] - previous[axis];
                if denominator.abs() > f32::EPSILON {
                    let t = (boundary - previous[axis]) / denominator;
                    polygon.push([
                        previous[0] + (current[0] - previous[0]) * t,
                        previous[1] + (current[1] - previous[1]) * t,
                        previous[2] + (current[2] - previous[2]) * t,
                    ]);
                }
            }
            if current_inside {
                polygon.push(current);
            }
            previous = current;
            previous_inside = current_inside;
        }
    }
    polygon
}

impl StageDocument {
    pub fn ensure_goop_authoring(&mut self) -> Result<&mut GoopAuthoringDocument> {
        if self.goop_authoring.is_none() {
            let Some(StageResourceDocument::PollutionMap(ymp)) =
                self.effective_resource_clone(GOOP_RESOURCE_PATH)?
            else {
                self.goop_authoring = Some(GoopAuthoringDocument::default());
                return Ok(self
                    .goop_authoring
                    .as_mut()
                    .expect("goop authoring inserted"));
            };
            let mut layers = Vec::with_capacity(ymp.layers.len());
            for (index, runtime) in ymp.layers.into_iter().enumerate() {
                let stem = pollution_stem(index, &self.stage_id);
                let bitmap_path = format!("map/pollution/{stem}.bmp");
                let bitmap = match self.effective_resource_clone(bitmap_path.as_bytes())? {
                    Some(StageResourceDocument::Bitmap(bitmap)) => Some(bitmap),
                    _ => None,
                };
                let plane = GoopPlane::from_runtime_code(runtime.flags)
                    .expect("all runtime plane codes have a lossless representation");
                layers.push(GoopLayerAuthoring {
                    id: format!("goop-layer-{index:02}"),
                    runtime_index: index,
                    origin: GoopLayerOrigin::Imported,
                    plane,
                    behavior: GoopBehavior::from_runtime_code(runtime.layer_type),
                    visible: true,
                    region: GoopRegion {
                        min_x: runtime.min_x,
                        min_z: runtime.min_z,
                        max_x: runtime.max_x,
                        max_z: runtime.max_z,
                    },
                    runtime,
                    bitmap,
                    generated_model: None,
                    style_source: None,
                    resource_stem: stem,
                    metadata_dirty: false,
                });
            }
            self.goop_authoring = Some(GoopAuthoringDocument {
                format_version: GOOP_AUTHORING_FORMAT_VERSION,
                layers,
                terrain_fingerprint: 0,
                stale: false,
            });
        }
        let authoring = self
            .goop_authoring
            .as_mut()
            .expect("goop authoring inserted");
        if authoring.format_version < GOOP_AUTHORING_FORMAT_VERSION {
            // Earlier generators used collision geometry for the visible BMD;
            // those meshes can be culled, displaced, or projected onto walls.
            // Mark every older generated layer for the normal template-backed
            // rebuild, which preserves/reprojects its authored mask.
            authoring.stale |= authoring
                .layers
                .iter()
                .any(|layer| layer.origin == GoopLayerOrigin::Generated);
            if !authoring
                .layers
                .iter()
                .any(|layer| layer.origin == GoopLayerOrigin::Generated)
            {
                authoring.format_version = GOOP_AUTHORING_FORMAT_VERSION;
            }
        }
        Ok(authoring)
    }

    pub fn compile_goop_authoring(&mut self) -> Result<()> {
        let Some(authoring) = self.goop_authoring.clone() else {
            return Ok(());
        };
        authoring.validate()?;
        if authoring
            .layers
            .iter()
            .any(|layer| layer.origin == GoopLayerOrigin::Generated || layer.metadata_dirty)
        {
            let compiled = match self.effective_resource_clone(GOOP_RESOURCE_PATH)? {
                Some(StageResourceDocument::PollutionMap(base)) => {
                    authoring.compiled_ymp_preserving(&base)?
                }
                _ => authoring.compiled_ymp()?,
            };
            self.upsert_authored_resource(
                GOOP_RESOURCE_PATH.to_vec(),
                StageResourceDocument::PollutionMap(compiled),
            );
        }
        for layer in authoring.layers {
            if let Some(bitmap) = layer.bitmap {
                self.upsert_authored_resource(
                    format!("map/pollution/{}.bmp", layer.resource_stem).into_bytes(),
                    StageResourceDocument::Bitmap(bitmap),
                );
            }
            if let Some(model) = layer.generated_model {
                self.upsert_authored_resource(
                    format!("map/pollution/{}.bmd", layer.resource_stem).into_bytes(),
                    StageResourceDocument::Model(model),
                );
            }
        }
        Ok(())
    }

    pub fn effective_terrain_fingerprint(&self) -> Result<u64> {
        let model = self
            .effective_resource_clone(b"map/map/map.bmd")?
            .map(|resource| resource.to_bytes())
            .transpose()?
            .unwrap_or_default();
        let collision = self
            .effective_resource_clone(b"map/map.col")?
            .or(self.effective_resource_clone(b"map/map/map.col")?)
            .map(|resource| resource.to_bytes())
            .transpose()?
            .unwrap_or_default();
        Ok(terrain_fingerprint(&model, &collision))
    }

    pub fn refresh_goop_stale_status(&mut self) -> Result<()> {
        let fingerprint = self.effective_terrain_fingerprint()?;
        if let Some(authoring) = &mut self.goop_authoring {
            if authoring
                .layers
                .iter()
                .any(|layer| layer.origin == GoopLayerOrigin::Generated)
                && authoring.terrain_fingerprint != 0
                && authoring.terrain_fingerprint != fingerprint
            {
                authoring.stale = true;
            }
        }
        Ok(())
    }
}

fn pollution_stem(index: usize, stage_id: &str) -> String {
    if stage_id.to_ascii_lowercase().starts_with("mare") {
        match index {
            7 => return "pollutionA".to_string(),
            8 => return "pollutionB".to_string(),
            _ => {}
        }
    }
    format!("pollution{index:02}")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn flat(size: f32, height: f32) -> Vec<GoopTerrainTriangle> {
        vec![
            GoopTerrainTriangle {
                vertices: [[0.0, height, 0.0], [size, height, 0.0], [0.0, height, size]],
            },
            GoopTerrainTriangle {
                vertices: [
                    [size, height, 0.0],
                    [size, height, size],
                    [0.0, height, size],
                ],
            },
        ]
    }

    fn render_flat(size: f32, height: f32) -> Vec<GoopRenderTriangle> {
        flat(size, height)
            .into_iter()
            .map(|triangle| GoopRenderTriangle {
                vertices: triangle.vertices,
                normals: Some([[0.0, 1.0, 0.0]; 3]),
            })
            .collect()
    }

    #[test]
    fn whole_region_is_cell_aligned_and_power_of_two() {
        let (region, width, height) = whole_terrain_region(&flat(300.0, 80.0)).unwrap();
        assert_eq!((width, height), (3, 3));
        assert_eq!(region.min_x, 0.0);
        assert_eq!(region.max_x, 320.0);
    }

    #[test]
    fn floor_generator_uses_tiled_depth_and_runtime_scale() {
        let triangles = flat(400.0, 125.0);
        let region = GoopRegion {
            min_x: 0.0,
            min_z: 0.0,
            max_x: 320.0,
            max_z: 160.0,
        };
        let (offset, depth_map) = generate_floor_depth_map(&triangles, region, 3, 2).unwrap();
        assert_eq!(offset, 120.0);
        let layer = YmpLayer {
            layer_type: 0,
            subtype: 0,
            flags: 0,
            reserved: 0,
            vertical_offset: offset,
            vertical_scale: 40.0,
            min_x: 0.0,
            min_z: 0.0,
            max_x: 320.0,
            max_z: 160.0,
            width_log2: 3,
            height_log2: 2,
            user_value: 0,
            map_offset: 0,
            depth_map,
        };
        assert_eq!(layer.depth_at(1, 1).unwrap(), 0);
    }

    #[test]
    fn floor_generator_selects_the_topmost_stacked_floor() {
        let mut triangles = flat(400.0, 80.0);
        triangles.extend(flat(400.0, 205.0));
        let region = GoopRegion {
            min_x: 0.0,
            min_z: 0.0,
            max_x: 320.0,
            max_z: 160.0,
        };
        let (offset, depth_map) = generate_floor_depth_map(&triangles, region, 3, 2).unwrap();
        assert_eq!(offset, 200.0);
        let mut layer = YmpLayer {
            layer_type: 0,
            subtype: 0,
            flags: 0,
            reserved: 0,
            vertical_offset: offset,
            vertical_scale: 40.0,
            min_x: region.min_x,
            min_z: region.min_z,
            max_x: region.max_x,
            max_z: region.max_z,
            width_log2: 3,
            height_log2: 2,
            user_value: 0,
            map_offset: 0,
            depth_map,
        };
        assert_eq!(layer.depth_at(1, 1).unwrap(), 0);
        layer.set_depth(1, 1, 12).unwrap();
        assert_eq!(layer.depth_at(1, 1).unwrap(), 12);
    }

    #[test]
    fn floor_generator_rejects_unrepresentable_vertical_span() {
        let mut triangles = vec![
            GoopTerrainTriangle {
                vertices: [
                    [-20.0, 0.0, -20.0],
                    [140.0, 0.0, -20.0],
                    [-20.0, 0.0, 200.0],
                ],
            },
            GoopTerrainTriangle {
                vertices: [
                    [140.0, 0.0, -20.0],
                    [140.0, 0.0, 200.0],
                    [-20.0, 0.0, 200.0],
                ],
            },
        ];
        triangles.extend([
            GoopTerrainTriangle {
                vertices: [
                    [160.0, 11_000.0, -20.0],
                    [400.0, 11_000.0, -20.0],
                    [160.0, 11_000.0, 200.0],
                ],
            },
            GoopTerrainTriangle {
                vertices: [
                    [400.0, 11_000.0, -20.0],
                    [400.0, 11_000.0, 200.0],
                    [160.0, 11_000.0, 200.0],
                ],
            },
        ]);
        let region = GoopRegion {
            min_x: 0.0,
            min_z: 0.0,
            max_x: 320.0,
            max_z: 160.0,
        };
        assert!(generate_floor_depth_map(&triangles, region, 3, 2).is_err());
    }

    #[test]
    fn pollution_model_keeps_template_material_and_replaces_texture_zero_with_i8() {
        let texture = GxEncodedTexture::encode_rgba(
            "retail_goop",
            &RgbaImage {
                width: 8,
                height: 4,
                pixels: vec![0; 8 * 4 * 4],
            },
            GxTextureEncodeOptions {
                encoding: GxTextureEncoding::Exact(GxTextureFormat::I8),
                ..GxTextureEncodeOptions::default()
            },
        )
        .unwrap();
        let template = compile_static_bmd3(&StaticModel {
            root_joint_name: "template".to_string(),
            meshes: vec![StaticModelMesh {
                name: "template".to_string(),
                material_index: 0,
                vertices: vec![
                    StaticModelVertex::new([0.0, 0.0, 0.0], [0.0, 1.0, 0.0]),
                    StaticModelVertex::new([320.0, 0.0, 0.0], [0.0, 1.0, 0.0]),
                    StaticModelVertex::new([0.0, 0.0, 160.0], [0.0, 1.0, 0.0]),
                ],
                triangles: vec![[0, 1, 2]],
            }],
            materials: vec![GxMaterial::default()],
            textures: vec![texture],
        })
        .unwrap();
        let region = GoopRegion {
            min_x: 0.0,
            min_z: 0.0,
            max_x: 320.0,
            max_z: 160.0,
        };
        let generated = generate_floor_pollution_model(
            &template,
            &render_flat(400.0, 0.0),
            region,
            16,
            8,
            false,
        )
        .unwrap();
        let template_material = template
            .sections
            .iter()
            .find(|section| matches!(section.data, J3dRebuildSectionData::Materials(_)))
            .unwrap();
        let generated_material = generated
            .sections
            .iter()
            .find(|section| matches!(section.data, J3dRebuildSectionData::Materials(_)))
            .unwrap();
        assert_eq!(generated_material, template_material);
        let first_texture = generated
            .sections
            .iter()
            .find_map(|section| match &section.data {
                J3dRebuildSectionData::Textures(textures) => textures.textures.first(),
                _ => None,
            })
            .unwrap();
        assert_eq!(first_texture.format, GxTextureFormat::I8 as u8);
        assert_eq!((first_texture.width, first_texture.height), (16, 8));
    }

    #[test]
    fn pollution_model_keeps_only_upward_render_faces_and_separates_them_from_the_map() {
        let texture = GxEncodedTexture::encode_rgba(
            "retail_goop",
            &RgbaImage {
                width: 8,
                height: 4,
                pixels: vec![0; 8 * 4 * 4],
            },
            GxTextureEncodeOptions {
                encoding: GxTextureEncoding::Exact(GxTextureFormat::I8),
                ..GxTextureEncodeOptions::default()
            },
        )
        .unwrap();
        let template = compile_static_bmd3(&StaticModel {
            root_joint_name: "template".to_string(),
            meshes: vec![StaticModelMesh {
                name: "template".to_string(),
                material_index: 0,
                vertices: vec![
                    StaticModelVertex::new([0.0, 0.0, 0.0], [0.0, 1.0, 0.0]),
                    StaticModelVertex::new([320.0, 0.0, 0.0], [0.0, 1.0, 0.0]),
                    StaticModelVertex::new([0.0, 0.0, 160.0], [0.0, 1.0, 0.0]),
                ],
                triangles: vec![[0, 2, 1]],
            }],
            materials: vec![GxMaterial::default()],
            textures: vec![texture],
        })
        .unwrap();
        let region = GoopRegion {
            min_x: 0.0,
            min_z: 0.0,
            max_x: 320.0,
            max_z: 160.0,
        };

        let upward = GoopRenderTriangle {
            vertices: [[0.0, 0.0, 0.0], [320.0, 0.0, 0.0], [0.0, 0.0, 160.0]],
            normals: Some([[0.0, 1.0, 0.0]; 3]),
        };
        let wall = GoopRenderTriangle {
            vertices: [[0.0, 0.0, 0.0], [0.0, 100.0, 0.0], [0.0, 0.0, 160.0]],
            normals: Some([[1.0, 0.0, 0.0]; 3]),
        };
        let generated =
            generate_floor_pollution_model(&template, &[upward, wall], region, 8, 4, false)
                .unwrap();
        let preview = sms_formats::J3dFile::parse(generated.to_bytes().unwrap())
            .unwrap()
            .geometry_preview()
            .unwrap();
        assert_eq!(preview.triangles.len(), 1);
        assert!(preview.triangles.iter().all(|triangle| {
            triangle_normal_y(triangle.vertices) < 0.0
                && triangle.cull_mode == Some(2)
                && triangle
                    .vertices
                    .iter()
                    .all(|vertex| (vertex[1] - 2.0).abs() <= f32::EPSILON)
        }));

        let underside = GoopRenderTriangle {
            vertices: [upward.vertices[0], upward.vertices[2], upward.vertices[1]],
            normals: Some([[0.0, -1.0, 0.0]; 3]),
        };
        assert!(
            generate_floor_pollution_model(&template, &[underside], region, 8, 4, false,).is_err()
        );
    }
}
