//! The Material Library: a library of named effects, dropped onto a material.
//!
//! Everything here targets a material. Shine is material state written into
//! MAT3; scroll and flipbook are animation files, but those are keyed by
//! material name too -- a BTK track says "this material's texture matrix, over
//! time". One concept, two mechanisms.
//!
//! See `docs/material-and-texture-animation.md`.
//!
//! A scaffold: the shapes are here, the panel that uses them is not, so nothing
//! below is reachable yet.

/// The slot the author has selected, drawn on every surface using it.
///
/// A material is not an object: `_0009mizu_1` in Ricco is the fountain basin
/// *and* a pool fourteen thousand units away. Which surfaces a name covers
/// cannot be read off a list, so it is shown on the model.
pub(super) const HIGHLIGHT_SELECTED: [u8; 4] = [0xE0, 0x7B, 0x1F, 0xFF];

/// What a drop would land on, while an effect is being dragged.
///
/// Distinct from the selection colour on purpose: orange answers *where is this
/// material*, purple answers *what am I about to change*. One colour doing both
/// leaves the author guessing which question is being answered.
pub(super) const HIGHLIGHT_DROP: [u8; 4] = [0x9A, 0x6F, 0xD8, 0xFF];

/// What the stage's model and animation edits were before the library wrote to
/// them.
///
/// `None` for a path means there was no edit there at all, and putting that
/// back means removing the library's rather than replacing it -- the stage then
/// falls back to the resource its archive already carries.
#[derive(Debug, Clone)]
pub(super) struct MaterialLibraryBaseline {
    pub(super) model: Option<sms_scene::StageResourceEdit>,
    pub(super) animation: Option<sms_scene::StageResourceEdit>,
}

/// An effect an author has put on one of this map's materials.
#[derive(Debug, Clone)]
pub(super) struct MaterialEffectAssignment {
    /// The material slot it landed on.
    pub(super) material: usize,
    /// Where the entry lives in `LIBRARY`: concept, then category, then entry.
    pub(super) concept: usize,
    pub(super) category: usize,
    pub(super) effect: usize,
}

impl MaterialEffectAssignment {
    pub(super) fn effect(&self) -> Option<&'static Sample> {
        runtime_library()
            .get(self.concept)?
            .categories
            .get(self.category)?
            .samples
            .get(self.effect)
    }
}

use crate::preview_types::PreviewRenderLayer as Layer;

/// Every layer the renderer draws, what to call it, and whether a click lands
/// on it out of the box.
///
/// The defaults say what is a surface and what is drawn over one -- a blob
/// shadow lies on the ground, a particle hangs over it, and the mirrored pass
/// is a second copy of the whole scene near the water. But which of those an
/// author wants to reach is their call, not a judgement to bake in: goop was
/// off here once because it is painted on top of things, and that quietly took
/// away a surface that had always been selectable.
pub(super) const PICK_LAYERS: [(Layer, &str, bool); 11] = [
    (Layer::Main, "Solid surfaces", true),
    (Layer::Water, "Water", true),
    (Layer::IndirectWater, "Water, indirect", true),
    (Layer::WaveFoam, "Wave foam", true),
    (Layer::MirrorSurface, "Mirror surfaces", true),
    (Layer::Goop, "Goop", true),
    (Layer::Shadow, "Shadows", false),
    (Layer::Particle, "Particles", false),
    (Layer::Heatwave, "Heat haze", false),
    (Layer::Sky, "Sky", false),
    (Layer::MirrorScene, "Mirrored scene copy", false),
];

/// Where a layer's toggle lives. Exhaustive on purpose: a layer added to the
/// renderer has to be given a place here rather than silently defaulting.
pub(super) fn layer_slot(layer: Layer) -> usize {
    match layer {
        Layer::Main => 0,
        Layer::Water => 1,
        Layer::IndirectWater => 2,
        Layer::WaveFoam => 3,
        Layer::MirrorSurface => 4,
        Layer::Goop => 5,
        Layer::Shadow => 6,
        Layer::Particle => 7,
        Layer::Heatwave => 8,
        Layer::Sky => 9,
        Layer::MirrorScene => 10,
    }
}

pub(super) fn default_pick_layers() -> [bool; PICK_LAYERS.len()] {
    std::array::from_fn(|slot| PICK_LAYERS[slot].2)
}

/// The stage's own model, and the animation that drives it.
const MATERIAL_LIBRARY_MAP_MODEL: &[u8] = b"map/map/map.bmd";
const MATERIAL_LIBRARY_MAP_ANIMATION: &[u8] = b"map/map/map.btk";

/// How often the surface under the pointer is resolved, in seconds.
const HOVER_PICK_INTERVAL_SECONDS: f64 = 0.12;

/// How far a highlighted surface is pulled toward the highlight colour.
///
/// Not opaque: an author needs to see *which* surface lit up, and a flat fill
/// hides the geometry that tells them.
const HIGHLIGHT_STRENGTH: f32 = 0.55;

fn highlight_rgba(rgba: [u8; 4]) -> [f32; 4] {
    [
        f32::from(rgba[0]) / 255.0,
        f32::from(rgba[1]) / 255.0,
        f32::from(rgba[2]) / 255.0,
        HIGHLIGHT_STRENGTH,
    ]
}

/// Which mechanism an effect reaches for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Mechanism {
    /// Texture scroll, scale and rotate. Written as a BTK beside the model.
    Scroll,
    /// A flipbook: which texture a slot samples, over time. Written as a BTP.
    /// Nothing offers one yet -- the harvest found 123 in the game, and they
    /// arrive with the sampled materials.
    #[allow(dead_code)]
    Pattern,
    /// Material state, written into MAT3. No file at all.
    Material,
}

impl Mechanism {
    pub(super) fn label(self) -> &'static str {
        match self {
            Self::Scroll => "BTK",
            Self::Pattern => "BTP",
            Self::Material => "MAT3",
        }
    }
}

/// One effect the library offers.
///
/// `reference` names the retail material this reproduces, so an effect is a
/// copy of something that shipped rather than an invention. Empty where there
/// is no retail equivalent.
#[derive(Debug, Clone, Copy)]
pub(super) struct Effect {
    pub(super) name: &'static str,
    pub(super) note: &'static str,
    pub(super) reference: &'static str,
    /// The stage the sample is taken from, and where in it. Applying opens this
    /// archive and installs the real material, so an effect is a copy of
    /// something that shipped rather than a description of it.
    pub(super) source_stage: &'static str,
    pub(super) source_model: &'static str,
    /// The animation that drives it there, if it has one. Empty where the
    /// material is state alone -- shine does not move.
    pub(super) source_animation: &'static str,
    /// How many of the surface's own texture slots to leave alone.
    ///
    /// Shading keeps one: asking for shine means wanting the wall you have to
    /// be shiny, not wanting somebody else's wall. Everything else installs the
    /// sample whole, because a fountain without the fountain's water is not the
    /// effect anybody asked for.
    pub(super) keep_texture_slots: usize,
}

/// A group of samples that dress one kind of thing.
///
/// `token` is the word Sunshine itself puts in a material's name -- `yuka` for
/// a floor, `kabe` for a wall, `hunsui` for a fountain. The harvest finds a
/// category's members by that word, so the categories are the game's own
/// vocabulary rather than a scheme laid over the top of it.
#[derive(Debug, Clone, Copy)]
pub(super) struct Category {
    pub(super) name: &'static str,
    pub(super) token: &'static str,
    pub(super) entries: &'static [Effect],
}

/// The three things an author reaches for.
///
/// Shading is how a surface takes light. Structures are the surfaces
/// themselves, the samples a stage is built out of. Effects are what moves --
/// water, fire, anything animated. A material can be more than one of these at
/// once, which is why they are three ways in rather than three folders.
#[derive(Debug, Clone, Copy)]
pub(super) struct Concept {
    pub(super) name: &'static str,
    pub(super) categories: &'static [Category],
}

const SHADING: &[Category] = &[
    Category {
        name: "Specular",
        token: "spec",
        entries: &[Effect {
            name: "Lit",
            note: "From the stage lights, computed per vertex",
            reference: "_m00kabe",
            source_stage: "dolpic0",
            source_model: "map/map/map.bmd",
            source_animation: "",
            keep_texture_slots: 1,
        }],
    },
    Category {
        name: "Reflection",
        token: "env",
        entries: &[
            Effect {
                name: "Metal",
                note: "Tracks the camera everywhere, with nothing shaping it",
                reference: "_env0",
                source_stage: "airport0",
                source_model: "map/map/map.bmd",
                source_animation: "",
                keep_texture_slots: 1,
            },
            Effect {
                name: "Roof",
                note: "Shaped by a painted mask: tile faces catch it, grout does not",
                reference: "_m_tras0",
                source_stage: "ricco0",
                source_model: "map/map/map.bmd",
                source_animation: "",
                keep_texture_slots: 1,
            },
        ],
    },
    Category {
        name: "Shine",
        token: "tekari",
        entries: &[Effect {
            name: "Window",
            note: "Masked to a bright band",
            reference: "_m_mado_tekari",
            source_stage: "dolpic0",
            source_model: "map/map/map.bmd",
            source_animation: "",
            keep_texture_slots: 1,
        }],
    },
];

const STRUCTURES: &[Category] = &[];

