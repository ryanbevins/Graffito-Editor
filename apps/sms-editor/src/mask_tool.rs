//! The Mask Tool: authoring washable goop masks on enemy actor models.
//!
//! This is a scaffold. The intended workflow, in phases, is:
//!
//! 1. **Pick an actor** from every enemy actor in the catalog, and load its
//!    model into a viewport (BrawlBox's Advanced Model Editor is the visual
//!    reference: an actor dropdown over a live 3D model).
//! 2. **Paint by projection** onto the model surface -- brush strokes ray-cast
//!    onto the mesh (reusing the triangle BVH), resolved to the model's mask UV
//!    set, accumulated into a grayscale mask texture.
//! 3. **Author the mask into the material.** The washable goop effect is a per
//!    -pixel TEV comparison the game already runs on wired actors:
//!    `visible = polmask(UV1) > K0_A`, where `K0_A` is a scalar the enemy's
//!    class drives (HP for StayPakkun). The mask the user paints is that
//!    grayscale `polmask` -- its intensity is the wash order. Colour comes from
//!    a second texture on the same UV. Both goop textures ride one UV set
//!    (UV1); there is no third UV.
//!
//! Actor tiers (verified from the decomp/model data), ascending in cost:
//! - **Already wired** (StayPakkun, Stu): the mask/compare/UV/scalar all exist
//!   -- author the textures and it works, no code.
//! - **Un-wired** (Blooper / `TGesso`): no mask UV, no compare TEV stage, no
//!   scalar driver. Needs a generated UV, the compare stage authored into the
//!   material, and a DOL scalar driver for the wash animation.
//!
//! The scaffold below stands up the actor picker and model summary. The paint
//! surface, UV work, and material authoring are the phases that follow.
//!
//! # Target: a BrawlBox-style preview window
//!
//! The tool opens as its own window (in the style of BrawlBox's Advanced Model
//! Editor): an actor dropdown over a live 3D model, with a **UV-layer menu**
//! down the side. The layer you select drives what the model shows:
//!
//! - **UV0** selected -> the model renders clean (its normal body skin).
//! - **the goop UV** selected -> the model renders coated in goop, composited
//!   through that UV: the colour map over the body, masked by the goop mask.
//!
//! A **"Generate goop map + mask"** button seeds the goop UV with example
//! content -- for the first version, a **rainbow** colour map and the retail
//! **32x32 StayPakkun mask** (`H_ma_polmask1_i4`, extractable from pakun.bmd).
//! So on a Blooper (`TGesso`) you would see it fully coated in rainbow goop.
//!
//! The **"Play full cycle"** button then simulates the wash on that coated
//! preview: it sweeps the threshold `K0_A` from full coverage to clean, and the
//! preview evaluates `visible = mask > K0_A` per pixel -- the crisp recede,
//! following the painted mask's gradient, exactly as the game does it. Bright
//! mask paint clings, dark clears first.
//!
//! Building that needs the render phase: draw the selected model in the window,
//! composite the goop textures through the selected UV, and animate the
//! per-pixel compare. The front-projection-bounds UV generator feeds it (retail
//! authored its goop UV as a front projection fit to the [0,1] canvas -- our
//! measurement of the real UV1 confirmed it lands exactly on the unit square).
//!
//! # Full window layout (the target to build)
//!
//! A standalone window, BrawlBox Advanced Model Editor in spirit:
//!
//! - **Menu bar**, BrawlBox's File/Edit/View repurposed to mask concerns:
//!   - *Actor* -- sample an actor, export.
//!   - *Edit* -- undo/redo/clear the painted mask.
//!   - *View* -- the UV-layer toggle (UV0 clean vs goop UV coated), wireframe.
//!   - *Mask* -- generate goop map + example mask, play the wash cycle.
//! - **Actor sampler at the top** -- a dropdown that samples the *loaded
//!   stage's own hierarchy* (the enemy actors placed in the level), not the
//!   whole catalog, so you edit goop on your stage's actors. `[TODO: the
//!   scaffle's `mask_actor_choices` still lists the full catalog; point it at
//!   `self.document`'s placed enemy actors.]`
//! - **Central 3D viewport** -- the selected model, orbitable.
//! - **UV-layer side menu** -- selecting UV0 renders clean; selecting the goop
//!   UV renders coated (colour map masked by the goop mask).
//! - **Generate** seeds rainbow colour + the retail 32x32 StayPakkun mask;
//!   **Play full cycle** sweeps `K0_A` and evaluates `mask > K0_A` per pixel.

use super::*;

/// A choice in the Mask Tool actor dropdown.
struct MaskActorChoice {
    factory_name: String,
    class_name: String,
}

impl SmsEditorApp {
    /// Enemy actors the Mask Tool can target, alphabetical by factory name.
    fn mask_actor_choices(&self) -> Vec<MaskActorChoice> {
        let Some(registry) = self.registry.as_ref() else {
            return Vec::new();
        };
        let mut choices = registry
            .enemy_actors
            .iter()
            .map(|actor| MaskActorChoice {
                factory_name: actor.factory_name.clone(),
                class_name: actor.class_name.clone(),
            })
            .collect::<Vec<_>>();
        choices.sort_by(|left, right| left.factory_name.cmp(&right.factory_name));
        choices
    }

