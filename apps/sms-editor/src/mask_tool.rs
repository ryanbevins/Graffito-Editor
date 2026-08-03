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