const EFFECTS: &[Category] = &[
    Category {
        name: "Fountains",
        token: "hunsui",
        entries: &[
            Effect {
                name: "Shower",
                note: "The falling dome",
                reference: "_0011hunsuimizu_1",
                source_stage: "ricco0",
                source_model: "map/map/map.bmd",
                source_animation: "map/map/map.btk",
                keep_texture_slots: 0,
            },
            Effect {
                name: "Foam",
                note: "A sheet two units above the basin",
                reference: "_0010sibuki1_1",
                source_stage: "ricco0",
                source_model: "map/map/map.bmd",
                source_animation: "map/map/map.btk",
                keep_texture_slots: 0,
            },
            Effect {
                name: "Basin",
                note: "The water it lands in",
                reference: "_0009mizu_1",
                source_stage: "ricco0",
                source_model: "map/map/map.bmd",
                source_animation: "map/map/map.btk",
                keep_texture_slots: 0,
            },
        ],
    },
    Category {
        name: "Sea",
        token: "umi",
        entries: &[
            Effect {
                name: "Surface",
                note: "Slow two-axis drift",
                reference: "_suimen_o",
                source_stage: "mare0",
                source_model: "map/map/map.bmd",
                source_animation: "map/map/map.btk",
                keep_texture_slots: 0,
            },
            Effect {
                name: "Refraction",
                note: "A second layer against the first",
                reference: "_SubwruMIzu1_NureMask",
                source_stage: "bianco0",
                source_model: "map/map/map.bmd",
                source_animation: "",
                keep_texture_slots: 0,
            },
        ],
    },
];

/// Curation over a harvest.
///
/// The harvest is every material in the retail game -- 23,392 of them across
/// 108 archives, read out of the models themselves rather than described. What
/// stays hand-written is this arrangement: which of the game's own words name a
/// category an author would look under, and which concept it belongs to.
/// `_0011hunsuimizu_1` is a fine key and a poor label.
pub(super) const LIBRARY: &[Concept] = &[
    Concept {
        name: "Shading",
        categories: SHADING,
    },
    Concept {
        name: "Structures",
        categories: STRUCTURES,
    },
    Concept {
        name: "Effects",
        categories: EFFECTS,
    },
];

/// One material of the open model, as the panel lists it.
///
/// Where these come from is the stage's own model rather than any one tool's
/// preview, so the panel works on map geometry as readily as on an actor.
#[derive(Debug, Clone)]
pub(super) struct MaterialSlot {
    /// The slot that stands for it -- the first one the scene loaded.
    pub(super) index: usize,
    /// Every slot wearing this name. A scene loads several models, and one
    /// authored material arrives once per model that uses it, so a pier can be
    /// two slots: the near end and the end across the map.
    pub(super) slots: Vec<usize>,
    pub(super) name: String,
    /// How many triangles carry it, across every slot.
    pub(super) triangles: usize,
    /// Whether the stage already animates it. Read from the preview, which
    /// loads the stage's own BTK and BTP.
    pub(super) animated: bool,
}
impl crate::SmsEditorApp {
    /// The materials of the stage's own model, with how much geometry each
    /// covers and what it already does.
    /// One pass over the triangles, not one pass per material.
    ///
    /// Counting per material walks the whole triangle list once for each of
    /// them, which on a map is materials times triangles -- tens of millions of
    /// iterations, every frame the panel draws. That, not the highlight, is
    /// what made the viewport crawl.
    pub(super) fn material_library_slots(&self) -> Vec<MaterialSlot> {
        let Some(preview) = self.model_preview.as_ref() else {
            return Vec::new();
        };
        let mut counts = vec![0usize; preview.materials.len()];
        for triangle in &preview.triangles {
            if let Some(index) = triangle.material_index {
                if let Some(count) = counts.get_mut(index) {
                    *count += 1;
                }
            }
        }
        // One row per material, not per slot. Listing slots separately made the
        // same pier appear twice, and picking its near end lit a row that was
        // not the row picking its far end lit -- which reads, correctly, as one
        // side of a surface being unclickable.
        let mut slots: Vec<MaterialSlot> = Vec::new();
        let mut by_name: std::collections::HashMap<&str, usize> = std::collections::HashMap::new();
        for (index, material) in preview.materials.iter().enumerate() {
            let triangles = counts[index];
            if triangles == 0 {
                continue;
            }
            // What the stage already does with it. The preview loads the
            // stage's own BTK and BTP, so this is read rather than guessed.
            let animated = preview
                .material_animation_bindings
                .get(index)
                .is_some_and(|bindings| !bindings.is_empty());
            match by_name.get(material.name.as_str()) {
                Some(position) => {
                    let slot: &mut MaterialSlot = &mut slots[*position];
                    slot.slots.push(index);
                    slot.triangles += triangles;
                    slot.animated |= animated;
                }
                None => {
                    by_name.insert(material.name.as_str(), slots.len());
                    slots.push(MaterialSlot {
                        index,
                        slots: vec![index],
                        name: material.name.clone(),
                        triangles,
                        animated,
                    });
                }
            }
        }
        slots
    }

    /// Every material slot sharing a slot's name.
    ///
    /// A map does not store one material per name: the same surface, authored
    /// once, arrives once per model that uses it. Those slots are one material
    /// as far as an author is concerned, so picking any of them picks all of
    /// them -- and the panel lists them as the single row they are.
    pub(super) fn material_library_group(&self, index: usize) -> Vec<usize> {
        let Some(preview) = self.model_preview.as_ref() else {
            return Vec::new();
        };
        let Some(name) = preview.materials.get(index).map(|material| &material.name) else {
            return Vec::new();
        };
        preview
            .materials
            .iter()
            .enumerate()
            .filter(|(_, material)| material.name == *name)
            .map(|(slot, _)| slot)
            .collect()
    }

    pub(super) fn material_library_panel(&mut self, ui: &mut egui::Ui) {
        // Escape backs out of a sample, the way it leaves anything else.
        if self.material_browser_inspecting.is_some()
            && ui.input(|input| input.key_pressed(egui::Key::Escape))
        {
            self.material_browser_inspecting = None;
        }
        if let Some(entry) = self.material_browser_inspecting {
            self.material_sample_panel(ui, entry);
            return;
        }
        ui.heading("Material Library");
        ui.label(
            egui::RichText::new("Effects dropped onto a material")
                .small()
                .color(egui::Color32::GRAY),
        );

        let slots = self.material_library_slots();
        if slots.is_empty() {
            ui.add_space(6.0);
            ui.label(
                egui::RichText::new(
                    "No stage model is loaded, so there are no materials to dress.",
                )
                .small()
                .weak(),
            );
            return;
        }

        let animated = slots.iter().filter(|slot| slot.animated).count();
        ui.add_space(8.0);
        ui.label(
            egui::RichText::new(format!(
                "{} material(s) \u{2014} {animated} already animated",
                slots.len()
            ))
            .small()
            .color(egui::Color32::GRAY),
        );

        // What the map already has, before anything is added to it. Nothing in
        // the editor said this until now, so the first sign of a clash was two
        // effects fighting in the game.
        if animated > 0 {
            ui.add_space(6.0);
            ui.label(egui::RichText::new("Already on this map").strong());
            for slot in slots.iter().filter(|slot| slot.animated) {
                ui.label(
                    egui::RichText::new(format!(
                        "{}  \u{2014}  {} triangles",
                        slot.name, slot.triangles
                    ))
                    .small()
                    .weak(),
                );
            }
        }

        ui.add_space(10.0);
        ui.separator();
        ui.label(egui::RichText::new("Material slots").strong());
        egui::ScrollArea::vertical()
            .max_height(220.0)
            .id_salt("material-library-slots")
            .show(ui, |ui| {
                for slot in &slots {
                    // Picked from either end of the map, it is the same row.
                    let selected = self
                        .material_library_selected
                        .is_some_and(|index| slot.slots.contains(&index));
                    let label = format!("{}  ({})", slot.name, slot.triangles);
                    let response =
                        ui.selectable_label(selected, label)
                            .on_hover_text(if slot.animated {
                                "Already animated by this stage"
                            } else {
                                "No effect on this material yet"
                            });
                    // A click in the viewport picks a slot the author never
                    // scrolled to, so the list comes to them rather than
                    // leaving them to hunt for what just lit up.
                    if selected && self.material_library_scroll_to_selected {
                        response.scroll_to_me(Some(egui::Align::Center));
                        self.material_library_scroll_to_selected = false;
                    }
                    if response.clicked() {
                        self.material_library_selected =
                            if selected { None } else { Some(slot.index) };
                        self.sync_material_library_highlight();
                    }
                }
            });

        if let Some(index) = self.material_library_selected {
            let parts = self.material_library_group(index).len();
            if parts > 1 {
                ui.add_space(4.0);
                ui.label(
                    egui::RichText::new(format!("Loaded as {parts} slots across this scene"))
                        .small()
                        .weak(),
                );
            }
        }

        ui.add_space(10.0);
        ui.separator();
        egui::CollapsingHeader::new("What a click can land on")
            .id_salt("material-library-layers")
            .show(ui, |ui| {
                ui.label(
                    egui::RichText::new("A click takes the nearest surface on an enabled layer.")
                        .small()
                        .color(egui::Color32::GRAY),
                );
                for (slot, (_, label, default)) in PICK_LAYERS.iter().enumerate() {
                    ui.checkbox(&mut self.material_library_layers[slot], *label)
                        .on_hover_text(if *default {
                            "A surface in its own right"
                        } else {
                            "Drawn over a surface, or a copy of one"
                        });
                }
                if ui.button("Reset").clicked() {
                    self.material_library_layers = default_pick_layers();
                }
            });

        if !self.material_library_assignments.is_empty() {
            ui.add_space(10.0);
            ui.separator();
            ui.label(egui::RichText::new("Dropped on this map").strong());
            let mut remove: Option<usize> = None;
            for (position, assignment) in self.material_library_assignments.iter().enumerate() {
                let Some(effect) = assignment.effect() else {
                    continue;
                };
                let material = slots
                    .iter()
                    .find(|slot| slot.index == assignment.material)
                    .map(|slot| slot.name.as_str())
                    .unwrap_or("(missing material)");
                ui.horizontal(|ui| {
                    if ui
                        .small_button("\u{00D7}")
                        .on_hover_text("Take this effect back off")
                        .clicked()
                    {
                        remove = Some(position);
                    }
                    ui.label(
                        egui::RichText::new(format!("{}  \u{2192}  {material}", effect.name))
                            .small(),
                    );
                });
            }
            if let Some(position) = remove {
                self.material_library_assignments.remove(position);
            }
            ui.add_space(4.0);
            if ui
                .button("Reset all")
                .on_hover_text(
                    "Put this stage's model and animation back to before any effect was applied",
                )
                .clicked()
            {
                self.reset_material_library();
            }
        }

        ui.add_space(10.0);
        ui.separator();
        ui.label(egui::RichText::new("Library").strong());
        // Where a click would land. Shown rather than assumed: an effect
        // applied to the wrong material is only visible once it moves.
        let target = self.material_library_selected.and_then(|index| {
            let name = self
                .model_preview
                .as_ref()?
                .materials
                .get(index)?
                .name
                .clone();
            Some((index, name))
        });
        match &target {
            Some((_, name)) => {
                ui.label(
                    egui::RichText::new(format!("Applies to {name}"))
                        .small()
                        .color(egui::Color32::from_rgb(0xE0, 0x7B, 0x1F)),
                );
            }
            None => {
                ui.label(
                    egui::RichText::new("Select a surface to apply an effect to it")
                        .small()
                        .color(egui::Color32::GRAY),
                );
            }
        }
        ui.add_space(10.0);
        ui.separator();
        ui.label(
            egui::RichText::new(
                "The library itself is in the Content browser below, while this tool is open.",
            )
            .small()
            .weak(),
        );
    }
}