    /// The inspector panel for the Mask Tool.
    pub(super) fn mask_tool_panel(&mut self, ui: &mut egui::Ui) {
        ui.heading("Mask Tool");
        ui.label(
            egui::RichText::new("Paint washable goop masks onto enemy actor models")
                .small()
                .color(egui::Color32::GRAY),
        );
        ui.separator();

        let choices = self.mask_actor_choices();
        if choices.is_empty() {
            ui.label("The enemy schema is unavailable, so no actors can be loaded.");
            return;
        }

        let selected_label = self
            .mask_selected_actor
            .as_deref()
            .unwrap_or("Choose an actor")
            .to_string();
        egui::ComboBox::from_label("Actor")
            .selected_text(selected_label)
            .show_ui(ui, |ui| {
                for choice in &choices {
                    let picked = self.mask_selected_actor.as_deref() == Some(&choice.factory_name);
                    if ui
                        .selectable_label(picked, &choice.factory_name)
                        .on_hover_text(&choice.class_name)
                        .clicked()
                    {
                        self.mask_selected_actor = Some(choice.factory_name.clone());
                    }
                }
            });

        let Some(actor) = self.mask_selected_actor.clone() else {
            ui.separator();
            ui.label("Pick an actor to load its model for painting.");
            return;
        };

        ui.separator();
        self.mask_actor_summary(ui, &actor);

        ui.separator();
        ui.heading("Brush");
        ui.add(
            egui::Slider::new(&mut self.mask_brush_radius, 1.0..=64.0)
                .text("Radius")
                .clamping(egui::SliderClamping::Always),
        );
        ui.add(
            egui::Slider::new(&mut self.mask_brush_opacity, 0.0..=1.0)
                .text("Opacity")
                .clamping(egui::SliderClamping::Always),
        );

        ui.separator();
        ui.heading("Goop UV layer");
        ui.label(
            egui::RichText::new(
                "Generate the mask's UV set by front-projecting the model \u{2014} the same way \
                 retail authored its goop UV (a front projection, front and back shared).",
            )
            .small()
            .color(egui::Color32::GRAY),
        );
        if ui
            .button("Create goop UV (front projection)")
            .on_hover_text(
                "Project the model from the front into a new UV layer for the goop mask, like \
                 Blender's Project from View (Bounds).",
            )
            .clicked()
        {
            self.log.push(
                "Front-projection goop UV: pending the model-load phase, which supplies the \
                 geometry to project."
                    .to_string(),
            );
        }

        ui.separator();
        ui.heading("Wash preview");
        self.mask_wash_controls(ui);

        ui.separator();
        ui.colored_label(
            egui::Color32::from_rgb(255, 180, 90),
            "Scaffold: painting, UV projection, and material authoring are not implemented yet.",
        );
        ui.label(
            egui::RichText::new(
                "Next: load the selected model into the viewport and project brush strokes onto \
                 its mask UV. See the module docs for the phase plan.",
            )
            .small()
            .color(egui::Color32::GRAY),
        );
    }

    /// The wash-cycle preview: a "Play full cycle" button that sweeps the
    /// threshold the game compares the mask against.
    ///
    /// The washable goop is `visible = mask > K0_A`, where `K0_A` runs from
    /// full coverage down to clean as the actor is sprayed. Sweeping it here
    /// previews exactly how a painted gradient recedes -- bright paint clings,
    /// dark paint clears first -- so an author sees the wash without the game.
    /// The visual application onto the model comes with the render phase; the
    /// control and its animation are stood up now.
    fn mask_wash_controls(&mut self, ui: &mut egui::Ui) {
        const CYCLE_SECONDS: f32 = 4.0;

        if self.mask_wash_playing {
            let dt = ui.input(|input| input.stable_dt).clamp(0.0, 0.1);
            self.mask_wash_phase -= dt / CYCLE_SECONDS;
            if self.mask_wash_phase <= 0.0 {
                self.mask_wash_phase = 0.0;
                self.mask_wash_playing = false;
            } else {
                ui.ctx().request_repaint();
            }
        }

        ui.horizontal(|ui| {
            let label = if self.mask_wash_playing {
                "Stop"
            } else {
                "Play full cycle"
            };
            if ui
                .button(label)
                .on_hover_text("Sweep the wash threshold from fully coated to clean")
                .clicked()
            {
                if self.mask_wash_playing {
                    self.mask_wash_playing = false;
                } else {
                    self.mask_wash_phase = 1.0;
                    self.mask_wash_playing = true;
                }
            }
            if ui.button("Reset").clicked() {
                self.mask_wash_playing = false;
                self.mask_wash_phase = 1.0;
            }
        });

        ui.add(
            egui::Slider::new(&mut self.mask_wash_phase, 0.0..=1.0)
                .text("Coverage")
                .clamping(egui::SliderClamping::Always),
        );
        ui.label(
            egui::RichText::new(format!(
                "threshold K0_A \u{2248} {}  (mask > this stays coated)",
                (self.mask_wash_phase * 255.0).round() as u16
            ))
            .small()
            .color(egui::Color32::GRAY),
        );
    }

    /// A read-only summary of the chosen actor's paintability, from its
    /// decomp-derived catalog template and model.
    fn mask_actor_summary(&mut self, ui: &mut egui::Ui, actor_factory: &str) {
        let Some(template) = self.object_authoring_catalog.find(actor_factory) else {
            ui.label(format!(
                "No retail authoring template for '{actor_factory}' yet, so its model cannot be \
                 loaded."
            ));
            return;
        };
        ui.label(format!("Catalog source stage: {}", template.source_stage));
        ui.label(
            egui::RichText::new(
                "Whether this actor already carries the goop mask wiring (mask UV, compare TEV \
                 stage, scalar driver) is determined when the model loads -- an already-wired \
                 actor needs only textures; an un-wired one needs UV + material + a scalar patch.",
            )
            .small()
            .color(egui::Color32::GRAY),
        );
    }
}