impl crate::SmsEditorApp {
    /// The material of the frontmost surface under the pointer.
    ///
    /// Picking is by material rather than by object because that is what an
    /// effect attaches to. Clicking the fountain selects `_0011hunsuimizu_1`,
    /// and every other surface wearing that name lights up with it -- which is
    /// the point, since a name can cover geometry the author cannot see.
    pub(super) fn material_at_screen_position(
        &self,
        rect: egui::Rect,
        position: egui::Pos2,
    ) -> Option<usize> {
        if !rect.contains(position) {
            return None;
        }
        let preview = self.model_preview.as_ref()?;
        let size = crate::software_renderer::framebuffer_size_for_rect(rect);
        let x = (position.x - rect.left()) * size[0] as f32 / rect.width().max(1.0);
        let y = (position.y - rect.top()) * size[1] as f32 / rect.height().max(1.0);
        preview
            .triangles
            .iter()
            .filter(|triangle| self.material_library_layers[layer_slot(triangle.render_layer)])
            .filter_map(|triangle| {
                let material_index = triangle.material_index?;
                let projected = self.project_preview_triangle(rect, size, triangle)?;
                let depth = crate::software_renderer::projected_triangle_depth_at_point(
                    projected.screen,
                    x,
                    y,
                )?;
                Some((depth, material_index))
            })
            .min_by(|left, right| left.0.total_cmp(&right.0))
            .map(|(_, material_index)| material_index)
    }

    /// Pushes the selection and hover tints down to the renderer.
    ///
    /// Only when they change: everything below this is a uniform write on the
    /// GPU material the draw already binds, so a highlighted material costs
    /// nothing per frame no matter how much geometry wears it.
    pub(super) fn sync_material_library_highlight(&mut self) {
        let wanted = if self.tool == crate::EditorTool::Material {
            let mut wanted: Vec<(usize, [f32; 4])> = Vec::new();
            // The whole material, which is every slot wearing its name. Not
            // every material: the group is one surface the author authored
            // once, so this lights both ends of a pier without touching any
            // other material that happens to look like it.
            // Selection first so it wins where the two land on the same slot:
            // `set_material_highlight` takes the first match.
            if let Some(index) = self.material_library_selected {
                let colour = highlight_rgba(HIGHLIGHT_SELECTED);
                wanted.extend(
                    self.material_library_group(index)
                        .into_iter()
                        .map(|slot| (slot, colour)),
                );
            }
            if let Some(index) = self.material_library_hovered {
                let colour = highlight_rgba(HIGHLIGHT_DROP);
                let fresh: Vec<usize> = self
                    .material_library_group(index)
                    .into_iter()
                    .filter(|slot| !wanted.iter().any(|(taken, _)| taken == slot))
                    .collect();
                wanted.extend(fresh.into_iter().map(|slot| (slot, colour)));
            }
            // Purple is "what a drop would land on". It answers that whether
            // the author is dragging an effect or just reading the map, so it
            // follows the pointer rather than waiting for a drag to start.
            wanted
        } else {
            Vec::new()
        };
        // Nothing is previewed synthetically any more. An effect used to be a
        // scroll rate applied over whatever texture the surface already had,
        // because installing the material was not built yet; now the real
        // material and its real animation are installed, and a stand-in
        // scrolling on top of them would only be a second, wrong motion.
        if let Some(gpu_viewport) = &self.gpu_viewport {
            gpu_viewport.set_material_preview_scroll(&[]);
        }

        // Pushed every frame rather than only on change. The renderer compares
        // per material and writes only what differs, so this is a short loop
        // over a few hundred float4s -- and unlike a cached "already pushed"
        // flag it survives the scene being rebuilt underneath it, which is what
        // made the highlight come and go.
        if let Some(gpu_viewport) = &self.gpu_viewport {
            gpu_viewport.set_material_highlight(&wanted);
        }
    }

    /// Click to select a material, hover to preview which surfaces one covers.
    ///
    /// Returns whether the viewport's own selection should be skipped: with
    /// this tool a click means "which material is this", never "select this
    /// object".
    pub(super) fn handle_material_library_viewport_input(
        &mut self,
        ui: &egui::Ui,
        rect: egui::Rect,
        response: &egui::Response,
    ) -> bool {
        if self.tool != crate::EditorTool::Material {
            self.material_library_hovered = None;
            self.material_library_hover_pos = None;
            return false;
        }
        // A pick walks every triangle in the map, so hover is resolved on a
        // budget rather than per frame: only where the pointer has actually
        // moved, and at most a few times a second. That is quick enough to feel
        // immediate and leaves the frame to the renderer.
        let now = ui.input(|input| input.time);
        match response.hover_pos() {
            Some(hover) => {
                let moved = self
                    .material_library_hover_pos
                    .is_none_or(|last| (hover - last).length() > 3.0);
                let due = now - self.material_library_hover_time >= HOVER_PICK_INTERVAL_SECONDS;
                if moved && due {
                    self.material_library_hover_pos = Some(hover);
                    self.material_library_hover_time = now;
                    self.material_library_hovered = self.material_at_screen_position(rect, hover);
                }
            }
            None => {
                self.material_library_hover_pos = None;
                self.material_library_hovered = None;
            }
        }
        if !response.clicked() {
            return false;
        }
        let Some(position) = response.interact_pointer_pos() else {
            return false;
        };
        // A click on empty space clears, rather than leaving a stale slot
        // selected somewhere off screen.
        self.material_library_selected = self.material_at_screen_position(rect, position);
        self.material_library_scroll_to_selected = self.material_library_selected.is_some();
        match self.material_library_selected {
            Some(index) => {
                let name = self
                    .model_preview
                    .as_ref()
                    .and_then(|preview| preview.materials.get(index))
                    .map(|material| material.name.clone())
                    .unwrap_or_else(|| format!("material {index}"));
                self.log.push(format!("Material: {name}"));
            }
            None => self
                .log
                .push("No material under the pointer there.".to_string()),
        }
        true
    }

    /// Takes an effect dropped onto the viewport and puts it on the surface
    /// under the pointer.
    ///
    /// Nothing is written to the stage here. A drop records what the author
    /// wants; turning that into a BTK beside the model is a separate, explicit
    /// step, so an experiment costs nothing to undo.
    pub(super) fn handle_material_library_drop(
        &mut self,
        rect: egui::Rect,
        response: &egui::Response,
    ) -> bool {
        // While a drag is in flight the surface it would land on is the one
        // worth showing, whichever tool is active.
        if let Some(pointer) = response.hover_pos() {
            if response
                .dnd_hover_payload::<crate::MaterialEffectDragPayload>()
                .is_some()
            {
                self.material_library_hovered = self.material_at_screen_position(rect, pointer);
            }
        }
        let Some(payload) = response.dnd_release_payload::<crate::MaterialEffectDragPayload>()
        else {
            return false;
        };
        let Some(pointer) = response
            .interact_pointer_pos()
            .or_else(|| response.hover_pos())
        else {
            return false;
        };
        let Some(material) = self.material_at_screen_position(rect, pointer) else {
            self.log
                .push("Dropped on nothing: no surface under the pointer.".to_string());
            return true;
        };
        let assignment = MaterialEffectAssignment {
            material,
            category: payload.category,
            concept: payload.concept,
            effect: payload.effect,
        };
        let name = assignment
            .effect()
            .map(|effect| effect.name.clone())
            .unwrap_or_else(|| "effect".to_string());
        let material_name = self
            .model_preview
            .as_ref()
            .and_then(|preview| preview.materials.get(material))
            .map(|material| material.name.clone())
            .unwrap_or_else(|| format!("material {material}"));
        self.material_library_assignments.push(assignment);
        self.material_library_selected = Some(material);
        self.material_library_scroll_to_selected = true;
        self.log.push(format!("{name} \u{2192} {material_name}"));
        true
    }

    /// Puts the stage's model and animation back to what they were before any
    /// effect was applied.
    ///
    /// Installing rewrites a material in place and merges tracks into the
    /// stage's own animation, and neither can be picked apart afterwards: the
    /// sample's TEV stages are indistinguishable from any other once they are
    /// in the table. So reset restores the whole of both resources from the
    /// snapshot, which also puts back anything applied on top of them.
    pub(super) fn reset_material_library(&mut self) {
        let applied = self.material_library_assignments.len();
        if !self.restore_material_library_baseline() {
            self.material_library_assignments.clear();
            return;
        }
        self.material_library_baseline = None;
        self.material_library_assignments.clear();
        self.log.push(format!(
            "Material Library reset: {applied} effect(s) removed"
        ));
        self.rebuild_model_preview_from_document_async();
    }

    /// Puts the two resources back to the snapshot, keeping the snapshot.
    ///
    /// Returns whether there was anything to put back.
    fn restore_material_library_baseline(&mut self) -> bool {
        let Some(baseline) = self.material_library_baseline.clone() else {
            return false;
        };
        let Some(document) = self.document.as_mut() else {
            return false;
        };
        for (path, edit) in [
            (MATERIAL_LIBRARY_MAP_MODEL, baseline.model),
            (MATERIAL_LIBRARY_MAP_ANIMATION, baseline.animation),
        ] {
            document
                .archive_edits
                .resources
                .retain(|candidate| candidate.raw_resource_path != path);
            if let Some(edit) = edit {
                document.archive_edits.resources.push(edit);
            }
        }
        true
    }

    /// Takes the last applied effect back off.
    ///
    /// An install cannot be picked apart once it has landed -- a sample's TEV
    /// stages are indistinguishable from the material's own once they are in
    /// the table, and its tracks are merged into the stage's animation. So the
    /// way back is to restore the snapshot and apply everything again except
    /// the last, which is exact where unpicking would be guesswork.
    pub(super) fn undo_material_library(&mut self) -> bool {
        if self.material_library_assignments.is_empty() {
            return false;
        }
        let mut remaining = self.material_library_assignments.clone();
        let undone = remaining.pop();
        if !self.restore_material_library_baseline() {
            return false;
        }
        self.material_library_assignments.clear();
        for assignment in remaining {
            let Some(entry) = assignment.effect().cloned() else {
                continue;
            };
            match self.install_material_effect(assignment.material, &entry, &[]) {
                Ok(_) => self.material_library_assignments.push(assignment),
                Err(error) => self
                    .log
                    .push(format!("could not reapply {}: {error}", entry.name)),
            }
        }
        if let Some(name) = undone
            .and_then(|assignment| assignment.effect())
            .map(|effect| effect.name.clone())
        {
            self.log.push(format!("Took {name} back off"));
        }
        self.rebuild_model_preview_from_document_async();
        true
    }
}

impl crate::SmsEditorApp {
    /// Installs a sampled material onto one of this stage's, with whatever
    /// animation drives it.
    ///
    /// Nothing here is a stand-in. It opens the retail archive the sample came
    /// from, lifts the material out of that stage's own model, and writes it
    /// into this stage's -- textures, TEV stages, blend state and all -- then
    /// retargets the animation that drove it and folds it into this stage's
    /// own. What the game loads afterwards is the effect, not an imitation.
    fn install_material_effect(
        &mut self,
        material: usize,
        effect: &Sample,
        skip_stages: &[usize],
    ) -> Result<String, String> {
        let target_material = self
            .model_preview
            .as_ref()
            .and_then(|preview| preview.materials.get(material))
            .map(|material| material.name.clone())
            .ok_or_else(|| "that material is no longer loaded".to_string())?;
        let base_root = self
            .document
            .as_ref()
            .map(|document| document.base_root.clone())
            .ok_or_else(|| "no stage is open".to_string())?;

        let archive_path = base_root
            .join("files")
            .join("data")
            .join("scene")
            .join(format!("{}.szs", effect.stage));
        let raw = std::fs::read(&archive_path)
            .map_err(|error| format!("read {}: {error}", archive_path.display()))?;
        let decompressed =
            sms_formats::decode_yaz0(&raw).map_err(|error| format!("decompress: {error}"))?;
        let archive = sms_formats::RarcArchive::parse(&decompressed)
            .map_err(|error| format!("read the archive: {error}"))?;
        let files = archive
            .files()
            .map_err(|error| format!("list the archive: {error}"))?;
        let entry_bytes = |wanted: &str| -> Result<Vec<u8>, String> {
            let file = files
                .iter()
                .find(|file| file.path.eq_ignore_ascii_case(wanted))
                .ok_or_else(|| format!("{} has no {wanted}", effect.stage))?;
            archive
                .file_bytes_raw(&file.raw_path)
                .map_err(|error| format!("read {wanted}: {error}"))
        };

        let source_model = sms_formats::J3dRebuildDocument::parse(entry_bytes(&effect.model)?)
            .map_err(|error| format!("read the sampled model: {error}"))?;

        // The animation is prepared before anything is written, so a sample
        // whose tracks cannot be retargeted changes nothing at all.
        let mut retargeted = None;
        if !effect.animation.is_empty() {
            let mut sampled =
                sms_formats::J3dAnimationRebuildDocument::parse(entry_bytes(&effect.animation)?)
                    .map_err(|error| format!("read the sampled animation: {error}"))?;
            if sampled
                .retain_material_bindings_named(&effect.material)
                .map_err(|error| format!("take the sampled tracks: {error}"))?
                .is_some_and(|kept| kept > 0)
            {
                sampled
                    .rename_material_bindings(&effect.material, &target_material)
                    .map_err(|error| format!("retarget the sampled tracks: {error}"))?;
                retargeted = Some(sampled);
            }
        }

        let document = self
            .document
            .as_mut()
            .ok_or_else(|| "no stage is open".to_string())?;
        // Once, before the first install: what these two paths looked like
        // before the library existed in this session.
        if self.material_library_baseline.is_none() {
            let taken = |path: &[u8]| {
                document
                    .archive_edits
                    .resources
                    .iter()
                    .find(|edit| edit.raw_resource_path == path)
                    .cloned()
            };
            self.material_library_baseline = Some(MaterialLibraryBaseline {
                model: taken(MATERIAL_LIBRARY_MAP_MODEL),
                animation: taken(MATERIAL_LIBRARY_MAP_ANIMATION),
            });
        }
        let Some(sms_scene::StageResourceDocument::Model(mut model)) = document
            .effective_resource_clone(MATERIAL_LIBRARY_MAP_MODEL)
            .map_err(|error| format!("read this stage's model: {error}"))?
        else {
            return Err("this stage has no map model to dress".to_string());
        };
        let report = model
            .install_material(&sms_formats::MaterialInstallRequest {
                target_material: &target_material,
                source: &source_model,
                source_material: &effect.material,
                texture_prefix: "lib_",
                keep_target_texture_slots: effect.keep_texture_slots,
                skip_stages,
            })
            .map_err(|error| format!("install the material: {error}"))?;
        document.archive_edits.upsert_resource(
            MATERIAL_LIBRARY_MAP_MODEL.to_vec(),
            sms_scene::StageResourceDocument::Model(model),
        );

        let mut animated = false;
        if let Some(sampled) = retargeted {
            // Into the animation the stage already loads. A stage reads one
            // animation per model, so an effect cannot bring a file of its own.
            let existing = document
                .effective_resource_clone(MATERIAL_LIBRARY_MAP_ANIMATION)
                .map_err(|error| format!("read this stage's animation: {error}"))?;
            let merged = match existing {
                Some(sms_scene::StageResourceDocument::Animation(mut existing)) => {
                    existing
                        .merge_animation(&sampled)
                        .map_err(|error| format!("merge the tracks: {error}"))?;
                    *existing
                }
                _ => sampled,
            };
            document.archive_edits.upsert_resource(
                MATERIAL_LIBRARY_MAP_ANIMATION.to_vec(),
                sms_scene::StageResourceDocument::Animation(Box::new(merged)),
            );
            animated = true;
        }

        Ok(format!(
            "{} onto {target_material}: {} texture(s), {} TEV stage(s){}",
            &effect.name,
            report.textures_added,
            report.tev_stages,
            if animated { ", with its animation" } else { "" }
        ))
    }
}

/// The Material Library's own content browser.
///
/// Deliberately the same furniture as the content browser it stands in for --
/// a source tree down the left, cards to the right, a search box above both --
/// because an author already knows how to read that. What changes is what the
/// tree holds: not where a file lives, but what a surface *is*.
const MATERIAL_BROWSER_CARD_HEIGHT: f32 = 96.0;

/// One accent per concept, so a card says which of the three it came from
/// before its name is read.
fn concept_accent(concept: usize) -> egui::Color32 {
    // Taken in order, so a concept added to the taxonomy gets a colour without
    // anyone having to choose one.
    const ACCENTS: [egui::Color32; 6] = [
        egui::Color32::from_rgb(0x48, 0xB0, 0xBE), // water
        egui::Color32::from_rgb(0xE8, 0xB5, 0x4A), // lit
        egui::Color32::from_rgb(0x7F, 0xA6, 0x63), // ground
        egui::Color32::from_rgb(0xC8, 0x7C, 0xA8), // skin
        egui::Color32::from_rgb(0x8A, 0x9C, 0xD8), // sky
        egui::Color32::from_rgb(0xD8, 0x9A, 0x70), // flare
    ];
    ACCENTS[concept % ACCENTS.len()]
}

/// Breaks a description at word boundaries so a card can show two lines of it.
fn wrap_note(note: &str, width: usize) -> Vec<String> {
    let mut lines = Vec::new();
    let mut current = String::new();
    for word in note.split_whitespace() {
        if !current.is_empty() && current.len() + 1 + word.len() > width {
            lines.push(std::mem::take(&mut current));
            if lines.len() == 2 {
                // Two lines is what the card has. The rest is in the panel.
                if let Some(last) = lines.last_mut() {
                    last.push('\u{2026}');
                }
                return lines;
            }
        }
        if !current.is_empty() {
            current.push(' ');
        }
        current.push_str(word);
    }
    if !current.is_empty() {
        lines.push(current);
    }
    lines
}

fn material_browser_card(
    ui: &mut egui::Ui,
    size: egui::Vec2,
    accent: egui::Color32,
    enabled: bool,
    sample: &Sample,
) -> egui::Response {
    let (rect, response) = ui.allocate_exact_size(size, egui::Sense::click());
    let fill = if !enabled {
        egui::Color32::from_rgb(33, 36, 37)
    } else if response.hovered() {
        egui::Color32::from_rgb(48, 53, 54)
    } else {
        egui::Color32::from_rgb(37, 41, 42)
    };
    ui.painter().rect_filled(rect, 7.0, fill);
    ui.painter().rect_stroke(
        rect,
        7.0,
        egui::Stroke::new(1.0, egui::Color32::from_rgb(62, 68, 69)),
        egui::StrokeKind::Inside,
    );
    // The concept's colour as a rail down the left edge, the way the content
    // browser marks a source.
    ui.painter().rect_filled(
        egui::Rect::from_min_max(
            rect.left_top(),
            egui::pos2(rect.left() + 4.0, rect.bottom()),
        ),
        7.0,
        if enabled {
            accent
        } else {
            accent.gamma_multiply(0.4)
        },
    );

    let inner = rect.shrink2(egui::vec2(12.0, 9.0));
    let dim = |color: egui::Color32| {
        if enabled {
            color
        } else {
            color.gamma_multiply(0.5)
        }
    };
    ui.painter().text(
        inner.left_top(),
        egui::Align2::LEFT_TOP,
        &sample.name,
        egui::FontId::proportional(14.0),
        dim(egui::Color32::from_rgb(226, 232, 233)),
    );
    // A harvested sample has no note, so it says what it is made of instead:
    // the numbers are the honest description when nobody has written one.
    // A hand-written note where there is one, and otherwise the description
    // the library worked out from the material itself.
    let note = if sample.note.is_empty() {
        sample.description.clone()
    } else {
        sample.note.clone()
    };
    let mut line = egui::pos2(inner.left(), inner.top() + 20.0);
    for part in wrap_note(&note, 34) {
        ui.painter().text(
            line,
            egui::Align2::LEFT_TOP,
            part,
            egui::FontId::proportional(11.5),
            dim(egui::Color32::from_rgb(150, 158, 159)),
        );
        line.y += 14.0;
    }
    // What it is made of, and where it shipped: the two things that say whether
    // this is the sample you meant.
    ui.painter().text(
        egui::pos2(inner.left(), inner.bottom() - 26.0),
        egui::Align2::LEFT_TOP,
        format!("{}   {}", sample.mechanism().label(), sample.stage),
        egui::FontId::monospace(10.5),
        dim(accent),
    );
    ui.painter().text(
        egui::pos2(inner.left(), inner.bottom() - 12.0),
        egui::Align2::LEFT_TOP,
        if sample.keep_texture_slots > 0 {
            format!(
                "{}  \u{00B7}  keeps this surface's texture",
                sample.material
            )
        } else {
            sample.material.clone()
        },
        egui::FontId::monospace(10.0),
        dim(egui::Color32::from_rgb(120, 128, 129)),
    );
    response
}

impl crate::SmsEditorApp {
    /// Replaces the content browser while the Material Library is the tool.
    pub(super) fn material_browser_panel(&mut self, ui: &mut egui::Ui) {
        let target = self.material_library_selected.and_then(|index| {
            let name = self
                .model_preview
                .as_ref()?
                .materials
                .get(index)?
                .name
                .clone();
            Some((index, name))
        });

        ui.horizontal(|ui| {
            ui.label(egui::RichText::new("Material Library").strong());
            ui.add(
                egui::TextEdit::singleline(&mut self.material_browser_query)
                    .hint_text("Search samples")
                    .desired_width(240.0),
            );
            if !self.material_browser_query.is_empty() && ui.small_button("Clear").clicked() {
                self.material_browser_query.clear();
            }
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                match &target {
                    Some((_, name)) => ui.label(
                        egui::RichText::new(format!("Applies to {name}"))
                            .small()
                            .color(egui::Color32::from_rgb(0xE0, 0x7B, 0x1F)),
                    ),
                    None => ui.label(
                        egui::RichText::new("Select a surface in the viewport")
                            .small()
                            .color(egui::Color32::GRAY),
                    ),
                };
            });
        });
        ui.separator();

        let body = ui.available_size();
        ui.allocate_ui_with_layout(body, egui::Layout::left_to_right(egui::Align::Min), |ui| {
            self.material_browser_tree(ui);
            ui.separator();
            let rest = ui.available_size();
            ui.allocate_ui_with_layout(rest, egui::Layout::top_down(egui::Align::Min), |ui| {
                self.material_browser_results(ui, target);
            });
        });
    }

    /// Concepts as roots, their categories beneath -- the same shape as PROJECT
    /// CONTENT and its folders.
    fn material_browser_tree(&mut self, ui: &mut egui::Ui) {
        ui.allocate_ui_with_layout(
            egui::vec2(200.0, ui.available_height()),
            egui::Layout::top_down(egui::Align::Min),
            |ui| {
                egui::ScrollArea::vertical()
                    .id_salt("material-browser-tree")
                    .auto_shrink([false, false])
                    .show(ui, |ui| {
                        for (concept_index, concept) in runtime_library().iter().enumerate() {
                            let expanded = self.material_browser_expanded.contains(&concept_index);
                            let accent = concept_accent(concept_index);
                            if ui
                                .add(
                                    egui::Button::new(
                                        egui::RichText::new(format!(
                                            "{} {}",
                                            if expanded { "▾" } else { "▸" },
                                            concept.name.to_uppercase()
                                        ))
                                        .small()
                                        .color(accent),
                                    )
                                    .frame(false),
                                )
                                .on_hover_text(concept.note)
                                .clicked()
                            {
                                if !self.material_browser_expanded.remove(&concept_index) {
                                    self.material_browser_expanded.insert(concept_index);
                                }
                            }
                            if !expanded {
                                continue;
                            }
                            for (category_index, category) in concept.categories.iter().enumerate()
                            {
                                let selected = self.material_browser_category
                                    == Some((concept_index, category_index));
                                ui.horizontal(|ui| {
                                    ui.add_space(8.0);
                                    if ui
                                        .selectable_label(
                                            selected,
                                            format!(
                                                "{}  ({})",
                                                category.name,
                                                category.samples.len()
                                            ),
                                        )
                                        .on_hover_text(if category.token.is_empty() {
                                            "Filed by what these are made of, not by name"
                                                .to_string()
                                        } else {
                                            format!(
                                                "Named \u{201C}{}\u{201D} in the game",
                                                category.token
                                            )
                                        })
                                        .clicked()
                                    {
                                        self.material_browser_category =
                                            Some((concept_index, category_index));
                                    }
                                });
                            }
                            ui.add_space(6.0);
                        }
                    });
            },
        );
    }

    fn material_browser_results(&mut self, ui: &mut egui::Ui, _target: Option<(usize, String)>) {
        // A search reaches across every concept; without one the tree decides.
        // With neither, nothing is listed: eighteen thousand cards is not a
        // landing page, it is a wall.
        let query = self.material_browser_query.trim().to_ascii_lowercase();
        let mut shown: Vec<(usize, usize, usize, &'static Sample)> = Vec::new();
        for (concept_index, concept) in runtime_library().iter().enumerate() {
            for (category_index, category) in concept.categories.iter().enumerate() {
                let chosen = self
                    .material_browser_category
                    .is_some_and(|chosen| chosen == (concept_index, category_index));
                if query.is_empty() && !chosen {
                    continue;
                }
                for (sample_index, sample) in category.samples.iter().enumerate() {
                    let matches = query.is_empty()
                        || sample.name.to_ascii_lowercase().contains(&query)
                        || sample.material.to_ascii_lowercase().contains(&query)
                        || sample.model.to_ascii_lowercase().contains(&query)
                        || sample.stage.to_ascii_lowercase().contains(&query)
                        || category.name.to_ascii_lowercase().contains(&query);
                    if matches {
                        shown.push((concept_index, category_index, sample_index, sample));
                    }
                }
            }
        }

        if shown.is_empty() {
            ui.add_space(12.0);
            ui.label(
                egui::RichText::new(if query.is_empty() {
                    "Pick a category, or search across all of them"
                } else {
                    "Nothing matches that"
                })
                .small()
                .weak(),
            );
            return;
        }

        ui.label(
            egui::RichText::new(format!("{} sample(s)", shown.len()))
                .small()
                .color(egui::Color32::GRAY),
        );
        let columns = ((ui.available_width() / 260.0).floor() as usize).max(1);
        let card_width = (ui.available_width() - 8.0) / columns as f32 - 8.0;
        let rows = shown.len().div_ceil(columns);
        let mut inspect = None;
        egui::ScrollArea::vertical()
            .id_salt("material-browser-grid")
            .auto_shrink([false, false])
            .show_rows(ui, MATERIAL_BROWSER_CARD_HEIGHT + 8.0, rows, |ui, range| {
                for row in range {
                    ui.horizontal(|ui| {
                        let start = row * columns;
                        let end = (start + columns).min(shown.len());
                        for (concept, category, index, sample) in &shown[start..end] {
                            let response = material_browser_card(
                                ui,
                                egui::vec2(card_width, MATERIAL_BROWSER_CARD_HEIGHT),
                                concept_accent(*concept),
                                true,
                                sample,
                            )
                            .on_hover_text("Look at what this sample is made of");
                            if response.clicked() {
                                inspect = Some((*concept, *category, *index));
                            }
                        }
                    });
                }
            });
        if let Some(entry) = inspect {
            self.material_browser_inspecting = Some(entry);
        }
    }
}

/// One texture a sample carries, and what the material does with it.
pub(super) struct SampleTexture {
    pub(super) name: String,
    pub(super) width: u16,
    pub(super) height: u16,
    pub(super) format: u8,
    /// What it is *for*, worked out from the texgen feeding the stage that
    /// samples it: a normal-sourced coordinate is a reflection, and an
    /// intensity-only texture read through ordinary UVs is a mask.
    pub(super) role: &'static str,
    pub(super) image: egui::ColorImage,
}

/// One stage of the material's TEV chain, in the order the hardware runs them.
pub(super) struct SampleStage {
    pub(super) texture: Option<String>,
    pub(super) coordinate: &'static str,
}

/// A horizontal slice of the atlas, and how much shine the mask allows there.
///
/// Where an atlas holds several surfaces -- roof tiles, shutters, walls --
/// these are the numbers that say which of them the sample actually lets
/// reflect. Naming the bands is left to whoever is looking: the tool can
/// measure a strip, not recognise a roof.
pub(super) struct SampleBand {
    pub(super) top: usize,
    pub(super) mask_mean: f32,
    /// How closely the mask follows the colour underneath it. Near +1 means it
    /// was taken from the artwork; a negative value means someone painted
    /// against it.
    pub(super) correlation: f32,
    pub(super) image: egui::ColorImage,
}

/// What a sample is made of, read out of the stage it ships in.
pub(super) struct SampleBreakdown {
    pub(super) key: String,
    pub(super) tex_gens: Vec<u8>,
    pub(super) stages: Vec<SampleStage>,
    pub(super) textures: Vec<SampleTexture>,
    pub(super) bands: Vec<SampleBand>,
    pub(super) handles: Vec<egui::TextureHandle>,
    pub(super) band_handles: Vec<egui::TextureHandle>,
    /// Which stages will be installed. All of them, until an author says
    /// otherwise -- a sample is whole unless it is deliberately taken apart.
    pub(super) stage_enabled: Vec<bool>,
}

/// GX texgen sources.
fn tex_gen_source_label(source: u8) -> &'static str {
    match source {
        0 => "position",
        1 => "normal",
        2 => "binormal",
        3 => "tangent",
        4..=11 => "UVs",
        _ => "other",
    }
}

fn texture_format_label(format: u8) -> &'static str {
    match format {
        0 => "I4",
        1 => "I8",
        2 => "IA4",
        3 => "IA8",
        4 => "RGB565",
        5 => "RGB5A3",
        6 => "RGBA8",
        8 => "C4",
        9 => "C8",
        10 => "C14X2",
        14 => "CMPR",
        _ => "?",
    }
}

/// Splits an atlas into bands and measures the mask against the colour.
fn sample_bands(colour: &SampleTexture, mask: &SampleTexture) -> Vec<SampleBand> {
    const BANDS: usize = 16;
    let (mw, mh) = (mask.width as usize, mask.height as usize);
    let (cw, ch) = (colour.width as usize, colour.height as usize);
    if mh < BANDS || ch < BANDS || mw == 0 || cw == 0 {
        return Vec::new();
    }
    let band_h = mh / BANDS;
    let mut bands = Vec::new();
    for band in 0..BANDS {
        let mut mask_values = Vec::new();
        let mut colour_values = Vec::new();
        for y in band * band_h..(band + 1) * band_h {
            for x in 0..mw {
                mask_values.push(mask.image.pixels[y * mw + x].r() as f32);
                // The same place on the colour sheet, whatever the two sizes.
                let cx = x * cw / mw;
                let cy = y * ch / mh;
                let pixel = colour.image.pixels[cy * cw + cx];
                colour_values.push(
                    0.299 * pixel.r() as f32 + 0.587 * pixel.g() as f32 + 0.114 * pixel.b() as f32,
                );
            }
        }
        let count = mask_values.len() as f32;
        let mask_mean = mask_values.iter().sum::<f32>() / count;
        let colour_mean = colour_values.iter().sum::<f32>() / count;
        let covariance: f32 = mask_values
            .iter()
            .zip(&colour_values)
            .map(|(m, c)| (m - mask_mean) * (c - colour_mean))
            .sum();
        let mask_spread = mask_values
            .iter()
            .map(|m| (m - mask_mean).powi(2))
            .sum::<f32>()
            .sqrt();
        let colour_spread = colour_values
            .iter()
            .map(|c| (c - colour_mean).powi(2))
            .sum::<f32>()
            .sqrt();
        let correlation = if mask_spread > 0.0 && colour_spread > 0.0 {
            covariance / (mask_spread * colour_spread)
        } else {
            0.0
        };

        // A strip of the colour sheet, so a row of numbers has a picture.
        let top = band * band_h * ch / mh;
        let bottom = ((band + 1) * band_h * ch / mh).min(ch);
        let mut strip = egui::ColorImage::new(
            [cw, bottom - top],
            vec![egui::Color32::TRANSPARENT; cw * (bottom - top)],
        );
        for y in top..bottom {
            for x in 0..cw {
                strip.pixels[(y - top) * cw + x] = colour.image.pixels[y * cw + x];
            }
        }
        bands.push(SampleBand {
            top,
            mask_mean,
            correlation,
            image: strip,
        });
    }
    bands
}

impl crate::SmsEditorApp {
    /// Reads a sample out of the stage it ships in, with its textures decoded.
    ///
    /// The same archive the install reads from, so what is shown is what would
    /// land -- not a description kept alongside it that can drift.
    fn load_sample_breakdown(&self, effect: &Sample) -> Result<SampleBreakdown, String> {
        let base_root = self
            .document
            .as_ref()
            .map(|document| document.base_root.clone())
            .ok_or_else(|| "no stage is open".to_string())?;
        let archive_path = base_root
            .join("files")
            .join("data")
            .join("scene")
            .join(format!("{}.szs", effect.stage));
        let raw = std::fs::read(&archive_path)
            .map_err(|error| format!("read {}: {error}", archive_path.display()))?;
        let decompressed =
            sms_formats::decode_yaz0(&raw).map_err(|error| format!("decompress: {error}"))?;
        let archive = sms_formats::RarcArchive::parse(&decompressed)
            .map_err(|error| format!("read the archive: {error}"))?;
        let files = archive
            .files()
            .map_err(|error| format!("list the archive: {error}"))?;
        let file = files
            .iter()
            .find(|file| file.path.eq_ignore_ascii_case(&effect.model))
            .ok_or_else(|| format!("{} has no {}", effect.stage, effect.model))?;
        let bytes = archive
            .file_bytes_raw(&file.raw_path)
            .map_err(|error| format!("read the model: {error}"))?;
        let preview = sms_formats::J3dFile::parse(&bytes)
            .and_then(|model| model.geometry_preview())
            .map_err(|error| format!("decode the model: {error}"))?;
        let material = preview
            .materials
            .iter()
            .find(|material| material.name == effect.material)
            .ok_or_else(|| format!("{} has no {}", effect.stage, effect.material))?;

        let mut textures = Vec::new();
        for (slot, index) in material.texture_indices.iter().enumerate() {
            let Some(index) = index else { continue };
            let Some(texture) = preview.textures.get(*index) else {
                continue;
            };
            let source = material
                .tev_stages
                .iter()
                .find(|stage| stage.order.tex_map == Some(slot as u8))
                .and_then(|stage| stage.order.tex_coord)
                .and_then(|coord| material.tex_gens.get(coord as usize))
                .map(|gen| gen.source);
            let intensity_only = matches!(texture.format, 0 | 1);
            let role = match source {
                Some(1) => "reflection, followed by the normal",
                _ if intensity_only && slot > 0 => "mask, painted over the UVs",
                _ if slot == 0 => "colour",
                _ => "second layer",
            };
            textures.push(SampleTexture {
                name: texture.name.clone(),
                width: texture.width,
                height: texture.height,
                format: texture.format,
                role,
                image: egui::ColorImage::from_rgba_unmultiplied(
                    [texture.width as usize, texture.height as usize],
                    &texture.rgba,
                ),
            });
        }

        let stage_count = material.tev_stages.len();
        let stages = material
            .tev_stages
            .iter()
            .map(|stage| SampleStage {
                texture: stage
                    .order
                    .tex_map
                    .and_then(|map| {
                        material
                            .texture_indices
                            .get(map as usize)
                            .copied()
                            .flatten()
                    })
                    .and_then(|index| preview.textures.get(index))
                    .map(|texture| texture.name.clone()),
                coordinate: stage
                    .order
                    .tex_coord
                    .and_then(|coord| material.tex_gens.get(coord as usize))
                    .map(|gen| tex_gen_source_label(gen.source))
                    .unwrap_or("none"),
            })
            .collect();

        // The measurement only means something where a mask sits over a colour
        // sheet, which is what a shine mask is.
        let bands = match (
            textures.first(),
            textures
                .iter()
                .find(|texture| texture.role.starts_with("mask")),
        ) {
            (Some(colour), Some(mask)) => sample_bands(colour, mask),
            _ => Vec::new(),
        };

        Ok(SampleBreakdown {
            key: format!("{}/{}", effect.stage, effect.material),
            tex_gens: material
                .tex_gens
                .iter()
                .take(material.tex_gen_count as usize)
                .map(|gen| gen.source)
                .collect(),
            stages,
            textures,
            bands,
            handles: Vec::new(),
            band_handles: Vec::new(),
            stage_enabled: vec![true; stage_count],
        })
    }

    /// The panel a preset takes over while it is being looked at.
    pub(super) fn material_sample_panel(
        &mut self,
        ui: &mut egui::Ui,
        entry: (usize, usize, usize),
    ) {
        let Some(effect) = runtime_library()
            .get(entry.0)
            .and_then(|concept| concept.categories.get(entry.1))
            .and_then(|category| category.samples.get(entry.2))
        else {
            self.material_browser_inspecting = None;
            return;
        };
        let key = format!("{}/{}", effect.stage, effect.material);
        let accent = concept_accent(entry.0);

        let target = self.material_library_selected.and_then(|index| {
            let name = self
                .model_preview
                .as_ref()?
                .materials
                .get(index)?
                .name
                .clone();
            Some((index, name))
        });
        ui.horizontal(|ui| {
            if ui.button("\u{2190} Back").clicked() {
                self.material_browser_inspecting = None;
            }
            ui.label(egui::RichText::new(&effect.name).strong());
        });
        ui.horizontal(|ui| {
            let apply = ui
                .add_enabled(
                    target.is_some(),
                    egui::Button::new(match &target {
                        Some((_, name)) => format!("Put this on {name}"),
                        None => "Select a surface first".to_string(),
                    }),
                )
                .clicked();
            if apply {
                if let Some((material, name)) = target.clone() {
                    let skipped = self
                        .material_sample
                        .as_ref()
                        .map(|sample| {
                            sample
                                .stage_enabled
                                .iter()
                                .enumerate()
                                .filter(|(_, enabled)| !**enabled)
                                .map(|(index, _)| index)
                                .collect::<Vec<_>>()
                        })
                        .unwrap_or_default();
                    match self.install_material_effect(material, &effect, &skipped) {
                        Ok(report) => {
                            self.material_library_assignments
                                .push(MaterialEffectAssignment {
                                    material,
                                    concept: entry.0,
                                    category: entry.1,
                                    effect: entry.2,
                                });
                            self.log.push(report);
                            self.sync_material_library_highlight();
                            self.rebuild_model_preview_from_document_async();
                        }
                        Err(error) => self
                            .log
                            .push(format!("{} could not go on {name}: {error}", effect.name)),
                    }
                }
            }
            if effect.keep_texture_slots > 0 {
                ui.label(
                    egui::RichText::new("keeps the surface's own texture")
                        .small()
                        .weak(),
                );
            }
        });
        ui.label(
            egui::RichText::new(&effect.note)
                .small()
                .color(egui::Color32::GRAY),
        );
        ui.label(
            egui::RichText::new(format!(
                "{}  \u{00B7}  {}  \u{00B7}  {}",
                effect.mechanism().label(),
                effect.stage,
                effect.material
            ))
            .small()
            .monospace()
            .color(accent),
        );
        // Where it actually lives, which is the thing a name rarely says: a
        // material called `_mizubashira1r` is the Wiggler's sand column because
        // the model it sits in is called `bosshanachan/sunabashira`.
        ui.label(
            egui::RichText::new(&effect.model)
                .small()
                .monospace()
                .weak(),
        );
        if !effect.description.is_empty() {
            ui.label(egui::RichText::new(&effect.description).small());
        }
        if effect.stages > 1 {
            ui.label(
                egui::RichText::new(format!("Ships in {} stages", effect.stages))
                    .small()
                    .weak(),
            );
        }

        // Read once and kept: opening an archive and decoding its textures is
        // not something to do on every frame the panel draws.
        if self
            .material_sample
            .as_ref()
            .is_none_or(|loaded| loaded.key != key)
        {
            match self.load_sample_breakdown(&effect) {
                Ok(mut breakdown) => {
                    breakdown.handles = breakdown
                        .textures
                        .iter()
                        .map(|texture| {
                            ui.ctx().load_texture(
                                format!("sample-{key}-{}", texture.name),
                                texture.image.clone(),
                                egui::TextureOptions::NEAREST,
                            )
                        })
                        .collect();
                    breakdown.band_handles = breakdown
                        .bands
                        .iter()
                        .map(|band| {
                            ui.ctx().load_texture(
                                format!("sample-band-{key}-{}", band.top),
                                band.image.clone(),
                                egui::TextureOptions::NEAREST,
                            )
                        })
                        .collect();
                    self.material_sample = Some(breakdown);
                }
                Err(error) => {
                    ui.add_space(8.0);
                    ui.label(
                        egui::RichText::new(format!("Could not read the sample: {error}"))
                            .small()
                            .color(egui::Color32::from_rgb(0xD0, 0x7A, 0x6A)),
                    );
                    return;
                }
            }
        }
        let Some(breakdown) = self.material_sample.as_ref() else {
            return;
        };

        let mut toggled: Option<(usize, bool)> = None;
        let mut repaint: Option<usize> = None;
        egui::ScrollArea::vertical()
            .id_salt("material-sample-breakdown")
            .auto_shrink([false, false])
            .show(ui, |ui| {
                ui.add_space(8.0);
                ui.separator();
                ui.label(egui::RichText::new("Layers").strong());
                for (index, (texture, handle)) in breakdown
                    .textures
                    .iter()
                    .zip(&breakdown.handles)
                    .enumerate()
                {
                    ui.add_space(6.0);
                    ui.horizontal_top(|ui| {
                        // Fitted to a readable height and kept in proportion:
                        // an atlas is tall enough to fill the panel on its own.
                        let height = 132.0_f32.min(texture.height as f32 * 2.0).max(24.0);
                        let width = height / texture.height.max(1) as f32 * texture.width as f32;
                        ui.image((handle.id(), egui::vec2(width, height)));
                        ui.vertical(|ui| {
                            ui.label(egui::RichText::new(&texture.name).small().monospace());
                            ui.label(
                                egui::RichText::new(format!(
                                    "{}\u{00D7}{}  {}",
                                    texture.width,
                                    texture.height,
                                    texture_format_label(texture.format)
                                ))
                                .small()
                                .weak(),
                            );
                            ui.label(egui::RichText::new(texture.role).small().color(accent));
                            if ui
                                .small_button("Repaint\u{2026}")
                                .on_hover_text(
                                    "Replace this texture in your stage from a PNG, in its own \
                                     format and size",
                                )
                                .clicked()
                            {
                                repaint = Some(index);
                            }
                        });
                    });
                }

                ui.add_space(10.0);
                ui.separator();
                ui.label(egui::RichText::new("TEV chain").strong());
                ui.label(
                    egui::RichText::new(format!(
                        "{} stage(s), {} texgen(s)",
                        breakdown.stages.len(),
                        breakdown.tex_gens.len()
                    ))
                    .small()
                    .color(egui::Color32::GRAY),
                );
                for (index, stage) in breakdown.stages.iter().enumerate() {
                    let mut enabled = breakdown.stage_enabled.get(index).copied().unwrap_or(true);
                    ui.horizontal(|ui| {
                        // Off leaves the stage behind at install, and the ones
                        // after it close the gap.
                        if ui
                            .checkbox(&mut enabled, "")
                            .on_hover_text("Install this stage")
                            .changed()
                        {
                            toggled = Some((index, enabled));
                        }
                        ui.label(
                            egui::RichText::new(format!(
                                "{index}  {:22}  via {}",
                                stage.texture.as_deref().unwrap_or("\u{2014}"),
                                stage.coordinate
                            ))
                            .small()
                            .monospace()
                            .color(if enabled {
                                egui::Color32::from_rgb(150, 158, 159)
                            } else {
                                egui::Color32::from_rgb(96, 102, 103)
                            }),
                        );
                    });
                }

                if breakdown.bands.is_empty() {
                    return;
                }
                ui.add_space(10.0);
                ui.separator();
                ui.label(egui::RichText::new("Across the atlas").strong());
                ui.label(
                    egui::RichText::new(
                        "How much shine the mask allows, band by band, and how closely it \
                         follows the artwork underneath",
                    )
                    .small()
                    .color(egui::Color32::GRAY),
                );
                for (band, handle) in breakdown.bands.iter().zip(&breakdown.band_handles) {
                    ui.horizontal(|ui| {
                        ui.image((handle.id(), egui::vec2(56.0, 18.0)));
                        ui.label(
                            egui::RichText::new(format!("{:5.0}", band.mask_mean))
                                .small()
                                .monospace()
                                .color(if band.mask_mean > 60.0 {
                                    accent
                                } else {
                                    egui::Color32::GRAY
                                }),
                        );
                        // Bright where the mask was taken from the artwork,
                        // dim where someone painted against it.
                        ui.label(
                            egui::RichText::new(format!("{:+.2}", band.correlation))
                                .small()
                                .monospace()
                                .color(if band.correlation < 0.0 {
                                    egui::Color32::from_rgb(0xD0, 0x7A, 0x6A)
                                } else {
                                    egui::Color32::GRAY
                                }),
                        );
                    });
                }
            });

        if let Some(index) = repaint {
            // Cloned out first: repainting borrows the app, and the sample it
            // reads from lives inside it.
            let texture = self
                .material_sample
                .as_ref()
                .and_then(|sample| sample.textures.get(index))
                .map(|texture| SampleTexture {
                    name: texture.name.clone(),
                    width: texture.width,
                    height: texture.height,
                    format: texture.format,
                    role: texture.role,
                    image: egui::ColorImage::default(),
                });
            if let Some(texture) = texture {
                match self.repaint_sample_layer(&texture) {
                    Ok(report) if report.is_empty() => {}
                    Ok(report) => {
                        self.log.push(report);
                        // The picture changed, so the decoded copy shown here
                        // is stale.
                        self.material_sample = None;
                    }
                    Err(error) => self.log.push(format!("Could not repaint: {error}")),
                }
            }
        }
        if let Some((index, enabled)) = toggled {
            if let Some(sample) = self.material_sample.as_mut() {
                if let Some(slot) = sample.stage_enabled.get_mut(index) {
                    *slot = enabled;
                }
            }
        }
    }
}

/// Every material the game ships, sorted, as generated by
/// `sms-xtask material-library`.
const MATERIAL_LIBRARY_INDEX: &str = include_str!("material_library_index.json");

/// One material an author can put on a surface.
///
/// Curated and harvested samples are the same thing here: a name, where to find
/// it, and how much of it to take. The curated ones only carry a friendlier
/// name and a note explaining what they are for.
#[derive(Debug, Clone)]
pub(super) struct Sample {
    /// What it is, in words: "Boss Wiggler sand column water column". Worked out
    /// from the model it lives in and its own name when the library was built,
    /// because `_mizubashira1r` is a key and not a name.
    pub(super) name: String,
    pub(super) note: String,
    /// The material's own name in the stage it comes from.
    pub(super) material: String,
    pub(super) stage: String,
    pub(super) model: String,
    /// The animation that drives it there, empty where it has none.
    pub(super) animation: String,
    pub(super) keep_texture_slots: usize,
    /// What it is made of, in a sentence, worked out when the library was
    /// generated. Every sample has one, which is more than could be said for a
    /// list where only the handful somebody wrote about meant anything.
    pub(super) description: String,
    /// How many stages ship it. A material in sixty-eight of them is the game's
    /// general answer to something; one in a single stage was made for that
    /// stage.
    pub(super) stages: usize,
}

impl Sample {
    pub(super) fn mechanism(&self) -> Mechanism {
        if self.animation.ends_with(".btp") {
            Mechanism::Pattern
        } else if self.animation.is_empty() {
            Mechanism::Material
        } else {
            Mechanism::Scroll
        }
    }
}

pub(super) struct RuntimeCategory {
    pub(super) name: String,
    pub(super) token: String,
    pub(super) samples: Vec<Sample>,
}

pub(super) struct RuntimeConcept {
    pub(super) name: String,
    pub(super) note: &'static str,
    pub(super) categories: Vec<RuntimeCategory>,
}

/// Read once, on the first frame that needs it.
///
/// Two and a half megabytes of JSON parsed at startup would be two and a half
/// megabytes parsed for every author who never opens this tool.
static RUNTIME_LIBRARY: std::sync::OnceLock<Vec<RuntimeConcept>> = std::sync::OnceLock::new();

pub(super) fn runtime_library() -> &'static [RuntimeConcept] {
    RUNTIME_LIBRARY.get_or_init(build_runtime_library)
}

fn build_runtime_library() -> Vec<RuntimeConcept> {
    let parsed: serde_json::Value = match serde_json::from_str(MATERIAL_LIBRARY_INDEX) {
        Ok(parsed) => parsed,
        // The hand-written samples still work without the index, which is what
        // an author would rather have than an empty tool.
        Err(_) => serde_json::json!({ "concepts": [] }),
    };
    let mut concepts: Vec<RuntimeConcept> = Vec::new();
    for concept in parsed["concepts"].as_array().into_iter().flatten() {
        let name = concept["name"].as_str().unwrap_or_default().to_string();
        let note = match name.as_str() {
            "Shading" => "How a surface takes light",
            "Structures" => "The surfaces a stage is built out of",
            "Effects" => "What moves",
            "Characters" => "Everything wearing a face",
            "Sky" => "What a stage is wrapped in",
            "Sprites" => "Drawn flat, in front of everything",
            _ => "",
        };
        // Shading is added to a surface rather than replacing it, so its
        // samples keep the picture already there.
        let keep = usize::from(name == "Shading");
        let mut categories = Vec::new();
        for category in concept["categories"].as_array().into_iter().flatten() {
            let samples = category["samples"]
                .as_array()
                .into_iter()
                .flatten()
                .map(|sample| Sample {
                    name: sample["readable"]
                        .as_str()
                        .filter(|readable| !readable.is_empty())
                        .or_else(|| sample["name"].as_str())
                        .unwrap_or_default()
                        .to_string(),
                    note: String::new(),
                    material: sample["name"].as_str().unwrap_or_default().to_string(),
                    stage: sample["stage"].as_str().unwrap_or_default().to_string(),
                    model: sample["model"].as_str().unwrap_or_default().to_string(),
                    animation: sample["animation"].as_str().unwrap_or_default().to_string(),
                    keep_texture_slots: keep,
                    description: sample["description"]
                        .as_str()
                        .unwrap_or_default()
                        .to_string(),
                    stages: sample["stages"].as_array().map(Vec::len).unwrap_or(1),
                })
                .collect();
            categories.push(RuntimeCategory {
                name: category["name"].as_str().unwrap_or_default().to_string(),
                token: category["token"].as_str().unwrap_or_default().to_string(),
                samples,
            });
        }
        concepts.push(RuntimeConcept {
            name,
            note,
            categories,
        });
    }

    // The hand-written ones go in front of the category they belong to, so a
    // named sample with an explanation is what an author meets first.
    for concept in LIBRARY {
        for category in concept.categories {
            for effect in category.entries {
                let sample = Sample {
                    name: effect.name.to_string(),
                    note: effect.note.to_string(),
                    material: effect.reference.to_string(),
                    stage: effect.source_stage.to_string(),
                    model: effect.source_model.to_string(),
                    animation: effect.source_animation.to_string(),
                    keep_texture_slots: effect.keep_texture_slots,
                    description: String::new(),
                    stages: 0,
                };
                let Some(target) = concepts
                    .iter_mut()
                    .find(|runtime| runtime.name == concept.name)
                else {
                    continue;
                };
                match target
                    .categories
                    .iter_mut()
                    .find(|runtime| runtime.name == category.name)
                {
                    Some(runtime) => {
                        // The harvested copy of the same material would only be
                        // the same sample without its explanation.
                        runtime
                            .samples
                            .retain(|existing| existing.material != sample.material);
                        runtime.samples.insert(0, sample);
                    }
                    None => target.categories.insert(
                        0,
                        RuntimeCategory {
                            name: category.name.to_string(),
                            token: category.token.to_string(),
                            samples: vec![sample],
                        },
                    ),
                }
            }
        }
    }
    concepts
}

impl crate::SmsEditorApp {
    /// Replaces one of a sample's textures in this stage with a PNG.
    ///
    /// The picture goes back in the format it came out in -- an I4 mask
    /// re-encodes to I4, a CMPR sheet to CMPR -- because a texture's format is
    /// part of how its material reads it, and a mask that arrives as RGBA8 is
    /// four times the size for no more detail.
    ///
    /// It writes to the texture as it exists in *this* stage, which is the copy
    /// installing brought across. The retail archive is never written to.
    fn repaint_sample_layer(&mut self, texture: &SampleTexture) -> Result<String, String> {
        let Some(path) = rfd::FileDialog::new()
            .add_filter("PNG image", &["png"])
            .set_title(format!("Repaint {}", texture.name))
            .pick_file()
        else {
            return Ok(String::new());
        };
        let painted = image::open(&path)
            .map_err(|error| format!("read {}: {error}", path.display()))?
            .to_rgba8();
        if painted.width() != u32::from(texture.width)
            || painted.height() != u32::from(texture.height)
        {
            return Err(format!(
                "that image is {}x{}, and this texture is {}x{}",
                painted.width(),
                painted.height(),
                texture.width,
                texture.height
            ));
        }
        // Stored bottom-up, the same flip the mask tool does on the way in.
        let flipped = image::imageops::flip_vertical(&painted);
        let rgba = sms_formats::RgbaImage::new(
            texture.width,
            texture.height,
            flipped.pixels().flat_map(|pixel| pixel.0).collect(),
        )
        .map_err(|error| format!("stage the image: {error}"))?;

        let format = sms_formats::GxTextureFormat::ALL
            .into_iter()
            .find(|candidate| *candidate as u8 == texture.format)
            .ok_or_else(|| format!("unknown texture format {}", texture.format))?;
        let mut options = sms_formats::GxTextureEncodeOptions::default();
        options.encoding = sms_formats::GxTextureEncoding::Exact(format);
        let bti = sms_formats::GxEncodedTexture::encode_rgba(texture.name.clone(), &rgba, options)
            .and_then(|encoded| encoded.to_bti())
            .map_err(|error| format!("encode as {format:?}: {error}"))?;

        let document = self
            .document
            .as_mut()
            .ok_or_else(|| "no stage is open".to_string())?;
        let Some(sms_scene::StageResourceDocument::Model(mut model)) = document
            .effective_resource_clone(MATERIAL_LIBRARY_MAP_MODEL)
            .map_err(|error| format!("read this stage's model: {error}"))?
        else {
            return Err("this stage has no map model".to_string());
        };
        // Installing prefixes what it brings across, so the stage's copy is the
        // prefixed one -- but a stage that already had this texture keeps its
        // own name for it.
        let imported = format!("lib_{}", texture.name);
        let target = if model.texture_names().iter().any(|name| *name == imported) {
            imported
        } else if model
            .texture_names()
            .iter()
            .any(|name| *name == texture.name)
        {
            texture.name.clone()
        } else {
            return Err(format!(
                "{} is not in this stage yet -- apply the sample first",
                texture.name
            ));
        };
        let replaced = model
            .replace_named_texture_from_bti(&target, &bti)
            .map_err(|error| format!("replace {target}: {error}"))?;
        document.archive_edits.upsert_resource(
            MATERIAL_LIBRARY_MAP_MODEL.to_vec(),
            sms_scene::StageResourceDocument::Model(model),
        );
        self.rebuild_model_preview_from_document_async();
        Ok(format!(
            "Repainted {target} from {} ({replaced} copy/copies, {format:?})",
            path.file_name()
                .map(|name| name.to_string_lossy().to_string())
                .unwrap_or_default()
        ))
    }
}
