use super::*;
use sms_authoring::{AssetId, ModelInstanceExportMode, ModelInstancePlacement};

#[test]
fn content_dock_window_clamp_does_not_replace_the_preferred_height() {
    assert_eq!(preferred_content_dock_height(700.0, 320.0, false), 700.0);
    assert_eq!(preferred_content_dock_height(700.0, 480.0, true), 480.0);
}

#[test]
fn animated_viewport_repaint_is_bounded_to_the_sunshine_frame_clock() {
    let ctx = egui::Context::default();
    let _ = ctx.run_ui(egui::RawInput::default(), |_| {});
    let output = ctx.run_ui(egui::RawInput::default(), |ui| {
        crate::viewport_ui::request_viewport_animation_repaint(ui.ctx());
    });
    let repaint_delay = output
        .viewport_output
        .get(&egui::ViewportId::ROOT)
        .expect("root viewport output")
        .repaint_delay;

    assert!(
        crate::viewport_ui::VIEWPORT_ANIMATION_REPAINT_INTERVAL
            >= std::time::Duration::from_millis(33)
    );
    assert!(
        repaint_delay >= std::time::Duration::from_millis(15),
        "egui reduced the animation wakeup to an immediate repaint: {repaint_delay:?}"
    );
    assert!(
        repaint_delay <= crate::viewport_ui::VIEWPORT_ANIMATION_REPAINT_INTERVAL,
        "egui scheduled later than the requested animation cadence: {repaint_delay:?}"
    );
}

#[test]
fn monte_palette_entries_use_friendly_pianta_names() {
    let object = ObjectDefinition {
        factory_name: "NPCMonteMH".to_string(),
        class_name: "TMonteMH".to_string(),
        category: "NPC".to_string(),
        source: sms_schema::SchemaSource::MarNameRefGen,
        display_name: None,
        preview_model: None,
        hidden: false,
        unsafe_to_edit: false,
    };
    assert_eq!(
        crate::ui_panels::object_palette_display_name(&object),
        "Pianta - Male (Variant H)"
    );

    let mut female = object;
    female.factory_name = "NPCMonteW".to_string();
    assert_eq!(
        crate::ui_panels::object_palette_display_name(&female),
        "Pianta - Female"
    );
}

#[test]
fn nozzle_box_palette_entry_uses_a_readable_name() {
    let object = ObjectDefinition {
        factory_name: "NozzleBox".to_string(),
        class_name: "TNozzleBox".to_string(),
        category: "MapObj".to_string(),
        source: sms_schema::SchemaSource::MarNameRefGen,
        display_name: None,
        preview_model: None,
        hidden: false,
        unsafe_to_edit: false,
    };
    assert_eq!(
        crate::ui_panels::object_palette_display_name(&object),
        "Nozzle Box"
    );
}

#[test]
fn editor_layout_defaults_to_the_unreal_style_workspace() {
    let app = SmsEditorApp::default();

    assert_eq!(app.tool, EditorTool::Move);
    assert_eq!(app.bottom_tab, BottomTab::Content);
    assert!(!app.show_project_settings);
    assert!(!app.show_issues);
    assert!(!app.show_console);
    assert!(!app.show_stats);
    assert!(!app.show_audio_helpers);
    assert!(app.show_effects);
    assert_eq!(app.level_transform_progress, FULL_DELFINO_PROGRESSION);
}

#[test]
fn viewport_toolbar_keeps_only_core_transform_tools_top_level() {
    assert_eq!(
        CORE_VIEWPORT_TOOLS,
        [EditorTool::Move, EditorTool::Rotate, EditorTool::Scale]
    );
    assert!(!CORE_VIEWPORT_TOOLS.contains(&EditorTool::Goop));
    assert!(!CORE_VIEWPORT_TOOLS.contains(&EditorTool::Place));
    // Select is a state reached by toggling a transform tool off, not a
    // button. A fourth button wrapped the row and pushed out the controls
    // below it.
    assert!(!CORE_VIEWPORT_TOOLS.contains(&EditorTool::Select));
}

#[test]
fn routes_menu_preserves_the_existing_mode_transition() {
    let mut app = SmsEditorApp {
        tool: EditorTool::Rotate,
        ..SmsEditorApp::default()
    };

    app.set_route_mode(true);
    assert!(app.route_mode);
    assert_eq!(app.tool, EditorTool::Move);

    app.set_route_mode(false);
    assert!(!app.route_mode);
    assert_eq!(app.tool, EditorTool::Move);
}

#[test]
fn content_browser_layout_wraps_to_the_available_width() {
    let narrow = content_browser_layout(360.0, 20);
    let medium = content_browser_layout(760.0, 20);
    let wide = content_browser_layout(1_240.0, 20);
    let sparse = content_browser_layout(1_240.0, 3);

    assert_eq!(narrow.columns, 1);
    assert!(medium.columns > narrow.columns);
    assert!(wide.columns > narrow.columns);
    assert_eq!(sparse.columns, 3);
    assert!((180.0..=260.0).contains(&wide.card_width));
    for (available_width, layout) in [(360.0, narrow), (760.0, medium), (1_240.0, wide)] {
        let occupied_width =
            layout.card_width * layout.columns as f32 + 8.0 * (layout.columns - 1) as f32;
        assert!(occupied_width <= available_width);
    }
}

#[test]
fn content_browser_cards_include_game_localized_stage_and_scenario_names() {
    let archive = SceneArchiveInfo {
        stage_id: "bianco0".to_string(),
        group: "bianco".to_string(),
        relative_path: PathBuf::from("files/data/scene/bianco0.szs"),
        path: PathBuf::from("C:/game/files/data/scene/bianco0.szs"),
        size_bytes: 2_300_000,
    };
    let localized = SceneArchiveLabel {
        stage_name: Some("BIANCO HILLS".to_string()),
        scenario_names: vec![
            "Road to the Big Windmill".to_string(),
            "The Hillside Cave Secret".to_string(),
        ],
    };

    let card = content_browser_card_text(&archive, Some(&localized));
    let hover = content_browser_hover_text(&archive, Some(&localized));
    assert!(card.contains("bianco0"));
    assert!(card.contains("BIANCO HILLS"));
    assert!(card.contains("Road to the Big Windmill (+1)"));
    assert_eq!(card.lines().count(), 4);
    assert!(hover.contains("The Hillside Cave Secret"));
}

#[test]
fn window_title_includes_the_project_and_open_level() {
    assert_eq!(
        editor_window_title(Some("Sunshine US"), Some("bianco3")),
        "Sunshine US - bianco3 - Graffito-Editor"
    );
    assert_eq!(
        editor_window_title(Some("Sunshine US"), None),
        "Sunshine US - Graffito-Editor"
    );
    assert_eq!(editor_window_title(None, None), "Graffito-Editor");
}

fn assert_vec3_close(actual: [f32; 3], expected: [f32; 3]) {
    for (actual, expected) in actual.into_iter().zip(expected) {
        assert!(
            (actual - expected).abs() < 0.001,
            "expected {expected}, got {actual}"
        );
    }
}

#[test]
fn coin_variants_use_the_item_managers_retail_rotation_speed() {
    for (factory, class) in [
        ("coin", "TCoin"),
        ("CoinRed", ""),
        ("CoinBlue", ""),
        ("coin_red", "TCoinRed"),
        ("coin_blue", "TCoinBlue"),
        ("FlowerCoin", "TFlowerCoin"),
        ("joint_coin", "TCoin"),
    ] {
        let mut object = SceneObject::new("coin-instance", factory);
        object.class_name = Some(class.to_string());
        assert_eq!(runtime_yaw_degrees_per_frame(&object), 2.0, "{factory}");
    }

    assert_eq!(
        runtime_yaw_degrees_per_frame(&SceneObject::new("tree-instance", "PalmTree")),
        0.0
    );
}

#[test]
fn runtime_rotation_uses_sunshines_clock_and_wraps_yaw() {
    let transform = Transform {
        rotation_degrees: [10.0, 350.0, 20.0],
        ..Transform::default()
    };
    let animated = runtime_rotated_transform(transform, 1.0, 2.0);

    assert_vec3_close(animated.rotation_degrees, [10.0, 110.0, 20.0]);
}

#[test]
fn full_billboard_local_positive_z_moves_toward_the_camera() {
    let billboard = J3dBillboard {
        mode: sms_formats::J3dBillboardMode::Full,
        center: [0.0, 0.0, 100.0],
        axes: [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]],
        offsets: [[0.0, 0.0, 10.0]; 3],
        normals: None,
    };
    let vertices = j3d_billboard_world_vertices(
        billboard,
        CameraFrame {
            position: [0.0; 3],
            right: [1.0, 0.0, 0.0],
            up: [0.0, 1.0, 0.0],
            forward: [0.0, 0.0, 1.0],
        },
    );
    assert_eq!(vertices[0], [0.0, 0.0, 90.0]);
}

fn camera_app() -> SmsEditorApp {
    let mut app = SmsEditorApp::default();
    {
        let camera = app.renderer.camera_mut();
        camera.focus = [0.0, 0.0, 1000.0];
        camera.yaw_degrees = 0.0;
        camera.pitch_degrees = 0.0;
        camera.distance = 1000.0;
    }
    app
}

#[test]
fn transform_shortcuts_do_not_exit_goop_mode() {
    for key in [egui::Key::Q, egui::Key::W, egui::Key::E, egui::Key::R] {
        assert_eq!(
            EditorTool::Goop.after_keyboard_shortcut(key),
            EditorTool::Goop
        );
    }

    assert_eq!(
        EditorTool::Move.after_keyboard_shortcut(egui::Key::E),
        EditorTool::Rotate
    );
    // G no longer switches tools: it starts a viewport grab, and Goop is
    // reached from the Tools menu.
    assert_eq!(
        EditorTool::Move.after_keyboard_shortcut(egui::Key::G),
        EditorTool::Move
    );
}

fn keyboard_input(key: egui::Key, modifiers: egui::Modifiers) -> egui::RawInput {
    egui::RawInput {
        modifiers,
        events: vec![egui::Event::Key {
            key,
            physical_key: Some(key),
            pressed: true,
            repeat: false,
            modifiers,
        }],
        ..egui::RawInput::default()
    }
}

fn clipboard_input(event: egui::Event) -> egui::RawInput {
    egui::RawInput {
        modifiers: egui::Modifiers::CTRL,
        events: vec![event],
        ..egui::RawInput::default()
    }
}

fn shortcut_test_document(object: SceneObject) -> StageDocument {
    StageDocument {
        stage_id: "fixture0".to_string(),
        base_root: PathBuf::from("."),
        assets: Vec::new(),
        objects: vec![object],
        changed_files: BTreeMap::new(),
        stage_archive: None,
        stage_archive_source_path: Some(PathBuf::from("virtual/fixture0.szs")),
        archive_edits: StageArchiveEdits::default(),
        registry: None,
        route_authoring: None,
        goop_authoring: None,
        dialogue_authoring: None,
        dialogue_library: Default::default(),
        load_issues: Vec::new(),
        lighting: StageLighting::default(),
        death_barrier: None,
        actor_previews: BTreeMap::new(),
        loaded_project: None,
    }
}

#[test]
fn viewport_focus_allows_world_edit_shortcuts_while_text_edit_focus_owns_them() {
    let context = egui::Context::default();
    let _ = context.run_ui(egui::RawInput::default(), |ui| {
        let viewport = ui.allocate_response(egui::vec2(100.0, 100.0), egui::Sense::click());
        viewport.request_focus();
    });
    assert!(context.egui_wants_keyboard_input());
    assert!(!text_editor_owns_shortcuts(&context));

    let mut object = SceneObject::new("source-object", "FixtureEnemy");
    object.transform.translation = [10.0, 20.0, 30.0];
    let mut app = SmsEditorApp {
        selected_object_id: Some(object.id.clone()),
        document: Some(shortcut_test_document(object.clone())),
        ..SmsEditorApp::default()
    };
    let _ = context.run_ui(
        keyboard_input(egui::Key::Delete, egui::Modifiers::NONE),
        |ui| app.handle_editor_shortcuts(ui.ctx()),
    );
    assert!(app.document.as_ref().unwrap().objects.is_empty());

    let _ = context.run_ui(keyboard_input(egui::Key::Z, egui::Modifiers::CTRL), |ui| {
        app.handle_editor_shortcuts(ui.ctx())
    });
    assert_eq!(
        app.document.as_ref().unwrap().objects,
        std::slice::from_ref(&object)
    );
    let _ = context.run_ui(keyboard_input(egui::Key::Y, egui::Modifiers::CTRL), |ui| {
        app.handle_editor_shortcuts(ui.ctx())
    });
    assert!(app.document.as_ref().unwrap().objects.is_empty());
    let _ = context.run_ui(keyboard_input(egui::Key::Z, egui::Modifiers::CTRL), |ui| {
        app.handle_editor_shortcuts(ui.ctx())
    });
    assert_eq!(
        app.document.as_ref().unwrap().objects,
        std::slice::from_ref(&object)
    );

    app.selected_object_id = Some(object.id.clone());
    let copy_output = context.run_ui(clipboard_input(egui::Event::Copy), |ui| {
        app.handle_editor_shortcuts(ui.ctx());
    });
    assert!(copy_output.platform_output.commands.iter().any(|command| {
        matches!(
            command,
            egui::OutputCommand::CopyText(text) if text == "Graffito Editor object"
        )
    }));
    let _ = context.run_ui(
        clipboard_input(egui::Event::Paste("Graffito Editor object".to_string())),
        |ui| app.handle_editor_shortcuts(ui.ctx()),
    );
    let objects = &app.document.as_ref().unwrap().objects;
    assert_eq!(objects.len(), 2);
    assert_eq!(
        objects[1].transform.translation,
        object.transform.translation
    );

    let _ = context.run_ui(keyboard_input(egui::Key::D, egui::Modifiers::CTRL), |ui| {
        app.handle_editor_shortcuts(ui.ctx())
    });
    let objects = &app.document.as_ref().unwrap().objects;
    assert_eq!(objects.len(), 3);
    assert_eq!(
        objects[2].transform.translation,
        object.transform.translation
    );

    let text_context = egui::Context::default();
    let mut text = String::new();
    let _ = text_context.run_ui(egui::RawInput::default(), |ui| {
        ui.text_edit_singleline(&mut text).request_focus();
    });
    assert!(text_editor_owns_shortcuts(&text_context));
}

#[test]
fn modal_grab_suppresses_global_edit_shortcuts() {
    let context = egui::Context::default();
    let object = SceneObject::new("grabbed-object", "FixtureEnemy");
    let mut app = SmsEditorApp {
        selected_object_id: Some(object.id.clone()),
        document: Some(shortcut_test_document(object.clone())),
        grab_drag: Some(GrabDrag {
            target: GrabTarget::Object,
            start_transform: object.transform,
            start_pointer: egui::Pos2::ZERO,
            axis: None,
            world_units_per_pixel: 1.0,
        }),
        ..SmsEditorApp::default()
    };

    let _ = context.run_ui(
        keyboard_input(egui::Key::Delete, egui::Modifiers::NONE),
        |ui| app.handle_editor_shortcuts(ui.ctx()),
    );

    assert_eq!(
        app.document.as_ref().unwrap().objects,
        std::slice::from_ref(&object)
    );
    assert!(app.undo_stack.is_empty());
}

#[test]
fn automatic_scene_refresh_is_queued_once_per_base_root() {
    let mut app = SmsEditorApp {
        base_root: ".".to_string(),
        ..SmsEditorApp::default()
    };
    let (_sender, receiver) = mpsc::channel();
    app.background_receiver = Some(receiver);

    app.refresh_scene_browser_if_needed();
    assert_eq!(app.pending_auto_refresh_root.as_deref(), Some("."));
    assert!(app.last_auto_refresh_attempt_root.is_empty());

    app.pending_auto_refresh_root = None;
    app.last_auto_refresh_attempt_root = ".".to_string();
    app.refresh_scene_browser_if_needed();
    assert!(app.pending_auto_refresh_root.is_none());
}

#[test]
fn fly_camera_velocity_interpolates_in_and_out() {
    let accelerated =
        viewport_ui::interpolate_camera_velocity([0.0; 3], [1000.0, 0.0, 0.0], 1.0 / 60.0, 8.0);
    assert!(accelerated[0] > 0.0);
    assert!(accelerated[0] < 1000.0);

    let accelerated_again =
        viewport_ui::interpolate_camera_velocity(accelerated, [1000.0, 0.0, 0.0], 1.0 / 60.0, 8.0);
    assert!(accelerated_again[0] > accelerated[0]);
    assert!(accelerated_again[0] < 1000.0);

    let decelerated =
        viewport_ui::interpolate_camera_velocity(accelerated_again, [0.0; 3], 1.0 / 60.0, 12.0);
    assert!(decelerated[0] > 0.0);
    assert!(decelerated[0] < accelerated_again[0]);
}

#[test]
fn project_camera_state_restores_the_last_stage_view() {
    let mut project = SmsProjectFile::new(
        "Camera Test",
        PathBuf::from(r"C:\Games\SunshineJPExtract"),
        PathBuf::from("Camera Test.smsdata"),
        None,
    );
    project.stage_cameras.insert(
        "bianco2".to_string(),
        ProjectCameraState {
            focus: [120.0, 340.0, 560.0],
            distance: 7_500.0,
            yaw_degrees: 135.0,
            pitch_degrees: -22.0,
            viewport_pan: [14.0, -9.0],
            viewport_zoom: 1.4,
            camera_speed: 0.5,
            navigation_distance: 6_200.0,
        },
    );
    let mut app = SmsEditorApp {
        current_project: Some(OpenProject {
            descriptor_path: PathBuf::from("Camera Test.sms"),
            descriptor: project,
        }),
        stage_id: "bianco2".to_string(),
        ..SmsEditorApp::default()
    };

    assert!(app.restore_project_camera_state());
    assert_vec3_close(app.renderer.camera().focus, [120.0, 340.0, 560.0]);
    assert_eq!(app.renderer.camera().distance, 7_500.0);
    assert_eq!(app.renderer.camera().yaw_degrees, 135.0);
    assert_eq!(app.renderer.camera().pitch_degrees, -22.0);
    assert_eq!(app.viewport_pan, egui::vec2(14.0, -9.0));
    assert_eq!(app.viewport_zoom, 1.4);
    assert_eq!(app.camera_speed, 0.5);
}

#[test]
fn fly_camera_scroll_adjusts_and_clamps_speed() {
    assert!(viewport_ui::camera_speed_after_scroll(1.0, 120.0) > 1.0);
    assert!(viewport_ui::camera_speed_after_scroll(1.0, -120.0) < 1.0);
    assert_eq!(viewport_ui::camera_speed_after_scroll(8.0, 10_000.0), 8.0);
    assert_eq!(
        viewport_ui::camera_speed_after_scroll(0.01, -10_000.0),
        0.01
    );
}

#[test]
fn captured_viewport_navigation_uses_unbounded_raw_mouse_motion() {
    let position_delta = egui::vec2(0.0, 0.0);
    let raw_motion = egui::vec2(18.0, -7.0);

    assert_eq!(
        viewport_ui::captured_viewport_pointer_delta(true, Some(raw_motion), position_delta),
        raw_motion
    );
    assert_eq!(
        viewport_ui::captured_viewport_pointer_delta(false, Some(raw_motion), position_delta),
        position_delta
    );
}

#[test]
fn viewport_mouse_capture_releases_with_button_or_window_focus() {
    assert!(!viewport_ui::viewport_mouse_capture_should_release(
        true, true, true
    ));
    assert!(viewport_ui::viewport_mouse_capture_should_release(
        true, false, true
    ));
    assert!(viewport_ui::viewport_mouse_capture_should_release(
        true, true, false
    ));
}

#[test]
fn viewport_markers_show_only_selection_outside_objects_mode() {
    let app_objects = vec![
        SceneObject::new("obj-a", "Coin"),
        SceneObject::new("obj-b", "Shine"),
    ];
    let mut app = SmsEditorApp {
        document: Some(test_document(app_objects)),
        selected_object_id: Some("obj-b".to_string()),
        view_mode: ViewMode::Lit,
        ..SmsEditorApp::default()
    };

    let marker_ids = |app: &SmsEditorApp| {
        app.viewport_marker_objects()
            .map(|object| object.id.clone())
            .collect::<Vec<_>>()
    };
    assert_eq!(marker_ids(&app), ["obj-b".to_string()]);

    app.view_mode = ViewMode::Collision;
    assert_eq!(marker_ids(&app), ["obj-b".to_string()]);

    app.view_mode = ViewMode::Objects;
    assert_eq!(marker_ids(&app), ["obj-a".to_string(), "obj-b".to_string()]);

    app.view_mode = ViewMode::Lit;
    app.selected_object_id = None;
    assert!(marker_ids(&app).is_empty());
}

#[test]
fn collision_preview_expands_col_groups_into_surface_typed_triangles() {
    let collision = ColFile::new(
        vec![
            sms_formats::ColVertex::new(0.0, 0.0, 0.0),
            sms_formats::ColVertex::new(100.0, 0.0, 0.0),
            sms_formats::ColVertex::new(0.0, 0.0, 100.0),
            sms_formats::ColVertex::new(100.0, 0.0, 100.0),
        ],
        vec![sms_formats::ColGroup {
            surface_type: 0x0102,
            has_per_triangle_data: false,
            triangles: vec![
                sms_formats::ColTriangle {
                    vertex_indices: [0, 1, 2],
                    attribute_0: 0,
                    attribute_1: 0,
                    data: None,
                },
                sms_formats::ColTriangle {
                    vertex_indices: [1, 3, 2],
                    attribute_0: 0,
                    attribute_1: 0,
                    data: None,
                },
            ],
        }],
    );
    let mut preview = CollisionPreviewBuild::default();

    preview.append_file(&collision);

    assert_eq!(preview.file_count, 1);
    assert_eq!(preview.triangles.len(), 2);
    assert_eq!(preview.triangles[0].surface_type, 0x0102);
    assert_eq!(
        preview.triangles[1].vertices,
        [[100.0, 0.0, 0.0], [100.0, 0.0, 100.0], [0.0, 0.0, 100.0]]
    );
    assert_eq!(preview.surface_types, BTreeSet::from([0x0102]));
}

fn single_triangle_collision(surface_type: u16) -> ColFile {
    ColFile::new(
        vec![
            sms_formats::ColVertex::new(0.0, 0.0, 0.0),
            sms_formats::ColVertex::new(10.0, 0.0, 0.0),
            sms_formats::ColVertex::new(0.0, 0.0, 10.0),
        ],
        vec![sms_formats::ColGroup {
            surface_type,
            has_per_triangle_data: false,
            triangles: vec![sms_formats::ColTriangle {
                vertex_indices: [0, 1, 2],
                attribute_0: 0,
                attribute_1: 0,
                data: None,
            }],
        }],
    )
}

#[test]
fn collision_preview_excludes_unplaced_stock_collision_resources() {
    let mut document = test_document(Vec::new());
    document.registry = Some(ObjectRegistry::default());
    let resources = BTreeMap::from([
        ("map/map.col".to_string(), single_triangle_collision(0)),
        (
            "mapobj/biabridge.col".to_string(),
            single_triangle_collision(0x0003),
        ),
        (
            "mapobj/bigwindmill.col".to_string(),
            single_triangle_collision(0x0005),
        ),
    ]);
    let mut preview = CollisionPreviewBuild::default();

    append_runtime_collision_preview(&document, &resources, &mut preview);

    assert_eq!(preview.file_count, 1);
    assert_eq!(preview.triangles.len(), 1);
    assert_eq!(preview.surface_types, BTreeSet::from([0]));
}

#[test]
fn collision_preview_places_only_schema_bound_object_collision() {
    let mut nail = SceneObject::new("nail", "MapObjNail");
    nail.set_raw_param("actor_tail_string", "MapObjNail");
    nail.transform = Transform {
        translation: [100.0, 200.0, 300.0],
        rotation_degrees: [0.0, 0.0, 0.0],
        scale: [2.0, 3.0, 4.0],
    };
    let mut air_wall = SceneObject::new("air-wall", "MapStaticObj");
    air_wall.set_raw_param("actor_tail_string", "BiancoAirWall");
    air_wall.transform.translation = [-50.0, 25.0, 75.0];
    let mut document = test_document(vec![nail, air_wall]);
    document.registry = Some(ObjectRegistry {
        map_obj_resources: vec![sms_schema::MapObjResourceDefinition {
            resource_name: "MapObjNail".to_string(),
            actor_type: 0,
            object_flags: 0,
            required_manager_name: "fixture manager".to_string(),
            has_hold_dependency: false,
            has_move_dependency: false,
            uses_resource_name_model_fallback: false,
            primary_model: Some("kugi.bmd".to_string()),
            animation_resources: Vec::new(),
            hold_model_path: None,
            move_bck_path: None,
            load_flags: 0,
            collision_resources: vec![sms_schema::MapObjCollisionResourceDefinition {
                resource_name: "kugi".to_string(),
                flags: 2,
                collision_kind: 2,
                max_vertices: None,
            }],
            source_file: "fixture.cpp".to_string(),
        }],
        map_obj_factories: vec!["MapObjNail".to_string()],
        map_static_models: vec![sms_schema::MapStaticModelDefinition {
            actor_name: "BiancoAirWall".to_string(),
            model_path: None,
            collision_path: Some("/scene/mapObj/BiaAirWall.col".to_string()),
            load_flags: 0,
            sound_id: None,
            source_file: "fixture.cpp".to_string(),
            stage_bootstrap_created: false,
        }],
        ..ObjectRegistry::default()
    });
    let resources = BTreeMap::from([
        (
            "mapobj/kugi.col".to_string(),
            single_triangle_collision(0x0005),
        ),
        (
            "mapobj/biaairwall.col".to_string(),
            single_triangle_collision(0x010A),
        ),
        (
            "mapobj/biabridge.col".to_string(),
            single_triangle_collision(0x0003),
        ),
    ]);
    let mut preview = CollisionPreviewBuild::default();

    append_runtime_collision_preview(&document, &resources, &mut preview);

    assert_eq!(preview.file_count, 2);
    assert_eq!(preview.triangles.len(), 2);
    assert_eq!(preview.surface_types, BTreeSet::from([0x0005, 0x010A]));
    let nail_triangle = preview
        .triangles
        .iter()
        .find(|triangle| triangle.surface_type == 0x0005)
        .unwrap();
    assert_eq!(
        nail_triangle.vertices,
        [
            [100.0, 200.0, 300.0],
            [120.0, 200.0, 300.0],
            [100.0, 200.0, 340.0],
        ]
    );
    let air_wall_triangle = preview
        .triangles
        .iter()
        .find(|triangle| triangle.surface_type == 0x010A)
        .unwrap();
    assert_eq!(air_wall_triangle.vertices[0], [-50.0, 25.0, 75.0]);
}

#[test]
fn collision_surface_colors_are_stable_and_distinguish_types() {
    assert_eq!(
        viewport_ui::collision_surface_color(0x0102),
        viewport_ui::collision_surface_color(0x0102)
    );
    assert_ne!(
        viewport_ui::collision_surface_color(0x0102),
        viewport_ui::collision_surface_color(0x0103)
    );
}

#[test]
fn collision_framebuffer_draws_loaded_geometry() {
    let mut preview = preview_for_texture_alpha(false, false);
    preview.collision_triangles = vec![CollisionPreviewTriangle {
        vertices: [
            [-1_000.0, 0.0, -1_000.0],
            [1_000.0, 0.0, -1_000.0],
            [0.0, 0.0, 1_000.0],
        ],
        surface_type: 1,
    }];
    preview.collision_file_count = 1;
    preview.collision_surface_count = 1;
    let app = SmsEditorApp {
        model_preview: Some(preview),
        view_mode: ViewMode::Collision,
        ..SmsEditorApp::default()
    };
    let rect = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(640.0, 480.0));

    let image = app.render_collision_framebuffer(rect).unwrap();
    let background = viewport_framebuffer_background(image.size);

    assert!(image
        .pixels
        .iter()
        .zip(background.pixels)
        .any(|(rendered, clear)| *rendered != clear));
}

#[test]
fn viewport_mesh_picking_selects_the_object_away_from_its_origin_marker() {
    let mut object = SceneObject::new("obj-mesh", "Coin");
    object.transform.translation = [600.0, 0.0, 1000.0];
    let mut preview = preview_for_texture_alpha(false, false);
    preview.object_model_indices.insert(object.id.clone(), 7);
    let mut triangle = textured_blended_triangle();
    triangle.vertices = [
        [-200.0, -200.0, 1000.0],
        [200.0, -200.0, 1000.0],
        [0.0, 200.0, 1000.0],
    ];
    triangle.model_index = 7;
    triangle.texture_index = None;
    triangle.tex_coords = None;
    preview.triangles.push(triangle);

    let app = SmsEditorApp {
        document: Some(test_document(vec![object])),
        model_preview: Some(preview),
        ..camera_app()
    };
    let rect = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(200.0, 200.0));

    assert_eq!(
        app.object_mesh_at_screen_position(rect, rect.center())
            .as_deref(),
        Some("obj-mesh")
    );
}

#[test]
fn viewport_mesh_picking_prefers_the_nearest_overlapping_object() {
    let mut preview = preview_for_texture_alpha(false, false);
    preview
        .object_model_indices
        .insert("far-object".to_string(), 1);
    preview
        .object_model_indices
        .insert("near-object".to_string(), 2);
    for (model_index, depth, extent) in [(1, 1000.0, 200.0), (2, 500.0, 100.0)] {
        let mut triangle = textured_blended_triangle();
        triangle.vertices = [
            [-extent, -extent, depth],
            [extent, -extent, depth],
            [0.0, extent, depth],
        ];
        triangle.model_index = model_index;
        triangle.texture_index = None;
        triangle.tex_coords = None;
        preview.triangles.push(triangle);
    }

    let app = SmsEditorApp {
        model_preview: Some(preview),
        ..camera_app()
    };
    let rect = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(200.0, 200.0));

    assert_eq!(
        app.object_mesh_at_screen_position(rect, rect.center())
            .as_deref(),
        Some("near-object")
    );
}

#[test]
fn viewport_picking_does_not_let_a_hidden_origin_behind_the_mesh_win() {
    let mut front = SceneObject::new("front-object", "Coin");
    front.transform.translation = [600.0, 0.0, 500.0];
    let mut behind = SceneObject::new("behind-object", "Coin");
    behind.transform.translation = [0.0, 0.0, 1000.0];
    let mut preview = preview_for_texture_alpha(false, false);
    preview.object_model_indices.insert(front.id.clone(), 1);
    let mut triangle = textured_blended_triangle();
    triangle.vertices = [
        [-100.0, -100.0, 500.0],
        [100.0, -100.0, 500.0],
        [0.0, 100.0, 500.0],
    ];
    triangle.model_index = 1;
    triangle.texture_index = None;
    triangle.tex_coords = None;
    preview.triangles.push(triangle);

    let app = SmsEditorApp {
        document: Some(test_document(vec![front, behind])),
        model_preview: Some(preview),
        ..camera_app()
    };
    let rect = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(200.0, 200.0));

    assert_eq!(
        app.object_at_screen_position(rect, rect.center())
            .as_deref(),
        Some("front-object")
    );
}

#[test]
fn viewport_picking_rejects_an_object_hidden_behind_stage_geometry() {
    let mut object = SceneObject::new("hidden-object", "Coin");
    object.transform.translation = [0.0, 0.0, 1000.0];
    let mut preview = preview_for_texture_alpha(false, false);
    preview.object_model_indices.insert(object.id.clone(), 2);
    for (model_index, depth, extent) in [(1, 500.0, 150.0), (2, 1000.0, 200.0)] {
        let mut triangle = textured_blended_triangle();
        triangle.vertices = [
            [-extent, -extent, depth],
            [extent, -extent, depth],
            [0.0, extent, depth],
        ];
        triangle.model_index = model_index;
        triangle.texture_index = None;
        triangle.tex_coords = None;
        preview.triangles.push(triangle);
    }

    let app = SmsEditorApp {
        document: Some(test_document(vec![object])),
        model_preview: Some(preview),
        ..camera_app()
    };
    let rect = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(200.0, 200.0));

    assert_eq!(app.object_at_screen_position(rect, rect.center()), None);
}

#[test]
fn viewport_picking_keeps_an_object_in_front_of_stage_geometry_selectable() {
    let mut object = SceneObject::new("visible-object", "Coin");
    object.transform.translation = [0.0, 0.0, 500.0];
    let mut preview = preview_for_texture_alpha(false, false);
    preview.object_model_indices.insert(object.id.clone(), 2);
    for (model_index, depth, extent) in [(1, 1000.0, 200.0), (2, 500.0, 150.0)] {
        let mut triangle = textured_blended_triangle();
        triangle.vertices = [
            [-extent, -extent, depth],
            [extent, -extent, depth],
            [0.0, extent, depth],
        ];
        triangle.model_index = model_index;
        triangle.texture_index = None;
        triangle.tex_coords = None;
        preview.triangles.push(triangle);
    }

    let app = SmsEditorApp {
        document: Some(test_document(vec![object])),
        model_preview: Some(preview),
        ..camera_app()
    };
    let rect = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(200.0, 200.0));

    assert_eq!(
        app.object_at_screen_position(rect, rect.center())
            .as_deref(),
        Some("visible-object")
    );
}

#[test]
fn world_selection_prefers_an_actor_already_proven_in_front_of_authored_terrain() {
    let terrain_id = uuid::Uuid::new_v4();

    assert_eq!(
        crate::viewport_ui::resolve_world_selection_hits(
            Some("front-actor".to_string()),
            Some(terrain_id),
            Some(terrain_id),
        ),
        (None, Some("front-actor".to_string()))
    );
    assert_eq!(
        crate::viewport_ui::resolve_world_selection_hits(None, Some(terrain_id), None),
        (Some(terrain_id), None)
    );
}

#[test]
fn viewport_focus_prefers_an_actor_in_front_of_authored_terrain() {
    let mut object = SceneObject::new("front-actor", "Coin");
    object.transform.translation = [0.0, 0.0, 500.0];
    let mut preview = preview_for_texture_alpha(false, false);
    preview.object_model_indices.insert(object.id.clone(), 2);
    let mut placement = ModelInstancePlacement::new(AssetId::new(), "terrain");
    placement.export_mode = ModelInstanceExportMode::MapTerrain;
    let terrain_id = placement.instance_id;
    preview.instance_model_indices.insert(terrain_id, 1);
    for (model_index, depth, extent) in [(1, 1000.0, 200.0), (2, 500.0, 150.0)] {
        let mut triangle = textured_blended_triangle();
        triangle.vertices = [
            [-extent, -extent, depth],
            [extent, -extent, depth],
            [0.0, extent, depth],
        ];
        triangle.model_index = model_index;
        triangle.texture_index = None;
        triangle.tex_coords = None;
        preview.triangles.push(triangle);
    }
    let mut app = SmsEditorApp {
        document: Some(test_document(vec![object])),
        model_preview: Some(preview),
        model_instances: vec![EditorModelInstance {
            stage_id: String::new(),
            placement,
            local_bounds: [[-200.0, -200.0, 1000.0], [200.0, 200.0, 1000.0]],
        }],
        ..camera_app()
    };
    let rect = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(200.0, 200.0));

    assert!(app.focus_camera_on_viewport_position(rect, rect.center()));

    let focus = app.camera_focus_animation.expect("focus animation");
    assert_eq!(focus.target_focus, [0.0, 0.0, 500.0]);
}

#[test]
fn viewport_picking_ignores_translucent_stage_geometry() {
    let mut object = SceneObject::new("object-under-water", "Coin");
    object.transform.translation = [0.0, 0.0, 1000.0];
    let mut preview = preview_for_texture_alpha(false, false);
    preview.object_model_indices.insert(object.id.clone(), 2);
    for (model_index, depth, extent, render_layer) in [
        (1, 500.0, 150.0, PreviewRenderLayer::Water),
        (2, 1000.0, 200.0, PreviewRenderLayer::Main),
    ] {
        let mut triangle = textured_blended_triangle();
        triangle.vertices = [
            [-extent, -extent, depth],
            [extent, -extent, depth],
            [0.0, extent, depth],
        ];
        triangle.model_index = model_index;
        triangle.render_layer = render_layer;
        triangle.texture_index = None;
        triangle.tex_coords = None;
        preview.triangles.push(triangle);
    }

    let app = SmsEditorApp {
        document: Some(test_document(vec![object])),
        model_preview: Some(preview),
        ..camera_app()
    };
    let rect = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(200.0, 200.0));

    assert_eq!(
        app.object_at_screen_position(rect, rect.center())
            .as_deref(),
        Some("object-under-water")
    );
}

#[test]
fn viewport_placement_hits_the_nearest_scene_or_object_geometry() {
    let mut preview = preview_for_texture_alpha(false, false);
    preview
        .object_model_indices
        .insert("placed-object".to_string(), 2);
    for (model_index, depth, extent) in [(1, 1_000.0, 200.0), (2, 500.0, 100.0)] {
        let mut triangle = textured_blended_triangle();
        triangle.vertices = [
            [-extent, -extent, depth],
            [extent, -extent, depth],
            [0.0, extent, depth],
        ];
        triangle.model_index = model_index;
        triangle.texture_index = None;
        triangle.tex_coords = None;
        preview.triangles.push(triangle);
    }
    let app = SmsEditorApp {
        model_preview: Some(preview),
        ..camera_app()
    };
    let rect = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(200.0, 200.0));

    assert_vec3_close(
        app.viewport_placement_position(rect, rect.center())
            .expect("center ray should hit the nearer object geometry"),
        [0.0, 0.0, 500.0],
    );
}

#[test]
fn viewport_placement_falls_back_to_the_focus_plane_over_void() {
    let mut preview = preview_for_texture_alpha(false, false);
    let mut triangle = textured_blended_triangle();
    triangle.vertices = [
        [400.0, -100.0, 500.0],
        [600.0, -100.0, 500.0],
        [500.0, 100.0, 500.0],
    ];
    triangle.texture_index = None;
    triangle.tex_coords = None;
    preview.triangles.push(triangle);
    let app = SmsEditorApp {
        model_preview: Some(preview),
        ..camera_app()
    };
    let rect = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(200.0, 200.0));

    assert_vec3_close(
        app.viewport_placement_position(rect, rect.center())
            .expect("placement over void should remain available"),
        [0.0, 0.0, 1_000.0],
    );
}

#[test]
fn viewport_placement_keeps_the_empty_stage_bootstrap_path() {
    let app = camera_app();
    let rect = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(200.0, 200.0));

    assert_vec3_close(
        app.viewport_placement_position(rect, rect.center())
            .expect("an empty stage still needs its first model placement"),
        [0.0, 0.0, 1_000.0],
    );
}

#[test]
fn viewport_drag_preview_follows_the_geometry_placement_without_hitting_itself() {
    let mut preview = preview_for_texture_alpha(false, false);
    for (model_index, depth, extent) in [(1, 1_000.0, 200.0), (2, 500.0, 100.0)] {
        let mut triangle = textured_blended_triangle();
        triangle.vertices = [
            [-extent, -extent, depth],
            [extent, -extent, depth],
            [0.0, extent, depth],
        ];
        triangle.model_index = model_index;
        triangle.texture_index = None;
        triangle.tex_coords = None;
        preview.triangles.push(triangle);
    }
    let geometry =
        viewport_drag_preview_geometry(&preview, 2, [0.0; 3]).expect("drag preview geometry");
    let app = SmsEditorApp {
        viewport_drag_preview: Some(ViewportDragPreview {
            key: ViewportDragPreviewKey::Object("Coin".to_string()),
            geometry,
            position: [0.0, 0.0, 500.0],
            had_stage_preview: true,
            triangle_range: 1..2,
            texture_start: preview.textures.len(),
            material_start: preview.materials.len(),
            material_binding_start: preview.material_animation_bindings.len(),
            model_index: 2,
        }),
        model_preview: Some(preview),
        ..camera_app()
    };
    let rect = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(200.0, 200.0));

    assert_vec3_close(
        app.viewport_placement_position(rect, rect.center())
            .expect("the stage surface behind the preview should remain hittable"),
        [0.0, 0.0, 1_000.0],
    );
}

#[test]
fn viewport_drag_preview_triangle_moves_and_remaps_shared_render_resources() {
    let mut triangle = textured_blended_triangle();
    triangle.vertices = [[10.0, 20.0, 30.0], [20.0, 20.0, 30.0], [10.0, 30.0, 30.0]];
    triangle.material_index = Some(2);
    triangle.texture_index = Some(3);
    triangle.mask_texture_index = Some(4);
    triangle.packet_index = 5;
    triangle.model_index = 6;

    let positioned = viewport_ui::positioned_viewport_drag_triangle(
        triangle,
        [10.0, 20.0, 30.0],
        [110.0, 220.0, 330.0],
        7,
        11,
        13,
        17,
    );

    assert_eq!(
        positioned.vertices,
        [
            [110.0, 220.0, 330.0],
            [120.0, 220.0, 330.0],
            [110.0, 230.0, 330.0],
        ]
    );
    assert_eq!(positioned.material_index, Some(9));
    assert_eq!(positioned.texture_index, Some(14));
    assert_eq!(positioned.mask_texture_index, Some(15));
    assert_eq!(positioned.packet_index, 18);
    assert_eq!(positioned.model_index, 17);
}

#[test]
fn selected_object_outline_keeps_the_silhouette_and_removes_internal_edges() {
    let mut preview = preview_for_texture_alpha(false, false);
    preview
        .object_model_indices
        .insert("selected-object".to_string(), 9);
    for vertices in [
        [
            [-100.0, -100.0, 1000.0],
            [100.0, -100.0, 1000.0],
            [100.0, 100.0, 1000.0],
        ],
        [
            [-100.0, -100.0, 1000.0],
            [100.0, 100.0, 1000.0],
            [-100.0, 100.0, 1000.0],
        ],
    ] {
        let mut triangle = textured_blended_triangle();
        triangle.vertices = vertices;
        triangle.model_index = 9;
        triangle.texture_index = None;
        triangle.tex_coords = None;
        preview.triangles.push(triangle);
    }

    let app = SmsEditorApp {
        model_preview: Some(preview),
        selected_object_id: Some("selected-object".to_string()),
        ..camera_app()
    };
    let rect = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(200.0, 200.0));

    let segments = app.selected_object_outline_segments(rect);
    let paths = viewport_ui::outline_paths_from_segments(&segments);
    assert_eq!(paths.len(), 1);
    assert_eq!(paths[0].first(), paths[0].last());
}

#[test]
fn selected_object_outline_excludes_object_shadow_geometry() {
    let mut preview = preview_for_texture_alpha(false, false);
    preview
        .object_model_indices
        .insert("selected-object".to_string(), 9);
    for vertices in [
        [
            [-100.0, -100.0, 1000.0],
            [100.0, -100.0, 1000.0],
            [100.0, 100.0, 1000.0],
        ],
        [
            [-100.0, -100.0, 1000.0],
            [100.0, 100.0, 1000.0],
            [-100.0, 100.0, 1000.0],
        ],
    ] {
        let mut triangle = textured_blended_triangle();
        triangle.vertices = vertices;
        triangle.model_index = 9;
        triangle.texture_index = None;
        triangle.tex_coords = None;
        preview.triangles.push(triangle);
    }

    let mut app = SmsEditorApp {
        model_preview: Some(preview),
        selected_object_id: Some("selected-object".to_string()),
        ..camera_app()
    };
    let rect = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(200.0, 200.0));
    let object_only_segments = app.selected_object_outline_segments(rect);
    assert!(!object_only_segments.is_empty());

    let mut shadow = textured_blended_triangle();
    shadow.vertices = [
        [200.0, -100.0, 1000.0],
        [300.0, -100.0, 1000.0],
        [250.0, 0.0, 1000.0],
    ];
    shadow.model_index = 9;
    shadow.render_layer = PreviewRenderLayer::Shadow;
    shadow.texture_index = None;
    shadow.tex_coords = None;
    app.model_preview.as_mut().unwrap().triangles.push(shadow);

    assert_eq!(
        app.selected_object_outline_segments(rect),
        object_only_segments
    );
}

#[test]
fn selected_object_outline_merges_overlapping_polygon_coverage() {
    let size = [8, 6];
    let mut coverage = vec![false; size[0] * size[1]];
    for y in 1..=4 {
        for x in 1..=4 {
            coverage[y * size[0] + x] = true;
        }
    }
    for y in 2..=3 {
        for x in 3..=6 {
            coverage[y * size[0] + x] = true;
        }
    }
    let rect = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(8.0, 6.0));
    let segments = viewport_ui::outline_segments_from_coverage(&coverage, size, [1, 6, 1, 4], rect);

    assert!(!segments.iter().any(|segment| {
        segment[0].x == 5.0 && segment[1].x == 5.0 && segment[0].y <= 3.0 && segment[1].y >= 3.0
    }));
}

#[test]
fn bounded_outline_coverage_matches_full_frame_coverage() {
    let size = [9, 7];
    let bounds = [3, 6, 2, 4];
    let rect = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(180.0, 140.0));
    let mut full = vec![false; size[0] * size[1]];
    for y in bounds[2]..=bounds[3] {
        for x in bounds[0]..=bounds[1] {
            full[y * size[0] + x] = !(x == 4 && y == 3);
        }
    }
    let bounded_size = [bounds[1] - bounds[0] + 1, bounds[3] - bounds[2] + 1];
    let mut bounded = vec![false; bounded_size[0] * bounded_size[1]];
    for y in bounds[2]..=bounds[3] {
        for x in bounds[0]..=bounds[1] {
            bounded[(y - bounds[2]) * bounded_size[0] + x - bounds[0]] = full[y * size[0] + x];
        }
    }

    assert_eq!(
        viewport_ui::outline_segments_from_coverage(&full, size, bounds, rect),
        viewport_ui::outline_segments_from_bounded_coverage(
            &bounded,
            bounded_size,
            [bounds[0], bounds[2]],
            size,
            bounds,
            rect,
        )
    );
}

#[test]
fn nozzle_box_tev_color_matches_runtime_item_type() {
    let registry = ObjectRegistry {
        objects: vec![sms_schema::ObjectDefinition {
            factory_name: "NozzleBox".to_string(),
            class_name: "TNozzleBox".to_string(),
            category: "MapObj".to_string(),
            source: sms_schema::SchemaSource::MarNameRefGen,
            display_name: None,
            preview_model: None,
            hidden: false,
            unsafe_to_edit: false,
        }],
        map_obj_string_tev_programs: vec![sms_schema::MapObjStringTevProgramDefinition {
            resource_name: "NozzleBox".to_string(),
            class_name: "TNozzleBox".to_string(),
            tev_register: 1,
            default_color: [255, 255, 255, 100],
            variants: vec![
                sms_schema::MapObjStringTevVariantDefinition {
                    selector_value: "normal_nozzle_item".to_string(),
                    color: [0, 0, 255, 100],
                },
                sms_schema::MapObjStringTevVariantDefinition {
                    selector_value: "rocket_nozzle_item".to_string(),
                    color: [255, 0, 0, 100],
                },
                sms_schema::MapObjStringTevVariantDefinition {
                    selector_value: "back_nozzle_item".to_string(),
                    color: [90, 90, 120, 100],
                },
            ],
            source_file: "src/MoveBG/Item.cpp".to_string(),
        }],
        ..ObjectRegistry::default()
    };
    let mut rocket = SceneObject::new("rocket-box", "NozzleBox");
    rocket.set_raw_param("actor_tail_string", "NozzleBox");
    rocket.set_raw_param("nozzle_box_item", "rocket_nozzle_item");
    let mut hover = SceneObject::new("hover-box", "NozzleBox");
    hover.set_raw_param("actor_tail_string", "NozzleBox");
    hover.set_raw_param("nozzle_box_item", "back_nozzle_item");
    let mut legacy = SceneObject::new("legacy-box", "NozzleBox");
    legacy.set_raw_param("actor_tail_string", "NozzleBox");
    legacy.set_raw_param("stream_string_1", "normal_nozzle_item");
    let color = |object: &SceneObject| {
        map_obj_string_tev_color(object, Some(&registry)).map(|definition| definition.color)
    };

    assert_eq!(color(&rocket), Some([255, 0, 0, 100]));
    assert_eq!(color(&hover), Some([90, 90, 120, 100]));
    assert_eq!(color(&legacy), Some([0, 0, 255, 100]));
    rocket.set_raw_param("nozzle_box_item", "Rocket_Nozzle_Item");
    assert_eq!(color(&rocket), Some([255, 255, 255, 100]));
    rocket.set_raw_param("actor_tail_string", "nozzlebox");
    assert_eq!(color(&rocket), None);

    let mut wrong_factory = SceneObject::new("wrong", "NozzleBoxAlias");
    wrong_factory.set_raw_param("actor_tail_string", "NozzleBox");
    wrong_factory.set_raw_param("nozzle_box_item", "normal_nozzle_item");
    assert_eq!(
        map_obj_string_tev_color(&wrong_factory, Some(&registry)),
        None
    );
}

#[test]
fn placement_stream_rgb_reaches_the_decomp_selected_tev_register() {
    let registry = ObjectRegistry {
        objects: vec![sms_schema::ObjectDefinition {
            factory_name: "FixturePaint".to_string(),
            class_name: "TFixturePaint".to_string(),
            category: "MapObj".to_string(),
            source: sms_schema::SchemaSource::MarNameRefGen,
            display_name: None,
            preview_model: None,
            hidden: false,
            unsafe_to_edit: false,
        }],
        map_obj_stream_tev_colors: vec![sms_schema::MapObjStreamTevColorDefinition {
            class_name: "TFixturePaint".to_string(),
            tev_register: 2,
            trailing_rgb_u32_count: 3,
            alpha: 255,
            source_file: "src/MoveBG/Fixture.cpp".to_string(),
        }],
        ..ObjectRegistry::default()
    };
    let mut object = SceneObject::new("paint", "FixturePaint");
    object.insert_source_raw_param("tev_red", "511");
    object.insert_source_raw_param("tev_green", "120");
    object.insert_source_raw_param("tev_blue", "305419785");

    assert_eq!(
        map_obj_stream_tev_color(&object, Some(&registry)),
        Some(sms_schema::MapObjTevColorDefinition {
            register: 2,
            color: [255, 120, 9, 255],
        })
    );
    object.factory_name = "UnrelatedPaint".to_string();
    assert_eq!(map_obj_stream_tev_color(&object, Some(&registry)), None);
}

#[test]
#[ignore = "requires the extracted retail game"]
fn retail_nozzle_boxes_keep_typed_items_and_tev_colors() {
    let base_root = std::env::var_os("SMS_BASE_ROOT")
        .map(PathBuf::from)
        .expect("set SMS_BASE_ROOT to the extracted game's root");
    let expected = [
        (
            "mamma0",
            vec![
                ("normal_nozzle_item", [0, 0, 255, 100]),
                ("rocket_nozzle_item", [255, 0, 0, 100]),
                ("back_nozzle_item", [90, 90, 120, 100]),
            ],
        ),
        ("dolpic0", vec![("rocket_nozzle_item", [255, 0, 0, 100])]),
        (
            "dolpic10",
            vec![
                ("normal_nozzle_item", [0, 0, 255, 100]),
                ("rocket_nozzle_item", [255, 0, 0, 100]),
                ("rocket_nozzle_item", [255, 0, 0, 100]),
                ("back_nozzle_item", [90, 90, 120, 100]),
            ],
        ),
    ];

    let decomp_root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../..");
    let registry = SchemaGenerator::new(decomp_root)
        .generate()
        .expect("generate decomp-derived NozzleBox colors");
    for (stage, mut expected) in expected {
        let document = StageDocument::open(&base_root, stage)
            .unwrap_or_else(|error| panic!("open retail {stage}: {error}"))
            .with_registry(registry.clone());
        let mut actual: Vec<_> = document
            .objects
            .iter()
            .filter(|object| object.factory_name == "NozzleBox")
            .map(|object| {
                (
                    object.raw_param("nozzle_box_item").unwrap_or_else(|| {
                        panic!("{stage} NozzleBox lost its typed item selector")
                    }),
                    map_obj_string_tev_color(object, document.registry.as_ref())
                        .map(|definition| definition.color)
                        .unwrap_or_else(|| panic!("{stage} NozzleBox lost its TEV color")),
                )
            })
            .collect();
        actual.sort_unstable();
        expected.sort_unstable();
        assert_eq!(actual, expected, "unexpected retail {stage} NozzleBoxes");
    }
}

#[test]
fn npc_root_material_colors_follow_schema_channels() {
    let mut monte = SceneObject::new("monte", "NPCMonteMA");
    monte
        .raw_params
        .insert("npc_body_color_index".to_string(), "9".to_string().into());
    monte
        .raw_params
        .insert("npc_cloth_color_index".to_string(), "3".to_string().into());
    let registry = ObjectRegistry {
        npc_actors: vec![sms_schema::NpcActorDefinition {
            actor_key: "MonteMA".to_string(),
            source_file: "src/NPC/NpcInitData.cpp".to_string(),
            parts: Vec::new(),
        }],
        npc_material_colors: vec![sms_schema::NpcMaterialColorDefinition {
            actor_key: "MonteMA".to_string(),
            model_index: 0,
            color_index_channel: 1,
            change: sms_schema::NpcColorChangeDefinition {
                mode: 2,
                material_name: "_fuku_mat".to_string(),
                colors0: vec![[1, 2, 3, 255]],
                colors1: vec![[4, 5, 6, 255]],
            },
            source_file: "src/NPC/NpcInitData.cpp".to_string(),
        }],
        ..ObjectRegistry::default()
    };

    assert_eq!(npc_root_color_index(&monte, 0), Some(9));
    assert_eq!(npc_root_color_index(&monte, 1), Some(3));
    assert_eq!(
        registry
            .npc_material_colors_for(&monte.factory_name)
            .next()
            .map(|definition| definition.change.material_name.as_str()),
        Some("_fuku_mat")
    );
}

#[test]
fn npc_pollution_uses_white_k_color_with_amount_as_alpha() {
    let mut monte = SceneObject::new("monte", "NPCMonteMA");
    monte
        .raw_params
        .insert("npc_pollution_amount".to_string(), "37".to_string().into());
    monte.raw_params.insert(
        "npc_parts_color_index_0".to_string(),
        "2".to_string().into(),
    );

    assert_eq!(npc_pollution_k_color(&monte), Some([255, 255, 255, 37]));
    let mut maremb = SceneObject::new("fisher", "NPCMareMB");
    maremb
        .raw_params
        .insert("npc_pollution_amount".to_string(), "0".to_string().into());
    assert_eq!(npc_pollution_k_color(&maremb), Some([255, 255, 255, 0]));
}

#[test]
fn material_table_candidates_include_base_actor_table() {
    assert_eq!(
        material_table_candidates_for_model("C:/game/dolpic.szs!/mapobj/kibako.bmd"),
        ["c:/game/dolpic.szs!/mapobj/kibako.bmt"]
    );
    assert_eq!(
        material_table_candidates_for_model("C:/game/dolpic.szs!/mapobj/kibako_crash.bmd"),
        [
            "c:/game/dolpic.szs!/mapobj/kibako_crash.bmt",
            "c:/game/dolpic.szs!/mapobj/kibako.bmt",
        ]
    );
    assert_eq!(
        material_table_candidates_for_model("C:/game/dolpic.szs!/mapobj/barrel_normal.bmd"),
        [
            "c:/game/dolpic.szs!/mapobj/barrel_normal.bmt",
            "c:/game/dolpic.szs!/mapobj/barrel.bmt",
        ]
    );
    assert_eq!(
        material_table_candidates_for_model("C:/game/dolpic.szs!/mapobj/barrel_offset.bmd"),
        [
            "c:/game/dolpic.szs!/mapobj/barrel_offset.bmt",
            "c:/game/dolpic.szs!/mapobj/barrel.bmt",
        ]
    );
    assert_eq!(
        material_table_candidates_for_model("C:/game/bianco0.szs!/mapobj/miniwindmilll.bmd"),
        [
            "c:/game/bianco0.szs!/mapobj/miniwindmilll.bmt",
            "c:/game/bianco0.szs!/mapobj/bianco.bmt",
        ]
    );
    assert_eq!(
        material_table_candidates_for_model("C:/game/mare0.szs!/marem/marem.bmd"),
        ["c:/game/mare0.szs!/marem/marem.bmt"]
    );
}

#[test]
fn dummy_texture_names_resolve_shared_material_tables() {
    let textures = [sms_formats::J3dTexturePreview {
        name: "J_barrel_dammy".to_string(),
        width: 8,
        height: 8,
        format: 0,
        wrap_s: 0,
        wrap_t: 0,
        min_filter: 1,
        mag_filter: 1,
        mipmap_enabled: false,
        do_edge_lod: false,
        bias_clamp: false,
        max_anisotropy: 0,
        min_lod: 0.0,
        max_lod: 0.0,
        lod_bias: 0.0,
        mipmap_count: 1,
        rgba: vec![255; 8 * 8 * 4],
        mips: Vec::new(),
    }];

    assert_eq!(
        material_table_asset_score(
            "C:/game/dolpic.szs!/mapobj/barrel_normal.bmd",
            &textures,
            "C:/game/dolpic.szs!/mapobj/barrel.bmt",
        ),
        Some((3, 1))
    );
    assert_eq!(
        material_table_asset_score(
            "C:/game/dolpic.szs!/mapobj/barrel_variant.bmd",
            &textures,
            "C:/game/dolpic.szs!/mapobj/barrel.bmt",
        ),
        Some((2, "barrel".len()))
    );
    assert_eq!(
        material_table_asset_score(
            "C:/game/dolpic.szs!/mapobj/barrel_variant.bmd",
            &textures,
            "C:/game/bianco.szs!/mapobj/barrel.bmt",
        ),
        None
    );
    assert_eq!(
        material_table_asset_score(
            "C:/game/dolpic.szs!/actors/barrel_variant.bmd",
            &textures,
            "C:/game/dolpic.szs!/mapobj/barrel.bmt",
        ),
        Some((2, "barrel".len()))
    );
}

#[test]
fn accessory_dummy_texture_resolves_archive_shared_material_table() {
    let textures = [sms_formats::J3dTexturePreview {
        name: "J_mare_dammy".to_string(),
        width: 8,
        height: 8,
        format: 0,
        wrap_s: 0,
        wrap_t: 0,
        min_filter: 1,
        mag_filter: 1,
        mipmap_enabled: false,
        do_edge_lod: false,
        bias_clamp: false,
        max_anisotropy: 0,
        min_lod: 0.0,
        max_lod: 0.0,
        lod_bias: 0.0,
        mipmap_count: 1,
        rgba: vec![255; 8 * 8 * 4],
        mips: Vec::new(),
    }];

    assert_eq!(
        material_table_asset_score(
            "stage.szs!/maremb/maremb_set.bmd",
            &textures,
            "stage.szs!/marecommon/mare.bmt",
        ),
        Some((2, "mare".len()))
    );
}

#[test]
fn normalized_dummy_names_resolve_differently_separated_model_names() {
    let textures = [sms_formats::J3dTexturePreview {
        name: "nozzleItem_dummy".to_string(),
        width: 1,
        height: 1,
        format: 0,
        wrap_s: 0,
        wrap_t: 0,
        min_filter: 1,
        mag_filter: 1,
        mipmap_enabled: false,
        do_edge_lod: false,
        bias_clamp: false,
        max_anisotropy: 0,
        min_lod: 0.0,
        max_lod: 0.0,
        lod_bias: 0.0,
        mipmap_count: 1,
        rgba: vec![255; 4],
        mips: Vec::new(),
    }];

    assert_eq!(
        material_table_asset_score(
            "stage.szs!/mapobj/normal_nozzle_item.bmd",
            &textures,
            "stage.szs!/mapobj/nozzleItem.bmt",
        ),
        Some((2, "nozzleitem".len()))
    );
}

#[test]
fn npc_starting_animation_uses_family_wait_resource() {
    let monte = SceneObject::new("monte", "NPCMonteMA");
    assert_eq!(
        starting_joint_animation_candidates(
            &monte,
            "C:/game/dolpic0.szs!/montema/moma_model.bmd",
            None,
        ),
        [
            "C:/game/dolpic0.szs!/montema/montema_wait.bck",
            "C:/game/dolpic0.szs!/montemcommon/mom_wait.bck",
            "C:/game/dolpic0.szs!/montem/mom_wait.bck",
        ]
    );

    let mare = SceneObject::new("mare", "NPCMareMB");
    assert_eq!(
        starting_joint_animation_candidates(&mare, "C:/game/mare0.szs!/marem/marem.bmd", None,),
        [
            "C:/game/mare0.szs!/maremb/maremb_wait.bck",
            "C:/game/mare0.szs!/marem/marem_wait.bck",
        ]
    );
}

#[test]
fn explicit_animation_candidates_precede_heuristic_fallbacks() {
    let monte = SceneObject::new("monte", "NPCMonteMA");
    assert_eq!(
        starting_joint_animation_candidates(
            &monte,
            "C:/game/dolpic0.szs!/montema/moma_model.bmd",
            Some("C:/game/dolpic0.szs!/bck/explicit_wait.bck"),
        ),
        [
            "C:/game/dolpic0.szs!/bck/explicit_wait.bck",
            "C:/game/dolpic0.szs!/montema/montema_wait.bck",
            "C:/game/dolpic0.szs!/montemcommon/mom_wait.bck",
            "C:/game/dolpic0.szs!/montem/mom_wait.bck",
        ]
    );
    assert_eq!(
        starting_texture_pattern_candidates(
            &monte,
            "C:/game/dolpic0.szs!/montema/moma_model.bmd",
            Some("C:/game/dolpic0.szs!/btp/explicit_wink.btp"),
        ),
        [
            "C:/game/dolpic0.szs!/btp/explicit_wink.btp",
            "C:/game/dolpic0.szs!/montemcommon/moma_wink.btp",
        ]
    );

    let mario = SceneObject::new("player", "Mario");
    assert_eq!(
        starting_joint_animation_candidates(
            &mario,
            "C:/game/files/data/mario.szs!/bmd/ma_mdl1.bmd",
            Some("C:/game/files/data/mario.szs!/bck/ma_wait.bck"),
        ),
        ["C:/game/files/data/mario.szs!/bck/ma_wait.bck"]
    );
    assert_eq!(
        starting_texture_pattern_candidates(
            &mario,
            "C:/game/files/data/mario.szs!/bmd/ma_mdl1.bmd",
            Some("C:/game/files/data/mario.szs!/btp/ma_wink_tx.btp"),
        ),
        ["C:/game/files/data/mario.szs!/btp/ma_wink_tx.btp"]
    );
}

#[test]
fn explicit_texture_pattern_animation_starts_at_phase_zero() {
    assert_eq!(
        starting_texture_pattern_phase_seconds("mario", 120, true),
        0.0
    );
    assert_eq!(
        starting_texture_pattern_phase_seconds("heuristic-npc", 0, false),
        0.0
    );

    let heuristic_phase = starting_texture_pattern_phase_seconds("heuristic-npc", 120, false);
    assert!(heuristic_phase > 0.0);
    assert_eq!(
        heuristic_phase,
        (stable_string_hash("heuristic-npc") % 120) as f32 / 60.0
    );
}

#[test]
fn object_preview_k_color_alpha_overrides_preserve_every_other_channel() {
    let mut colors = [
        [79, 108, 97, 128],
        [1, 2, 3, 4],
        [5, 6, 7, 8],
        [9, 10, 11, 12],
    ];
    apply_object_preview_k_color_alpha_overrides(
        &mut colors,
        &[
            sms_schema::ObjectPreviewTevKColorAlphaOverride {
                register: 0,
                alpha: 0,
            },
            sms_schema::ObjectPreviewTevKColorAlphaOverride {
                register: 3,
                alpha: 17,
            },
            sms_schema::ObjectPreviewTevKColorAlphaOverride {
                register: 9,
                alpha: 255,
            },
        ],
    );

    assert_eq!(colors[0], [79, 108, 97, 0]);
    assert_eq!(colors[1], [1, 2, 3, 4]);
    assert_eq!(colors[2], [5, 6, 7, 8]);
    assert_eq!(colors[3], [9, 10, 11, 17]);
}

#[test]
fn hidden_object_preview_shapes_keep_animated_triangle_alignment() {
    let hidden_shapes = [10];
    assert!(
        !object_preview_shape_is_hidden(4, &hidden_shapes),
        "the embedded cap remains visible until the separate cap model is previewed"
    );
    let initial_shapes = [9, 10, 11];
    let initial_visible = initial_shapes
        .into_iter()
        .filter(|shape| !object_preview_shape_is_hidden(*shape, &hidden_shapes))
        .collect::<Vec<_>>();
    assert_eq!(initial_visible, [9, 11]);

    let posed_triangles = [(9, "body"), (10, "shirt"), (11, "later shape")];
    let animated_visible = posed_triangles
        .into_iter()
        .filter(|(shape, _)| !object_preview_shape_is_hidden(*shape, &hidden_shapes))
        .collect::<Vec<_>>();
    assert_eq!(animated_visible, [(9, "body"), (11, "later shape")]);
}

#[test]
fn level_transformation_overrides_scrub_from_retail_start_to_bind_pose() {
    let target = LevelTransformTarget {
        joint_index: 7,
        translation_offset: [0.0, -1500.0, 0.0],
        scale_multiplier: [1.0, 0.008, 1.0],
        behavior: LevelTransformBehavior::Linear,
    };

    let start = level_transform_overrides(&[target], 0.0)[0];
    assert_eq!(start.translation_offset, [0.0, -1500.0, 0.0]);
    assert_eq!(start.scale_multiplier, [1.0, 0.008, 1.0]);

    let middle = level_transform_overrides(&[target], 0.5)[0];
    assert_eq!(middle.translation_offset, [0.0, -750.0, 0.0]);
    assert!((middle.scale_multiplier[1] - 0.504).abs() < 0.0001);

    let end = level_transform_overrides(&[target], 1.0)[0];
    assert_eq!(end.translation_offset, [0.0; 3]);
    assert_eq!(end.scale_multiplier, [1.0; 3]);
}

#[test]
fn linked_pollution_meshes_follow_retail_visibility_swap() {
    let hidden = LevelTransformTarget {
        joint_index: 3,
        translation_offset: [0.0; 3],
        scale_multiplier: [1.0; 3],
        behavior: LevelTransformBehavior::AlwaysHidden,
    };
    let cleaned = LevelTransformTarget {
        joint_index: 4,
        translation_offset: [0.0; 3],
        scale_multiplier: [1.0; 3],
        behavior: LevelTransformBehavior::HideAfterStart,
    };

    assert!(level_transform_target_is_hidden(&hidden, 0.0));
    assert!(!level_transform_target_is_hidden(&cleaned, 0.0));
    assert!(level_transform_target_is_hidden(&cleaned, 0.1));
    assert_eq!(
        level_transform_overrides(&[hidden], 0.0)[0].scale_multiplier,
        [1.0; 3]
    );
}

#[test]
fn gatekeeper_uses_retail_sleep_and_texture_animations() {
    let gatekeeper = SceneObject::new("boss", "GateKeeper");
    let model = "C:/game/dolpic0.szs!/gatekeeper/gene_pakkun_model1.bmd";

    assert_eq!(
        starting_joint_animation_candidates(&gatekeeper, model, None),
        ["C:/game/dolpic0.szs!/gatekeeper/gene_pakkun_wait1.bck"]
    );
    assert_eq!(
        model_texture_srt_animation_paths(model),
        [
            "C:/game/dolpic0.szs!/gatekeeper/gene_pakkun_tex0.btk",
            "C:/game/dolpic0.szs!/gatekeeper/gene_pakkun_tex1.btk",
        ]
    );
}

#[test]
fn gatekeeper_replaces_its_dummy_with_the_stage_pollution_texture() {
    assert_eq!(
        actor_runtime_texture_replacements("GateKeeper", None),
        [(
            "Q_kepper_dummy_128IA4".to_string(),
            "/map/pollution/h_ma_rak.bti".to_string()
        )]
    );
    assert!(actor_runtime_texture_replacements("gatekeeper", None).is_empty());
}

#[test]
fn pakkun_family_uses_the_decomp_runtime_pollution_texture_binding() {
    let registry = ObjectRegistry {
        runtime_texture_replacements: ["Pakkun", "StayPakkun"]
            .into_iter()
            .map(
                |factory_name| sms_schema::RuntimeTextureReplacementDefinition {
                    factory_name: factory_name.to_string(),
                    dummy_texture_name: "H_ma_rak_dummy".to_string(),
                    resource_path: "/scene/map/pollution/H_ma_rak.bti".to_string(),
                    source_file: "src/Enemy/pakkun.cpp".to_string(),
                },
            )
            .collect(),
        ..ObjectRegistry::default()
    };
    let expected = [(
        "H_ma_rak_dummy".to_string(),
        "/map/pollution/h_ma_rak.bti".to_string(),
    )];

    assert_eq!(
        actor_runtime_texture_replacements("Pakkun", Some(&registry)),
        expected
    );
    assert_eq!(
        actor_runtime_texture_replacements("StayPakkun", Some(&registry)),
        expected
    );
    assert!(actor_runtime_texture_replacements("BossPakkun", Some(&registry)).is_empty());
}

#[test]
fn stay_pakkun_preview_replaces_every_dummy_with_the_stage_goop_texture() {
    fn texture(name: &str, value: u8) -> sms_formats::J3dTexturePreview {
        sms_formats::J3dTexturePreview {
            name: name.to_string(),
            width: 1,
            height: 1,
            format: 6,
            wrap_s: 0,
            wrap_t: 0,
            min_filter: 0,
            mag_filter: 0,
            mipmap_enabled: false,
            do_edge_lod: false,
            bias_clamp: false,
            max_anisotropy: 0,
            min_lod: 0.0,
            max_lod: 0.0,
            lod_bias: 0.0,
            mipmap_count: 1,
            rgba: vec![value; 4],
            mips: Vec::new(),
        }
    }

    let root = tempfile::tempdir().unwrap();
    let texture_path = root.path().join("map/pollution/H_ma_rak.bti");
    std::fs::create_dir_all(texture_path.parent().unwrap()).unwrap();
    let stage_rgba = vec![
        0x12, 0x34, 0x56, 0x78, 0x21, 0x43, 0x65, 0x87, 0x9a, 0xbc, 0xde, 0xf0, 0xaa, 0x55, 0x11,
        0xee,
    ];
    let image = sms_formats::RgbaImage::new(2, 2, stage_rgba.clone()).unwrap();
    let encoded = sms_formats::GxEncodedTexture::encode_rgba(
        "H_ma_rak",
        &image,
        sms_formats::GxTextureEncodeOptions {
            encoding: sms_formats::GxTextureEncoding::Exact(sms_formats::GxTextureFormat::Rgba8),
            ..Default::default()
        },
    )
    .unwrap();
    std::fs::write(&texture_path, encoded.to_bti().unwrap().encode().unwrap()).unwrap();

    let mut document = test_document(Vec::new());
    document.base_root = root.path().to_path_buf();
    document.registry = Some(ObjectRegistry {
        runtime_texture_replacements: vec![sms_schema::RuntimeTextureReplacementDefinition {
            factory_name: "StayPakkun".to_string(),
            dummy_texture_name: "H_ma_rak_dummy".to_string(),
            resource_path: "/scene/map/pollution/H_ma_rak.bti".to_string(),
            source_file: "src/Enemy/pakkun.cpp".to_string(),
        }],
        ..ObjectRegistry::default()
    });
    document.assets.push(StageAsset {
        path: texture_path,
        kind: StageAssetKind::Texture,
    });
    let mut preview = sms_formats::J3dGeometryPreview {
        positions: Vec::new(),
        triangles: Vec::new(),
        textures: vec![
            texture("H_ma_rak_dummy", 1),
            texture("unrelated", 2),
            texture("H_MA_RAK_DUMMY", 3),
        ],
        materials: Vec::new(),
        bounds_min: [0.0; 3],
        bounds_max: [0.0; 3],
        adjusted_zero_normals: 0,
    };

    apply_actor_runtime_textures(
        &document,
        &SceneObject::new("fixed pakkun", "StayPakkun"),
        &mut preview,
    );

    assert_eq!(preview.textures[0].rgba, stage_rgba);
    assert_eq!(preview.textures[2].rgba, preview.textures[0].rgba);
    assert_eq!(preview.textures[0].name, "H_ma_rak_dummy");
    assert_eq!(preview.textures[2].name, "H_ma_rak_dummy");
    assert_eq!(preview.textures[1].rgba, vec![2; 4]);
}

#[test]
fn monte_starting_eye_pattern_uses_retail_variant_resource() {
    let monte = SceneObject::new("monte", "NPCMonteMA");
    assert_eq!(
        starting_texture_pattern_candidates(
            &monte,
            "C:/game/dolpic10.szs!/montema/moma_model.bmd",
            None,
        ),
        ["C:/game/dolpic10.szs!/montemcommon/moma_wink.btp"]
    );
}

#[test]
fn npc_eye_material_names_are_treated_as_two_sided_decals() {
    assert!(is_npc_eye_material_name("_eye_mat"));
    assert!(is_npc_eye_material_name("1_eye_mat"));
    assert!(!is_npc_eye_material_name("_hand_mat"));
}

#[test]
fn enemy_material_colors_override_only_decomp_assigned_channels() {
    let registry = ObjectRegistry {
        enemy_material_colors: vec![sms_schema::EnemyMaterialTevColorDefinition {
            factory_name: "PoiHanaRed".to_string(),
            material_name: "_body".to_string(),
            tev_register: 0,
            color: [Some(283), Some(-53), Some(-122), None],
            source_file: "src/Enemy/poihana.cpp".to_string(),
        }],
        ..ObjectRegistry::default()
    };
    let mut tev_colors = [[0; 4]; 4];
    tev_colors[0] = [1, 2, 3, 77];

    apply_enemy_tev_overrides(&mut tev_colors, "_body", "PoiHanaRed", Some(&registry));

    assert_eq!(tev_colors[0], [283, -53, -122, 77]);

    let mut wrong_case = [[0; 4]; 4];
    apply_enemy_tev_overrides(&mut wrong_case, "_body", "poihanared", Some(&registry));
    assert_eq!(wrong_case, [[0; 4]; 4]);
}

#[test]
fn surf_geso_shared_model_colors_follow_exact_decomp_resource_variants() {
    let variants = [
        ("SurfGesoRed", [255, 180, 255, 255]),
        ("SurfGesoYellow", [255, 255, 125, 255]),
        ("SurfGesoGreen", [180, 255, 180, 255]),
    ];
    let registry = ObjectRegistry {
        objects: variants
            .iter()
            .map(|(factory_name, _)| ObjectDefinition {
                factory_name: (*factory_name).to_string(),
                class_name: "TSurfGesoObj".to_string(),
                category: "MapObj".to_string(),
                source: sms_schema::SchemaSource::MarNameRefGen,
                display_name: None,
                preview_model: None,
                hidden: false,
                unsafe_to_edit: false,
            })
            .collect(),
        map_obj_model_overrides: variants
            .iter()
            .map(
                |(resource_name, color)| sms_schema::MapObjModelOverrideDefinition {
                    resource_name: (*resource_name).to_string(),
                    class_name: "TSurfGesoObj".to_string(),
                    model_path: "/scene/mapObj/surfgeso.bmd".to_string(),
                    load_flags: 0x1022_0000,
                    tev_color: Some(sms_schema::MapObjTevColorDefinition {
                        register: 1,
                        color: *color,
                    }),
                    binding_source_file: "src/MoveBG/MapObjRicco.cpp".to_string(),
                    model_source_file: "src/MoveBG/MapObjManager.cpp".to_string(),
                },
            )
            .collect(),
        ..ObjectRegistry::default()
    };

    for (factory_name, expected) in variants {
        let mut object = SceneObject::new(factory_name, factory_name);
        object.set_raw_param("actor_tail_string", factory_name);
        assert_eq!(
            map_obj_model_override_tev_color(&object, Some(&registry)),
            Some(sms_schema::MapObjTevColorDefinition {
                register: 1,
                color: expected
            })
        );
    }

    let mut wrong_class = SceneObject::new("wrong", "Shine");
    wrong_class.set_raw_param("actor_tail_string", "SurfGesoRed");
    assert_eq!(
        map_obj_model_override_tev_color(&wrong_class, Some(&registry)),
        None
    );
}

#[test]
fn npc_parts_mask_uses_decomp_schema_metadata() {
    let mut document = test_document(Vec::new());
    document.registry = Some(ObjectRegistry {
        npc_actors: vec![sms_schema::NpcActorDefinition {
            actor_key: "MareM".to_string(),
            source_file: "src/NPC/NpcInitData.cpp".to_string(),
            parts: vec![sms_schema::NpcPartDefinition {
                bit_index: 0,
                color_index_channel: 0,
                models: vec![sms_schema::NpcPartModelDefinition {
                    joint_name: Some("kubi".to_string()),
                    model_name: "custom_hat.bmd".to_string(),
                }],
                color_changes: vec![sms_schema::NpcColorChangeDefinition {
                    mode: 2,
                    material_name: "_hat".to_string(),
                    colors0: vec![[10, 20, 30, 255]],
                    colors1: vec![[40, 50, 60, 255]],
                }],
                uses_pollution: true,
                uses_shared_materials: true,
            }],
        }],
        ..ObjectRegistry::default()
    });
    let mut mare = SceneObject::new("mare", "NPCMareMA");
    mare.raw_params
        .insert("npc_parts_mask".to_string(), "1".to_string().into());
    let parts = npc_accessory_specs(&document, &mare);

    assert_eq!(parts.len(), 1);
    assert_eq!(parts[0].joint_name.as_deref(), Some("kubi"));
    assert_eq!(parts[0].asset_suffix, "/custom_hat.bmd");
    assert_eq!(parts[0].color_index_channel, 0);
    assert_eq!(parts[0].color_changes[0].material_name, "_hat");
    assert_eq!(parts[0].color_changes[0].colors1[0], [40, 50, 60, 255]);
    assert!(parts[0].uses_pollution);

    // Retail NPC placements use -1 as a no-parts sentinel. The game clamps
    // it to zero before testing the schema-derived part bits.
    mare.raw_params
        .insert("npc_parts_mask".to_string(), "-1".to_string().into());
    assert!(npc_accessory_specs(&document, &mare).is_empty());
}

#[test]
fn peach_hair_parts_use_their_retail_wait_animations() {
    assert_eq!(
        accessory_joint_animation_path("stage.szs!/peach/peach_hair_normal.bmd").as_deref(),
        Some("stage.szs!/peach/peach_hair_normal_wait.bck")
    );
    assert_eq!(
        accessory_joint_animation_path("stage.szs!/peach/peach_hair_ponytail.bmd").as_deref(),
        Some("stage.szs!/peach/peach_hair_ponytail_wait.bck")
    );
    assert_eq!(
        accessory_joint_animation_path("stage.szs!/custom/lantern.bdl").as_deref(),
        Some("stage.szs!/custom/lantern_wait.bck")
    );
}

#[test]
fn npc_circle_shadow_uses_retail_default_radius() {
    let mut triangles = Vec::new();
    push_npc_circle_shadow(
        &mut triangles,
        Transform {
            translation: [10.0, 20.0, 30.0],
            ..Transform::default()
        },
        4,
        8,
    );

    assert_eq!(triangles.len(), 20);
    assert_eq!(triangles[0].render_layer, PreviewRenderLayer::Shadow);
    assert_eq!(triangles[0].vertices[0], [10.0, 21.5, 30.0]);
    assert!((triangles[0].vertices[1][0] - 70.0).abs() < 0.001);
    assert_eq!(triangles[0].blend_mode.unwrap().mode, 1);
}

#[test]
fn coin_circle_shadow_uses_retail_radius_on_the_world_surface() {
    let mut world = textured_blended_triangle();
    world.vertices = [
        [-100.0, 10.0, -100.0],
        [100.0, 10.0, -100.0],
        [0.0, 10.0, 100.0],
    ];
    let mut water = world;
    water.vertices = water.vertices.map(|mut vertex| {
        vertex[1] = 20.0;
        vertex
    });
    water.render_layer = PreviewRenderLayer::Water;

    let transform = Transform {
        translation: [0.0, 50.0, 0.0],
        scale: [0.7, 0.7, 0.7],
        ..Transform::default()
    };
    let ground_y = shadow_ground_height(transform.translation, &[world, water]).unwrap();
    let mut shadows = Vec::new();
    push_coin_circle_shadow(&mut shadows, transform, ground_y, 4, 8);

    assert_eq!(ground_y, 10.0);
    assert_eq!(shadows.len(), 20);
    assert_eq!(shadows[0].vertices[0], [0.0, 11.5, 0.0]);
    assert!((shadows[0].vertices[1][0] - 35.0).abs() < 0.001);
    assert_eq!(shadows[0].render_layer, PreviewRenderLayer::Shadow);
}

#[test]
fn invisible_coin_proxy_does_not_get_a_preview_shadow() {
    let mut object = SceneObject::new("coin-proxy", "Coin");
    object.set_raw_param("stream_string_0", "コイン キャラ");
    object.set_raw_param("actor_tail_string", "invisible_coin");

    assert!(!is_coin_object(&object));
}

#[test]
fn monte_model_loader_flags_follow_manager_entries() {
    assert_eq!(
        actor_model_loader_flags(&SceneObject::new("ma", "NPCMonteMA")),
        Some(0x1030_0000)
    );
    assert_eq!(
        actor_model_loader_flags(&SceneObject::new("md", "NPCMonteMD")),
        Some(0x1021_0000)
    );
    assert_eq!(
        actor_model_loader_flags(&SceneObject::new("boss", "GateKeeper")),
        None,
        "enemy loader flags come from the decomp-derived preview catalog"
    );
    assert_eq!(
        actor_model_loader_flags(&SceneObject::new("mare-m", "NPCMareMD")),
        Some(0x1030_0000)
    );
    assert_eq!(
        actor_model_loader_flags(&SceneObject::new("mare-w", "NPCMareWB")),
        Some(0x1030_0000)
    );
    assert_eq!(
        actor_model_loader_flags(&SceneObject::new("wrong-case", "npcMonteMA")),
        None
    );
}

#[test]
fn npc_archive_models_are_supported_object_previews() {
    assert!(is_supported_object_preview_model_path(
        "stage.szs!/montema/moma_model.bmd"
    ));
    assert!(is_supported_object_preview_model_path(
        "/scene/kinopio/kinopio_body.bmd"
    ));
    assert!(is_supported_object_preview_model_path(
        "stage.szs!/sambohead/sambohead.bmd"
    ));
}

#[test]
fn world_model_path_normalization_deduplicates_scene_instances() {
    let world_models = BTreeSet::from([normalized_preview_asset_path(
        r"C:\game\dolpic0.szs!/map/map/sky.bmd",
    )]);

    assert!(!should_instance_object_preview_model(
        "C:/GAME/dolpic0.szs!/map/map/sky.bmd",
        &world_models
    ));
    assert!(should_instance_object_preview_model(
        "C:/game/dolpic0.szs!/montema/moma_model.bmd",
        &world_models
    ));
    assert!(should_instance_object_preview_model(
        "C:/game/dolpic0.szs!/sambohead/sambohead.bmd",
        &world_models
    ));
}

#[test]
fn palm_leaf_placement_is_kept_as_an_object_preview() {
    let mut palm_leaf = SceneObject::new("PalmLeaf 2", "Palm");
    palm_leaf
        .raw_params
        .insert("name".to_string(), "PalmLeaf 2".to_string().into());
    palm_leaf.asset_hints.push(AssetRef {
        path: "stage.szs!/mapobj/palmleaf.bmd".to_string(),
        role: AssetRole::PreviewModel,
    });

    assert_eq!(
        object_preview_model_path(&palm_leaf, &BTreeSet::new()).as_deref(),
        Some("stage.szs!/mapobj/palmleaf.bmd")
    );
}

#[test]
fn explicit_preview_hints_stay_distinct_from_inferred_fallbacks() {
    let mut object = SceneObject::new("boss", "BossTelesa");
    object.asset_hints.push(AssetRef {
        path: "stage.szs!/btelesa/guessed.bmd".to_string(),
        role: AssetRole::InferredPreviewModel,
    });

    assert!(object_preview_model_path(&object, &BTreeSet::new()).is_none());
    assert_eq!(
        object_inferred_preview_model_path(&object, &BTreeSet::new()).as_deref(),
        Some("stage.szs!/btelesa/guessed.bmd")
    );

    object.asset_hints.push(AssetRef {
        path: "stage.szs!/btelesa/explicit.bmd".to_string(),
        role: AssetRole::PreviewModel,
    });
    assert_eq!(
        object_preview_model_path(&object, &BTreeSet::new()).as_deref(),
        Some("stage.szs!/btelesa/explicit.bmd")
    );
}

fn preview_for_texture_alpha(has_alpha: bool, has_translucent_alpha: bool) -> ModelPreview {
    let image = egui::ColorImage::filled([1, 1], egui::Color32::WHITE);
    ModelPreview {
        points: Vec::new(),
        triangles: Vec::new(),
        collision_triangles: Vec::new(),
        collision_file_count: 0,
        collision_surface_count: 0,
        failed_collision_files: 0,
        collision_failures: Vec::new(),
        textures: vec![PreviewTexture {
            image: image.clone(),
            mips: vec![image],
            format: 6,
            wrap_s: 1,
            wrap_t: 1,
            min_filter: 1,
            mag_filter: 1,
            mipmap_enabled: false,
            do_edge_lod: false,
            bias_clamp: false,
            max_anisotropy: 0,
            min_lod: 0.0,
            max_lod: 0.0,
            lod_bias: 0.0,
            mipmap_count: 1,
            has_alpha,
            has_translucent_alpha,
        }],
        materials: Vec::new(),
        texture_srt_animations: Vec::new(),
        texture_pattern_animations: Vec::new(),
        material_animation_bindings: Vec::new(),
        pollution_texture_indices: BTreeMap::new(),
        bounds_min: [0.0, 0.0, 0.0],
        bounds_max: [1.0, 1.0, 1.0],
        camera_bounds_min: [0.0, 0.0, 0.0],
        camera_bounds_max: [1.0, 1.0, 1.0],
        loaded_models: 1,
        failed_models: 0,
        model_warnings: Vec::new(),
        model_failures: Vec::new(),
        source_vertices: 0,
        source_triangles: 0,
        source_textures: 1,
        goop_surface_model_indices: BTreeSet::new(),
        object_model_indices: BTreeMap::new(),
        instance_model_indices: BTreeMap::new(),
        mirror_actor_positions: BTreeMap::new(),
        mirror_cubes: Vec::new(),
        mirror_model_slots: BTreeMap::new(),
        animated_models: Vec::new(),
        animated_flags: Vec::new(),
        rotating_models: Vec::new(),
        level_transform_models: Vec::new(),
        level_transform_particles: Vec::new(),
        actor_particles: Vec::new(),
        level_transform_duration_frames: 600.0,
        level_transform_particle_end_frames: 600.0,
    }
}

fn preview_for_alpha_texture(has_translucent_alpha: bool) -> ModelPreview {
    preview_for_texture_alpha(true, has_translucent_alpha)
}

fn textured_blended_triangle() -> PreviewTriangle {
    PreviewTriangle {
        vertices: [[0.0, 0.0, 0.0], [1.0, 0.0, 0.0], [0.0, 1.0, 0.0]],
        normals: None,
        color_channels: [None; 2],
        tex_coord_sets: [None; 8],
        material_index: None,
        packet_index: 0,
        model_index: 1,
        render_layer: PreviewRenderLayer::Main,
        color: None,
        vertex_colors: None,
        combine_mode: J3dPreviewCombineMode::TextureOnly,
        tex_coords: Some([[0.0, 0.0], [1.0, 0.0], [0.0, 1.0]]),
        texture_index: Some(0),
        mask_tex_coords: None,
        mask_texture_index: None,
        cull_mode: None,
        alpha_compare: None,
        blend_mode: Some(J3dBlendMode {
            mode: 1,
            src_factor: 4,
            dst_factor: 5,
            logic_op: 0,
        }),
        z_mode: None,
        billboard: None,
        particle_type: None,
        particle_pivot: None,
        particle_direction: None,
        particle_color_mode: None,
        particle_environment_color: None,
        particle_extra_texture: None,
    }
}

#[test]
fn rmb_free_look_keeps_camera_position_fixed() {
    let mut app = camera_app();
    let old_position = app.camera_frame().position;

    app.rotate_camera_in_place(egui::vec2(80.0, -30.0));

    assert_vec3_close(app.camera_frame().position, old_position);
}

#[test]
fn rmb_horizontal_drag_uses_unreal_style_yaw_sign() {
    let mut app = camera_app();

    app.rotate_camera_in_place(egui::vec2(80.0, 0.0));

    assert!(app.renderer.camera().yaw_degrees < 0.0);
}

#[test]
fn alt_orbit_uses_same_horizontal_yaw_sign() {
    let mut app = camera_app();

    app.orbit_camera(egui::vec2(80.0, 0.0));

    assert!(app.renderer.camera().yaw_degrees < 0.0);
}

#[test]
fn gizmo_move_snaps_only_the_dragged_axis() {
    let drag = GizmoDrag {
        axis: GizmoAxis::X,
        tool: EditorTool::Move,
        start_pointer: egui::pos2(100.0, 100.0),
        screen_origin: egui::pos2(100.0, 100.0),
        screen_direction: egui::vec2(1.0, 0.0),
        world_units_per_pixel: 1.0,
        start_transform: Transform {
            translation: [13.0, 17.0, 23.0],
            rotation_degrees: [7.0, 11.0, 19.0],
            scale: [1.1, 1.2, 1.3],
        },
    };

    let transformed = viewport_ui::transform_from_gizmo_drag(
        drag,
        egui::pos2(160.0, 100.0),
        true,
        50.0,
        15.0,
        0.1,
    );

    assert_eq!(transformed.translation, [50.0, 17.0, 23.0]);
    assert_eq!(transformed.rotation_degrees, [7.0, 11.0, 19.0]);
    assert_eq!(transformed.scale, [1.1, 1.2, 1.3]);
}

#[test]
fn gizmo_rotate_and_scale_edit_only_the_active_axis() {
    let start_transform = Transform {
        translation: [13.0, 17.0, 23.0],
        rotation_degrees: [7.0, 11.0, 19.0],
        scale: [1.1, 1.2, 1.3],
    };
    let rotated = viewport_ui::transform_from_gizmo_drag(
        GizmoDrag {
            axis: GizmoAxis::Y,
            tool: EditorTool::Rotate,
            start_pointer: egui::pos2(164.0, 100.0),
            screen_origin: egui::pos2(100.0, 100.0),
            screen_direction: egui::vec2(1.0, 0.0),
            world_units_per_pixel: 1.0,
            start_transform,
        },
        egui::pos2(100.0, 164.0),
        true,
        50.0,
        15.0,
        0.1,
    );
    let scaled = viewport_ui::transform_from_gizmo_drag(
        GizmoDrag {
            axis: GizmoAxis::Z,
            tool: EditorTool::Scale,
            start_pointer: egui::pos2(100.0, 100.0),
            screen_origin: egui::pos2(100.0, 100.0),
            screen_direction: egui::vec2(0.0, 1.0),
            world_units_per_pixel: 1.0,
            start_transform,
        },
        egui::pos2(100.0, 160.0),
        true,
        50.0,
        15.0,
        0.1,
    );

    assert_eq!(rotated.rotation_degrees, [7.0, 105.0, 19.0]);
    assert_eq!(rotated.translation, start_transform.translation);
    assert_eq!(rotated.scale, start_transform.scale);
    assert_eq!(scaled.scale[0..2], start_transform.scale[0..2]);
    assert!(scaled.scale[2] > start_transform.scale[2]);
    assert_eq!(scaled.translation, start_transform.translation);
    assert_eq!(scaled.rotation_degrees, start_transform.rotation_degrees);
}

#[test]
fn authored_water_reflections_follow_environment_visibility() {
    let path = "stage.szs!/map/map/reflectparts.bmd";

    assert!(path_is_water_reflection_model_path(path));
    assert!(is_default_preview_model_path(path, true, true, false));
    assert!(!is_default_preview_model_path(path, false, true, true));
    assert_eq!(
        preview_render_layer_for_model_path(path),
        PreviewRenderLayer::MirrorScene
    );
    assert!(!is_camera_bounds_model_path(path));

    // Sunshine copies the main sky's material table onto ReflectSky before
    // drawing it. The editor mirrors its already-loaded sky instead of showing
    // the unpatched helper geometry.
    let reflect_sky = "stage.szs!/map/map/reflectsky.bmd";
    assert!(path_is_mirror_sky_helper_model_path(reflect_sky));
    assert!(!path_is_water_reflection_model_path(reflect_sky));
    assert!(!is_default_preview_model_path(
        reflect_sky,
        true,
        true,
        false
    ));
    assert!(!is_default_preview_model_path(
        reflect_sky,
        true,
        true,
        true
    ));
    assert_eq!(
        preview_render_layer_for_model_path(reflect_sky),
        PreviewRenderLayer::MirrorScene
    );
}

#[test]
fn shimmer_models_are_controlled_by_effect_visibility() {
    let path = "stage.szs!/mapobj/shimmerhi.bmd";

    assert_eq!(
        preview_render_layer_for_model_path(path),
        PreviewRenderLayer::Heatwave
    );
    assert!(preview_render_layer_is_effect(PreviewRenderLayer::Heatwave));
    assert!(preview_render_layer_is_effect(PreviewRenderLayer::Particle));
    assert!(!preview_render_layer_is_effect(PreviewRenderLayer::Main));
}

#[test]
fn authored_mirror_surface_follows_environment_visibility() {
    let path = "stage.szs!/map/mirror/mirror00.bmd";

    assert!(path_is_mirror_surface_model_path(path));
    assert!(is_default_preview_model_path(path, true, true, false));
    assert!(!is_default_preview_model_path(path, false, true, true));
    assert_eq!(
        preview_render_layer_for_model_path(path),
        PreviewRenderLayer::MirrorSurface
    );
    assert!(!is_camera_bounds_model_path(path));
}

#[test]
fn mirror_surface_slots_follow_the_runtime_filename_mapping() {
    assert_eq!(
        mirror_surface_model_slot("bianco7", "stage.szs!/map/mirror/mirror00.bmd"),
        Some(0)
    );
    assert_eq!(
        mirror_surface_model_slot("pinna0", "stage.szs!/map/mirror/mirror205.bmd"),
        Some(0)
    );
    assert_eq!(
        mirror_surface_model_slot("bianco7", "stage.szs!/map/map/map.bmd"),
        None
    );

    let active = BTreeSet::from([0]);
    assert!(mirror_surface_model_is_active(
        "bianco7",
        "stage.szs!/map/mirror/mirror00.bmd",
        &active,
    ));
    assert!(!mirror_surface_model_is_active(
        "bianco7",
        "stage.szs!/map/mirror/mirror01.bmd",
        &active,
    ));
}

#[test]
fn mirror_cube_membership_matches_sunshines_bottom_anchored_rotated_volume() {
    let axis_aligned = PreviewMirrorCube {
        center: [10.0, 20.0, 30.0],
        rotation_degrees: [0.0; 3],
        dimensions: [100.0, 200.0, 300.0],
        model_slot: 0,
    };
    assert!(axis_aligned.contains([10.0, 20.1, 30.0]));
    assert!(axis_aligned.contains([59.9, 219.9, 179.9]));
    assert!(!axis_aligned.contains([10.0, 20.0, 30.0]));
    assert!(!axis_aligned.contains([10.0, 220.0, 30.0]));
    assert!(!axis_aligned.contains([60.0, 100.0, 30.0]));

    let yawed = PreviewMirrorCube {
        center: [0.0; 3],
        rotation_degrees: [0.0, 90.0, 0.0],
        dimensions: [100.0, 100.0, 20.0],
        model_slot: 1,
    };
    assert!(yawed.contains([0.0, 50.0, 40.0]));
    assert!(!yawed.contains([40.0, 50.0, 0.0]));
}

#[test]
fn skybox_model_is_loaded_as_camera_relative_environment() {
    let path = "stage.szs!/map/map/sky.bmd";

    assert!(is_default_preview_model_path(path, false, false, false));
    assert!(!is_camera_bounds_model_path(path));
    assert_eq!(
        preview_render_layer_for_model_path(path),
        PreviewRenderLayer::Sky
    );
    assert_eq!(
        model_loader_flags_for_path(path),
        SMS_DEFAULT_OBJECT_MODEL_LOAD_FLAGS
    );
    assert!(!path_is_sky_model_path("stage.szs!/map/map/reflectsky.bmd"));
}

#[test]
fn shimmer_models_use_the_heatwave_layer_and_indirect_loader_flags() {
    for path in [
        "stage.szs!/mapobj/shimmerlow.bmd",
        "stage.szs!/mapobj/shimmerlowfar.bmd",
        "stage.szs!/mapobj/shimmerhi.bmd",
        "stage.szs!/mapobj/shimmerhifar.bmd",
    ] {
        assert!(path_is_shimmer_model_path(path));
        assert_eq!(
            preview_render_layer_for_model_path(path),
            PreviewRenderLayer::Heatwave
        );
        assert_eq!(model_loader_flags_for_path(path), 0x1101_0000);
    }

    assert!(!path_is_shimmer_model_path(
        "stage.szs!/mapobj/shimmerunrelated.bmd"
    ));
}

#[test]
fn shimmer_draw_transform_keeps_scale_but_cancels_placement_pose() {
    let transform = Transform {
        translation: [10.0, 20.0, 30.0],
        rotation_degrees: [40.0, 50.0, 60.0],
        scale: [1.12, 1.12, 1.0],
    };

    assert_eq!(
        shimmer_preview_transform(transform),
        Transform {
            translation: [0.0; 3],
            rotation_degrees: [0.0; 3],
            scale: transform.scale,
        }
    );
}

#[test]
fn actor_runtime_scale_replaces_authored_non_uniform_preview_scale() {
    let transform = Transform {
        scale: [1.0, 5.0, 1.0],
        ..Transform::default()
    };
    let preview = sms_scene::ActorPreview {
        model_path: "stage.szs!/fixture/default.bmd".to_string(),
        load_flags: 0,
        manager_factory: "FixtureManager".to_string(),
        runtime_uniform_scale: Some(1.0),
    };

    assert_eq!(
        actor_runtime_preview_transform(transform, Some(&preview)).scale,
        [1.0; 3]
    );
    assert_eq!(actor_runtime_preview_transform(transform, None), transform);
}

#[test]
fn reset_fruit_draw_transform_matches_runtime_body_radius_offsets() {
    let registry = reset_fruit_registry();
    let transform = Transform {
        translation: [10.0, 300.0, 30.0],
        rotation_degrees: [0.0; 3],
        scale: [1.0; 3],
    };

    for (resource_name, expected_y) in [
        ("FruitCoconut", 340.0),
        ("FruitPapaya", 340.0),
        ("FruitDurian", 345.0),
        ("FruitPine", 350.0),
        ("RedPepper", 350.0),
        ("FruitBanana", 300.0),
    ] {
        let mut object = SceneObject::new(resource_name, "ResetFruit");
        object.raw_params.insert(
            "stream_string_0".to_string(),
            resource_name.to_string().into(),
        );

        assert_eq!(
            reset_fruit_preview_transform(&object, transform, Some(&registry)).translation[1],
            expected_y
        );
    }
}

#[test]
fn reset_fruit_draw_transform_scales_the_runtime_body_radius() {
    let registry = reset_fruit_registry();
    let mut object = SceneObject::new("pine", "ResetFruit");
    object.raw_params.insert(
        "stream_string_0".to_string(),
        "FruitPine".to_string().into(),
    );
    let transform = Transform {
        translation: [0.0, 100.0, 0.0],
        rotation_degrees: [0.0; 3],
        scale: [2.0; 3],
    };

    assert_eq!(
        reset_fruit_preview_transform(&object, transform, Some(&registry)).translation[1],
        210.0
    );

    object.factory_name = "resetFruit".to_string();
    assert_eq!(
        reset_fruit_preview_transform(&object, transform, Some(&registry)),
        transform
    );
}

#[test]
fn reset_fruit_matrix_correction_includes_xyz_rotation() {
    let registry = reset_fruit_registry();
    let mut banana = SceneObject::new("banana", "ResetFruit");
    banana.set_raw_param("actor_tail_string", "FruitBanana");
    let transform = Transform {
        translation: [0.0, 100.0, 0.0],
        rotation_degrees: [30.0, 40.0, 50.0],
        scale: [1.0, 1.5, 1.0],
    };

    let transformed = reset_fruit_preview_transform(&banana, transform, Some(&registry));
    assert!((transformed.translation[1] - 114.784_58).abs() < 0.000_1);
}

fn reset_fruit_registry() -> ObjectRegistry {
    let entries = [
        ("FruitCoconut", 0x4000_0390, 40, None, None),
        ("FruitPapaya", 0x4000_0391, 40, None, None),
        ("FruitPine", 0x4000_0392, 50, None, Some(10)),
        ("FruitDurian", 0x4000_0393, 45, None, None),
        ("FruitBanana", 0x4000_0394, 50, Some(50), None),
        ("RedPepper", 0x4000_0395, 50, None, None),
    ];
    ObjectRegistry {
        map_obj_resources: entries
            .iter()
            .map(
                |(resource_name, actor_type, _, _, _)| sms_schema::MapObjResourceDefinition {
                    resource_name: (*resource_name).to_string(),
                    actor_type: *actor_type,
                    object_flags: 0,
                    required_manager_name: "fixture map object manager".to_string(),
                    has_hold_dependency: false,
                    has_move_dependency: false,
                    uses_resource_name_model_fallback: true,
                    primary_model: Some(format!("{resource_name}.bmd")),
                    animation_resources: Vec::new(),
                    hold_model_path: None,
                    move_bck_path: None,
                    load_flags: 0x1022_0000,
                    collision_resources: Vec::new(),
                    source_file: "src/MoveBG/MapObjInit.cpp".to_string(),
                },
            )
            .collect(),
        map_obj_ball_transforms: entries
            .iter()
            .map(|(_, actor_type, body_radius, positive, one_minus)| {
                sms_schema::MapObjBallTransformDefinition {
                    actor_type: *actor_type,
                    body_radius: *body_radius,
                    positive_y_axis_subtract: *positive,
                    one_minus_y_axis_subtract: *one_minus,
                    source_file: "src/MoveBG/MapObjBall.cpp".to_string(),
                }
            })
            .collect(),
        ..ObjectRegistry::default()
    }
}

#[test]
fn case_distinct_factories_do_not_inherit_coin_or_npc_behavior() {
    assert!(!is_coin_object(&SceneObject::new("wrong-case", "coinRed")));
    assert_eq!(
        runtime_yaw_degrees_per_frame(&SceneObject::new("wrong-case", "shine")),
        0.0
    );

    let wrong_case_npc = SceneObject::new("wrong-case", "npcMonteMA");
    assert!(starting_joint_animation_candidates(
        &wrong_case_npc,
        "C:/game/dolpic0.szs!/montema/moma_model.bmd",
        None,
    )
    .is_empty());
    assert!(starting_texture_pattern_candidates(
        &wrong_case_npc,
        "C:/game/dolpic0.szs!/montema/moma_model.bmd",
        None,
    )
    .is_empty());
    assert!(actor_runtime_texture_replacements(&wrong_case_npc.factory_name, None).is_empty());
}

#[test]
fn skybox_vertices_track_camera_translation() {
    let vertices = [[1.0, 2.0, 3.0], [-4.0, 5.0, 6.0], [7.0, 8.0, -9.0]];
    let camera = [100.0, 200.0, 300.0];

    assert_eq!(
        preview_triangle_world_vertices(vertices, PreviewRenderLayer::Sky, camera),
        [
            [101.0, 202.0, 303.0],
            [96.0, 205.0, 306.0],
            [107.0, 208.0, 291.0],
        ]
    );
    assert_eq!(
        preview_triangle_world_vertices(vertices, PreviewRenderLayer::Main, camera),
        vertices
    );
}

#[test]
fn viewport_background_is_one_continuous_gradient() {
    let rect = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(800.0, 600.0));
    let mesh = viewport_background_mesh(rect);

    assert_eq!(mesh.vertices.len(), 4);
    assert_eq!(mesh.indices, [0, 1, 2, 0, 2, 3]);
    assert_eq!(mesh.vertices[0].color, mesh.vertices[1].color);
    assert_eq!(mesh.vertices[2].color, mesh.vertices[3].color);
    assert_ne!(mesh.vertices[0].color, mesh.vertices[2].color);
}

#[test]
fn viewport_lines_clip_at_the_near_plane_instead_of_the_crosshair() {
    let camera = CameraFrame {
        position: [0.0, 0.0, 0.0],
        right: [1.0, 0.0, 0.0],
        up: [0.0, 1.0, 0.0],
        forward: [0.0, 0.0, 1.0],
    };

    let clipped =
        clip_world_segment_to_near_plane(camera, [0.0, 0.0, -10.0], [10.0, 0.0, 10.0], 1.0)
            .unwrap();

    assert_vec3_close(clipped[0], [5.5, 0.0, 1.0]);
    assert_vec3_close(clipped[1], [10.0, 0.0, 10.0]);
}

#[test]
fn viewport_lines_fully_behind_the_camera_are_hidden() {
    let camera = CameraFrame {
        position: [0.0, 0.0, 0.0],
        right: [1.0, 0.0, 0.0],
        up: [0.0, 1.0, 0.0],
        forward: [0.0, 0.0, 1.0],
    };

    assert!(
        clip_world_segment_to_near_plane(camera, [0.0, 0.0, -10.0], [10.0, 0.0, -1.0], 1.0,)
            .is_none()
    );
}

#[test]
fn pollution_meshes_are_goop_not_generic_effects() {
    let path = "stage.szs!/map/pollution/pollution00.bmd";

    assert!(is_default_preview_model_path(path, true, true, false));
    assert!(!is_default_preview_model_path(path, true, false, true));
    assert_eq!(
        preview_render_layer_for_model_path(path),
        PreviewRenderLayer::Goop
    );
    assert!(!is_camera_bounds_model_path(path));
}

#[test]
fn archived_pollution_models_require_an_active_ymap_layer() {
    let first = "stage.szs!/map/pollution/pollution00.bmd";
    let second = "stage.szs!/map/pollution/pollution01.bdl";

    assert!(!pollution_layer_model_is_active(first, 0));
    assert!(pollution_layer_model_is_active(first, 1));
    assert!(!pollution_layer_model_is_active(second, 1));
    assert!(pollution_layer_model_is_active(second, 2));
}

#[test]
fn ymap_layer_count_is_read_as_big_endian() {
    assert_eq!(
        pollution_layer_count_from_bytes(&[0, 0, 0, 0, 0, 0, 0, 8]),
        Some(0)
    );
    assert_eq!(pollution_layer_count_from_bytes(&[0, 0, 0, 3]), Some(3));
    assert_eq!(pollution_layer_count_from_bytes(&[0, 0, 0]), None);
}

#[test]
fn mare_lettered_pollution_layers_follow_the_runtime_name_table() {
    assert_eq!(
        pollution_layer_model_index("stage.szs!/map/pollution/pollutionA.bmd"),
        Some(7)
    );
    assert_eq!(
        pollution_layer_model_index("stage.szs!/map/pollution/pollutionB.bmd"),
        Some(8)
    );
}

#[test]
fn named_static_pollution_models_are_not_ymap_layers() {
    let path = "stage.szs!/map/map/mareSeaPollutionS0.bmd";

    assert_eq!(pollution_layer_model_index(path), None);
    assert!(pollution_layer_model_is_active(path, 0));
}

#[test]
fn pollution_bitmap_replaces_the_embedded_authoring_mask_top_down() {
    let mut bmp = vec![0u8; 70];
    bmp[0..2].copy_from_slice(b"BM");
    bmp[10..14].copy_from_slice(&54u32.to_le_bytes());
    bmp[14..18].copy_from_slice(&40u32.to_le_bytes());
    bmp[18..22].copy_from_slice(&2i32.to_le_bytes());
    bmp[22..26].copy_from_slice(&2i32.to_le_bytes());
    bmp[26..28].copy_from_slice(&1u16.to_le_bytes());
    bmp[28..30].copy_from_slice(&8u16.to_le_bytes());
    // BMP rows are bottom-up and padded to four bytes.
    bmp[54..62].copy_from_slice(&[30, 40, 0, 0, 10, 20, 0, 0]);

    let (width, height, rgba) = decode_pollution_bitmap_mask(&bmp).unwrap();

    assert_eq!((width, height), (2, 2));
    assert_eq!(
        rgba,
        vec![10, 10, 10, 10, 20, 20, 20, 20, 30, 30, 30, 30, 40, 40, 40, 40]
    );
}

#[test]
fn pollution_bitmap_replaces_every_material_alias_of_the_dynamic_texture() {
    fn texture(name: &str, width: u16, height: u16, value: u8) -> sms_formats::J3dTexturePreview {
        sms_formats::J3dTexturePreview {
            name: name.to_string(),
            width,
            height,
            format: 1,
            wrap_s: 0,
            wrap_t: 0,
            min_filter: 0,
            mag_filter: 0,
            mipmap_enabled: true,
            do_edge_lod: false,
            bias_clamp: false,
            max_anisotropy: 0,
            min_lod: 0.0,
            max_lod: 1.0,
            lod_bias: 0.0,
            mipmap_count: 2,
            rgba: vec![value; width as usize * height as usize * 4],
            mips: vec![],
        }
    }

    let mut textures = vec![
        texture("DummyPollution256x256_I8", 2, 2, 1),
        texture("TestChoco2", 2, 2, 2),
        texture("DummyPollution256x256_I8", 2, 2, 3),
        texture("DummyPollution256x256_I8", 1, 1, 4),
    ];
    let runtime_mask = vec![9; 2 * 2 * 4];

    replace_pollution_mask_texture_aliases(&mut textures, 2, 2, &runtime_mask);

    assert_eq!(textures[0].rgba, runtime_mask);
    assert_eq!(textures[2].rgba, runtime_mask);
    assert_eq!(textures[0].mipmap_count, 1);
    assert_eq!(textures[2].mipmap_count, 1);
    assert!(!textures[0].mipmap_enabled);
    assert_eq!(textures[0].min_lod, 0.0);
    assert_eq!(textures[0].max_lod, 0.0);
    assert_eq!(textures[0].lod_bias, 0.0);
    assert!(textures[0].mips.is_empty());
    assert!(textures[2].mips.is_empty());
    assert_eq!(textures[1].rgba, vec![2; 2 * 2 * 4]);
    assert_eq!(textures[3].rgba, vec![4; 4]);
}

#[test]
fn pollution_bitmap_rejects_non_i8_or_truncated_inputs() {
    assert_eq!(decode_pollution_bitmap_mask(b"not a bitmap"), None);

    let mut bmp = vec![0u8; 54];
    bmp[0..2].copy_from_slice(b"BM");
    bmp[10..14].copy_from_slice(&54u32.to_le_bytes());
    bmp[18..22].copy_from_slice(&1i32.to_le_bytes());
    bmp[22..26].copy_from_slice(&1i32.to_le_bytes());
    bmp[26..28].copy_from_slice(&1u16.to_le_bytes());
    bmp[28..30].copy_from_slice(&24u16.to_le_bytes());
    assert_eq!(decode_pollution_bitmap_mask(&bmp), None);
}

#[test]
fn every_model_layer_uses_its_same_basename_btk() {
    for (model, animation) in [
        (
            "stage.szs!/map/pollution/pollution00.bmd",
            "stage.szs!/map/pollution/pollution00.btk",
        ),
        ("stage.szs!/map/map/sea.bmd", "stage.szs!/map/map/sea.btk"),
        ("stage.szs!/map/map/sky.bmd", "stage.szs!/map/map/sky.btk"),
        (
            "stage.szs!/mapobj/animated.bdl",
            "stage.szs!/mapobj/animated.btk",
        ),
    ] {
        assert_eq!(
            model_texture_srt_animation_path(model).as_deref(),
            Some(animation)
        );
    }
}

#[test]
fn named_pollution_map_meshes_are_goop() {
    let path = "stage.szs!/map/map/mareseapollutions0.bmd";

    assert!(is_default_preview_model_path(path, true, true, false));
    assert!(!is_default_preview_model_path(path, true, false, true));
    assert_eq!(
        preview_render_layer_for_model_path(path),
        PreviewRenderLayer::Goop
    );
    assert!(!is_camera_bounds_model_path(path));
}

#[test]
fn decomp_owned_map_static_models_require_a_matching_placement() {
    let mut document = test_document(Vec::new());
    document.registry = Some(ObjectRegistry {
        map_static_models: vec![
            sms_schema::MapStaticModelDefinition {
                actor_name: "BiancoRiver".to_string(),
                model_path: Some("/scene/map/map/BiancoRiver.bmd".to_string()),
                collision_path: None,
                load_flags: 0x1021_0000,
                sound_id: None,
                source_file: "src/Map/MapStaticObject.cpp".to_string(),
                stage_bootstrap_created: false,
            },
            sms_schema::MapStaticModelDefinition {
                actor_name: "BiaWaterPollution".to_string(),
                model_path: Some("/scene/map/map/BiaWaterPollution.bmd".to_string()),
                collision_path: None,
                load_flags: 0x1122_0000,
                sound_id: None,
                source_file: "src/Map/MapStaticObject.cpp".to_string(),
                stage_bootstrap_created: false,
            },
            sms_schema::MapStaticModelDefinition {
                actor_name: "sea".to_string(),
                model_path: Some("/scene/map/map/sea.bmd".to_string()),
                collision_path: None,
                load_flags: 0x1022_0000,
                sound_id: None,
                source_file: "src/Map/MapStaticObject.cpp".to_string(),
                stage_bootstrap_created: true,
            },
            sms_schema::MapStaticModelDefinition {
                actor_name: "mareSeaPollutionS34567".to_string(),
                model_path: None,
                collision_path: None,
                load_flags: 0x1021_0000,
                sound_id: None,
                source_file: "src/Map/MapStaticObject.cpp".to_string(),
                stage_bootstrap_created: false,
            },
        ],
        ..ObjectRegistry::default()
    });
    let mut river = SceneObject::new("river", "MapStaticObj");
    river.raw_params.insert(
        "stream_string_0".to_string(),
        "BiancoRiver".to_string().into(),
    );
    document.objects.push(river);

    assert!(map_static_model_is_active(
        &document,
        "stage.szs!/map/map/BiancoRiver.bmd"
    ));
    assert!(!map_static_model_is_active(
        &document,
        "stage.szs!/map/map/BiaWaterPollution.bmd"
    ));
    assert!(map_static_model_is_active(
        &document,
        "stage.szs!/map/map/sea.bmd"
    ));
    assert!(map_static_model_is_active(
        &document,
        "stage.szs!/map/map/map.bmd"
    ));
    assert_eq!(
        map_static_model_loader_flags(&document, "stage.szs!/map/map/sea.bmd"),
        Some(0x1022_0000)
    );

    let mut pollution = SceneObject::new("dirty lake", "MapStaticObj");
    pollution.raw_params.insert(
        "stream_string_0".to_string(),
        "BiaWaterPollution".to_string().into(),
    );
    document.objects.push(pollution);
    assert!(map_static_model_is_active(
        &document,
        "stage.szs!/map/map/BiaWaterPollution.bmd"
    ));
    assert_eq!(
        map_static_model_loader_flags(&document, "stage.szs!/map/map/BiaWaterPollution.bmd"),
        Some(0x1122_0000)
    );

    assert!(map_static_model_is_active(
        &document,
        "stage.szs!/map/map/mareSeaPollutionS34567.bmd"
    ));
    assert_eq!(
        map_static_model_loader_flags(&document, "stage.szs!/map/map/mareSeaPollutionS34567.bmd"),
        None
    );
}

#[test]
fn sea_meshes_are_level_water_layer() {
    let path = "stage.szs!/map/map/sea.bmd";

    assert!(is_default_preview_model_path(path, true, true, false));
    assert!(!is_default_preview_model_path(path, false, true, true));
    assert_eq!(
        preview_render_layer_for_model_path(path),
        PreviewRenderLayer::Water
    );
    assert!(!is_camera_bounds_model_path(path));
}

#[test]
fn source_named_river_models_are_level_water_layers() {
    for path in [
        "stage.szs!/map/map/BiancoRiver.bmd",
        "stage.szs!/map/map/MonteRiver.bmd",
    ] {
        assert_eq!(
            preview_render_layer_for_model_path(path),
            PreviewRenderLayer::Water
        );
    }
}

#[test]
fn map_puddles_are_level_water_layer() {
    let path = "stage.szs!/map/mirror/puddle00.bmd";

    assert!(is_default_preview_model_path(path, true, true, false));
    assert!(!is_default_preview_model_path(path, false, true, true));
    assert_eq!(
        preview_render_layer_for_model_path(path),
        PreviewRenderLayer::Water
    );
    assert!(!is_camera_bounds_model_path(path));
}

#[test]
fn sea_indirect_is_the_default_screen_copy_water_effect() {
    let sea_path = "stage.szs!/map/map/seaindirect.bmd";

    assert!(is_default_preview_model_path(sea_path, true, true, false));
    assert!(!is_default_preview_model_path(sea_path, false, true, true));
    assert_eq!(
        preview_render_layer_for_model_path(sea_path),
        PreviewRenderLayer::IndirectWater
    );
    assert!(!is_camera_bounds_model_path(sea_path));
}

#[test]
fn dormant_puddle_indirect_helpers_stay_hidden_by_default() {
    let path = "stage.szs!/map/mirror/puddle_ind00.bmd";

    assert!(!is_default_preview_model_path(path, true, true, true));
    assert_ne!(
        preview_render_layer_for_model_path(path),
        PreviewRenderLayer::Water
    );
    assert!(!is_camera_bounds_model_path(path));
}

#[test]
fn water_layer_renders_translucent_without_texture_alpha() {
    let preview = preview_for_alpha_texture(false);
    let mut triangle = textured_blended_triangle();
    triangle.render_layer = PreviewRenderLayer::Water;
    triangle.texture_index = None;
    triangle.tex_coords = None;
    triangle.blend_mode = None;

    assert!(!preview_triangle_uses_alpha_test(&preview, &triangle));
    assert!(preview_triangle_is_translucent(&preview, &triangle));
}

#[test]
fn unmasked_goop_layer_renders_as_translucent_overlay() {
    let preview = preview_for_texture_alpha(false, false);
    let mut triangle = textured_blended_triangle();
    triangle.render_layer = PreviewRenderLayer::Goop;
    triangle.blend_mode = None;

    assert!(!preview_triangle_uses_alpha_test(&preview, &triangle));
    assert!(preview_triangle_is_translucent(&preview, &triangle));
}

#[test]
fn masked_goop_layer_uses_alpha_test() {
    let preview = preview_for_texture_alpha(false, false);
    let mut triangle = textured_blended_triangle();
    triangle.render_layer = PreviewRenderLayer::Goop;
    triangle.blend_mode = None;
    triangle.mask_texture_index = Some(0);
    triangle.mask_tex_coords = triangle.tex_coords;

    assert!(preview_triangle_uses_alpha_test(&preview, &triangle));
    assert!(!preview_triangle_is_translucent(&preview, &triangle));
}

#[test]
fn blended_cutout_texture_uses_alpha_test_not_translucency() {
    let preview = preview_for_alpha_texture(false);
    let triangle = textured_blended_triangle();

    assert!(preview_triangle_uses_alpha_test(&preview, &triangle));
    assert!(!preview_triangle_is_translucent(&preview, &triangle));
}

#[test]
fn blended_fractional_alpha_texture_stays_translucent() {
    let preview = preview_for_alpha_texture(true);
    let triangle = textured_blended_triangle();

    assert!(!preview_triangle_uses_alpha_test(&preview, &triangle));
    assert!(preview_triangle_is_translucent(&preview, &triangle));
}

#[test]
fn masked_texture_triangle_uses_alpha_test() {
    let preview = preview_for_texture_alpha(false, false);
    let mut triangle = textured_blended_triangle();
    triangle.blend_mode = None;
    triangle.mask_texture_index = Some(0);
    triangle.mask_tex_coords = triangle.tex_coords;

    assert!(preview_triangle_uses_alpha_test(&preview, &triangle));
    assert!(!preview_triangle_is_translucent(&preview, &triangle));
}

#[test]
fn parses_comma_separated_camera_focus_arg() {
    assert_eq!(
        parse_vec3_arg("-995.2,8353,6493"),
        Some([-995.2, 8353.0, 6493.0])
    );
    assert_eq!(parse_vec3_arg("1,2"), None);
}

#[test]
fn textured_material_tint_does_not_inherit_material_alpha() {
    let tints = preview_texture_tints(
        Some([128, 128, 128, 50]),
        None,
        J3dPreviewCombineMode::TextureModulateMaterial,
        PreviewRenderLayer::Main,
    );

    assert_eq!(
        tints[0],
        egui::Color32::from_rgba_unmultiplied(128, 128, 128, 255)
    );
}

#[test]
fn color32_to_rgba_unpremultiplies_transparent_editor_tints() {
    let rgba = color32_to_rgba(egui::Color32::from_rgba_unmultiplied(144, 217, 255, 50));

    assert!((rgba[0] - 144.0 / 255.0).abs() < 0.01);
    assert!((rgba[1] - 217.0 / 255.0).abs() < 0.01);
    assert!((rgba[2] - 1.0).abs() < 0.01);
    assert!((rgba[3] - 50.0 / 255.0).abs() < 0.01);
}

#[test]
fn software_opaque_pass_outputs_solid_pixels_after_alpha_keep() {
    let mut image = egui::ColorImage::filled([1, 1], egui::Color32::from_rgb(10, 20, 30));
    let mut depth = vec![f32::INFINITY];
    let src = software_output_color_for_pass([0.8, 0.4, 0.2, 0.25], true);

    blend_depth_pixel(&mut image, &mut depth, 0, 42.0, src, true);

    let rgba = color32_to_rgba(image.pixels[0]);
    assert!((rgba[0] - 0.8).abs() < 0.01);
    assert!((rgba[1] - 0.4).abs() < 0.01);
    assert!((rgba[2] - 0.2).abs() < 0.01);
    assert!((rgba[3] - 1.0).abs() < 0.01);
    assert_eq!(depth[0], 42.0);
}

#[test]
fn software_translucent_pass_keeps_fractional_alpha_blending() {
    let mut image = egui::ColorImage::filled([1, 1], egui::Color32::from_rgb(10, 20, 30));
    let mut depth = vec![f32::INFINITY];
    let src = software_output_color_for_pass([1.0, 0.0, 0.0, 0.25], false);

    blend_depth_pixel(&mut image, &mut depth, 0, 42.0, src, false);

    let rgba = color32_to_rgba(image.pixels[0]);
    assert!(rgba[0] > 0.25);
    assert!(rgba[1] < 20.0 / 255.0);
    assert!(rgba[2] < 30.0 / 255.0);
    assert_eq!(depth[0], f32::INFINITY);
}

#[test]
fn textured_material_alpha_does_not_make_opaque_texture_translucent() {
    let preview = preview_for_texture_alpha(false, false);
    let mut triangle = textured_blended_triangle();
    triangle.color = Some([128, 128, 128, 50]);
    triangle.combine_mode = J3dPreviewCombineMode::TextureModulateMaterial;

    assert!(!preview_triangle_uses_alpha_test(&preview, &triangle));
    assert!(!preview_triangle_is_translucent(&preview, &triangle));
}

#[test]
fn retransform_preview_point_preserves_object_local_space() {
    let old_transform = Transform {
        translation: [100.0, 20.0, -40.0],
        rotation_degrees: [0.0, 90.0, 0.0],
        scale: [2.0, 1.0, 1.0],
    };
    let new_transform = Transform {
        translation: [-30.0, 10.0, 80.0],
        rotation_degrees: [0.0, -45.0, 0.0],
        scale: [1.0, 2.0, 1.0],
    };
    let local = [8.0, 4.0, -12.0];
    let old_world = transform_preview_point(local, old_transform);
    let new_world = transform_preview_point(local, new_transform);

    assert_vec3_close(
        retransform_preview_point(old_world, old_transform, new_transform),
        new_world,
    );
}

#[test]
fn transform_preview_normal_ignores_translation_and_normalizes() {
    let transform = Transform {
        translation: [500.0, 0.0, -1000.0],
        rotation_degrees: [0.0, 90.0, 0.0],
        scale: [2.0, 1.0, 1.0],
    };

    assert_vec3_close(
        transform_preview_normal([1.0, 0.0, 0.0], transform),
        [0.0, 0.0, -1.0],
    );
}

#[test]
fn billboard_transform_tracks_instance_center_rotation_and_scale() {
    let billboard = J3dBillboard {
        mode: sms_formats::J3dBillboardMode::Full,
        center: [1.0, 2.0, 3.0],
        axes: [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]],
        offsets: [[2.0, 3.0, 4.0]; 3],
        normals: None,
    };
    let transform = Transform {
        translation: [10.0, 20.0, 30.0],
        rotation_degrees: [0.0, 90.0, 0.0],
        scale: [2.0, 3.0, 4.0],
    };
    let transformed = transform_j3d_billboard(billboard, transform, None).unwrap();

    assert_vec3_close(transformed.center, [22.0, 26.0, 28.0]);
    assert_vec3_close(transformed.offsets[0], [4.0, 9.0, 16.0]);
    assert_vec3_close(transformed.axes[0], [0.0, 0.0, -1.0]);
    assert_vec3_close(transformed.axes[1], [0.0, 1.0, 0.0]);
    assert_vec3_close(transformed.axes[2], [1.0, 0.0, 0.0]);
}

#[test]
fn updating_object_transform_moves_cached_preview_mesh() {
    let old_transform = Transform::default();
    let new_transform = Transform {
        translation: [50.0, 0.0, -25.0],
        ..Transform::default()
    };
    let mut object_model_indices = BTreeMap::new();
    object_model_indices.insert("obj-1".to_string(), 7);
    let mut app = SmsEditorApp {
        model_preview: Some(ModelPreview {
            points: vec![PreviewPoint {
                position: [1.0, 2.0, 3.0],
                model_index: 7,
            }],
            triangles: vec![PreviewTriangle {
                vertices: [[1.0, 0.0, 0.0], [0.0, 2.0, 0.0], [0.0, 0.0, 3.0]],
                normals: Some([[0.0, 1.0, 0.0]; 3]),
                color_channels: [None; 2],
                tex_coord_sets: [None; 8],
                material_index: None,
                packet_index: 0,
                model_index: 7,
                render_layer: PreviewRenderLayer::Main,
                color: None,
                vertex_colors: None,
                combine_mode: J3dPreviewCombineMode::VertexOnly,
                tex_coords: None,
                texture_index: None,
                mask_tex_coords: None,
                mask_texture_index: None,
                cull_mode: None,
                alpha_compare: None,
                blend_mode: None,
                z_mode: None,
                billboard: None,
                particle_type: None,
                particle_pivot: None,
                particle_direction: None,
                particle_color_mode: None,
                particle_environment_color: None,
                particle_extra_texture: None,
            }],
            collision_triangles: Vec::new(),
            collision_file_count: 0,
            collision_surface_count: 0,
            failed_collision_files: 0,
            collision_failures: Vec::new(),
            textures: Vec::new(),
            materials: Vec::new(),
            texture_srt_animations: Vec::new(),
            texture_pattern_animations: Vec::new(),
            material_animation_bindings: Vec::new(),
            pollution_texture_indices: BTreeMap::new(),
            bounds_min: [0.0, 0.0, 0.0],
            bounds_max: [1.0, 2.0, 3.0],
            camera_bounds_min: [0.0, 0.0, 0.0],
            camera_bounds_max: [1.0, 2.0, 3.0],
            loaded_models: 1,
            failed_models: 0,
            model_warnings: Vec::new(),
            model_failures: Vec::new(),
            source_vertices: 3,
            source_triangles: 1,
            source_textures: 0,
            goop_surface_model_indices: BTreeSet::new(),
            instance_model_indices: BTreeMap::new(),
            object_model_indices,
            mirror_actor_positions: BTreeMap::from([(7, old_transform.translation)]),
            mirror_cubes: Vec::new(),
            mirror_model_slots: BTreeMap::new(),
            animated_models: Vec::new(),
            animated_flags: Vec::new(),
            rotating_models: Vec::new(),
            level_transform_models: Vec::new(),
            level_transform_particles: Vec::new(),
            actor_particles: Vec::new(),
            level_transform_duration_frames: 600.0,
            level_transform_particle_end_frames: 600.0,
        }),
        ..SmsEditorApp::default()
    };

    assert!(app
        .update_object_preview_transform("obj-1", old_transform, new_transform)
        .is_some());
    assert_eq!(
        app.model_preview
            .as_ref()
            .and_then(|preview| preview.mirror_actor_positions.get(&7)),
        Some(&new_transform.translation)
    );
    let preview = app.model_preview.as_ref().unwrap();
    let ranges = document_commands::preview_triangle_ranges_for_model(preview, "obj-1");
    assert_eq!(ranges.len(), 1);
    assert_eq!(ranges[0], 0..1);
    assert_vec3_close(preview.points[0].position, [51.0, 2.0, -22.0]);
    assert_vec3_close(preview.triangles[0].vertices[0], [51.0, 0.0, -25.0]);
    assert_vec3_close(preview.triangles[0].vertices[1], [50.0, 2.0, -25.0]);
    assert_vec3_close(preview.triangles[0].vertices[2], [50.0, 0.0, -22.0]);
}

#[test]
fn dirty_state_tracks_saved_object_content() {
    let object = SceneObject::new("obj-1", "coin");
    let mut app = SmsEditorApp {
        document: Some(test_document(vec![object.clone()])),
        saved_objects: vec![object],
        ..SmsEditorApp::default()
    };
    assert!(!app.is_dirty());

    app.mutate_document("Moved object", |document| {
        document.objects[0].transform.translation[0] = 25.0;
    });
    assert!(app.is_dirty());

    app.mutate_document("Restored object", |document| {
        document.objects[0].transform.translation[0] = 0.0;
    });
    assert!(!app.is_dirty());
}

#[test]
fn object_edits_do_not_clear_unsaved_lighting_changes() {
    let object = SceneObject::new("obj-1", "coin");
    let mut app = SmsEditorApp {
        document: Some(test_document(vec![object.clone()])),
        saved_objects: vec![object],
        ..SmsEditorApp::default()
    };

    app.document
        .as_mut()
        .unwrap()
        .lighting
        .ambients
        .push(sms_formats::JDramaAmbient {
            name: Some("Object ambient".to_string()),
            color: [32, 48, 64, 255],
        });
    app.document_dirty = true;

    app.mutate_document("Moved object", |document| {
        document.objects[0].transform.translation[0] = 25.0;
    });
    app.undo();

    assert!(app.is_dirty());
    assert_eq!(app.document.as_ref().unwrap().objects, app.saved_objects);
    assert_ne!(app.document.as_ref().unwrap().lighting, app.saved_lighting);
}

#[test]
fn project_save_uses_the_same_trimmed_project_path_as_project_load() {
    let root = std::env::temp_dir().join(format!(
        "sms-editor-app-save-path-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    let mut app = SmsEditorApp {
        base_root: ".".to_string(),
        project_root: format!("  {}  ", root.display()),
        document: Some(test_document(vec![SceneObject::new("obj-1", "Coin")])),
        ..SmsEditorApp::default()
    };

    assert!(app.save_project());
    assert!(root.join("sms-project.toml").is_file());
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn project_save_succeeds_with_stage_validation_errors() {
    let root = std::env::temp_dir().join(format!(
        "sms-editor-app-invalid-stage-save-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    let mut app = SmsEditorApp {
        base_root: ".".to_string(),
        project_root: root.to_string_lossy().into_owned(),
        document: Some(test_document(vec![
            SceneObject::new("duplicate", "Coin"),
            SceneObject::new("duplicate", "Coin"),
        ])),
        document_dirty: true,
        ..SmsEditorApp::default()
    };

    assert!(app.save_project());
    assert!(!app.document_dirty);
    assert!(app
        .issues
        .iter()
        .any(|issue| issue.severity == ValidationSeverity::Error));
    assert!(app.log.iter().any(|message| {
        message.contains("Saved with") && message.contains("build and launch remain blocked")
    }));
    assert!(root.join("sms-project.toml").is_file());

    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn project_save_is_blocked_when_the_selected_base_differs_from_the_open_document() {
    let root = std::env::temp_dir().join(format!(
        "sms-editor-app-stale-base-save-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    let jp_base = root.join("SunshineJPExtract");
    let us_base = root.join("SunshineUSExtract");
    let project_root = root.join("us-project");
    std::fs::create_dir_all(&jp_base).unwrap();
    std::fs::create_dir_all(&us_base).unwrap();
    let mut document = test_document(Vec::new());
    document.base_root = jp_base;
    let mut app = SmsEditorApp {
        base_root: us_base.to_string_lossy().into_owned(),
        project_root: project_root.to_string_lossy().into_owned(),
        document: Some(document),
        ..SmsEditorApp::default()
    };

    assert!(!app.save_project());
    assert!(!project_root.exists());
    assert!(app
        .log
        .iter()
        .any(|message| message.contains("open stage belongs") && message.contains("blocked")));

    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn project_from_another_base_selects_separate_project_without_blocking_stage_open() {
    let root = std::env::temp_dir().join(format!(
        "sms-editor-app-project-base-mismatch-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    let jp_base = root.join("jp-base");
    let us_base = root.join("us-base");
    let project_root = root.join("sms-editor-project");
    std::fs::create_dir_all(&jp_base).unwrap();
    std::fs::create_dir_all(&us_base).unwrap();

    let mut jp_document = test_document(vec![SceneObject::new("jp-object", "Coin")]);
    jp_document.base_root = jp_base.clone();
    jp_document.save_project_folder(&project_root).unwrap();

    let mut us_document = test_document(vec![SceneObject::new("us-object", "Coin")]);
    us_document.base_root = us_base.clone();
    let selection =
        load_project_for_stage(&mut us_document, &project_root.to_string_lossy()).unwrap();
    let warning = selection
        .warning
        .expect("a mismatched project should select a separate project folder");

    assert!(warning.contains("Project Folder automatically switched"));
    assert_ne!(selection.project_root, project_root.to_string_lossy());
    assert!(selection.project_root.contains("us-base"));
    assert_eq!(us_document.objects[0].id, "us-object");

    us_document
        .save_project_folder(&selection.project_root)
        .unwrap();
    let mut reopened_us = test_document(vec![SceneObject::new("base-us-object", "Coin")]);
    reopened_us.base_root = us_base;
    assert!(reopened_us
        .load_project_folder(&selection.project_root)
        .unwrap());
    assert_eq!(reopened_us.objects[0].id, "us-object");

    let mut reopened_jp = test_document(vec![SceneObject::new("base-jp-object", "Coin")]);
    reopened_jp.base_root = jp_base;
    let jp_selection = load_project_for_stage(&mut reopened_jp, &selection.project_root).unwrap();
    assert_eq!(PathBuf::from(jp_selection.project_root), project_root);
    assert_eq!(reopened_jp.objects[0].id, "jp-object");

    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn project_folder_inside_the_base_is_moved_to_a_safe_sibling() {
    let root = std::env::temp_dir().join(format!(
        "sms-editor-app-project-overlap-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    let base_root = root.join("SunshineUSExtract");
    std::fs::create_dir_all(&base_root).unwrap();
    let mut document = test_document(Vec::new());
    document.base_root = base_root.clone();

    let selection = load_project_for_stage(&mut document, &base_root.to_string_lossy()).unwrap();

    assert_eq!(
        PathBuf::from(&selection.project_root),
        root.join("SunshineUSExtract-graffito-editor-project")
    );
    assert!(selection
        .warning
        .is_some_and(|warning| warning.contains("must be outside")));

    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn stage_build_requires_a_saved_sms_project() {
    let mut app = SmsEditorApp {
        document: Some(test_document(Vec::new())),
        ..SmsEditorApp::default()
    };

    app.build_game();

    assert!(app.background_receiver.is_none());
    assert!(app
        .log
        .iter()
        .any(|message| message.contains("requires a saved .sms project")));
}

#[test]
fn completed_stage_build_reports_the_managed_game_relative_output() {
    let (sender, receiver) = std::sync::mpsc::channel();
    sender
        .send(BackgroundResult::Build(Ok(
            managed_build::ManagedGameBuildOutcome {
                run: managed_build::ManagedRunMirrorOutcome {
                    build_root: PathBuf::from("project.smsbuild"),
                    run_root: PathBuf::from("project.smsbuild/run-root"),
                    run_main_dol: PathBuf::from("project.smsbuild/run-root/sys/main.dol"),
                    source_relative_path: PathBuf::from("files/data/scene/dolpic0.szs"),
                    stage_output_path: PathBuf::from(
                        "project.smsbuild/run-root/files/data/scene/dolpic0.szs",
                    ),
                    stage_size_bytes: 1234,
                    stage_replaced: false,
                    copied_files: 3,
                    reused_files: 0,
                    removed_entries: 0,
                },
            },
        )))
        .unwrap();
    let mut app = SmsEditorApp {
        background_receiver: Some(receiver),
        background_label: Some("Building managed game".to_string()),
        ..SmsEditorApp::default()
    };

    app.poll_background_task(&egui::Context::default(), None);

    assert!(app.background_receiver.is_none());
    assert!(app.background_label.is_none());
    assert!(app.log.iter().any(|message| {
        message.contains("1234-byte")
            && message.contains("project.smsbuild/run-root/files/data/scene/dolpic0.szs")
    }));
    assert!(app.log.iter().any(|message| {
        message.contains("Managed game directory") && message.contains("project.smsbuild/run-root")
    }));
    assert!(app
        .log
        .iter()
        .any(|message| message.contains("extracted base game was not modified")));
}

#[test]
fn completed_managed_launch_reports_the_resolved_direct_boot_target() {
    let (sender, receiver) = std::sync::mpsc::channel();
    sender
        .send(BackgroundResult::BuildAndRun {
            mode: DolphinLaunchMode::External,
            result: Ok(managed_build::ManagedGameLaunchOutcome {
                run: managed_build::ManagedRunMirrorOutcome {
                    build_root: PathBuf::from("project.smsbuild"),
                    run_root: PathBuf::from("project.smsbuild/run-root"),
                    run_main_dol: PathBuf::from("project.smsbuild/run-root/sys/main.dol"),
                    source_relative_path: PathBuf::from("files/data/scene/pinnaBeach4.szs"),
                    stage_output_path: PathBuf::from(
                        "project.smsbuild/run-root/files/data/scene/pinnaBeach4.szs",
                    ),
                    stage_size_bytes: 4321,
                    stage_replaced: true,
                    copied_files: 1,
                    reused_files: 2,
                    removed_entries: 0,
                },
                direct_boot: managed_build::ManagedDirectBootOutcome {
                    launch_dol: PathBuf::from("project.smsbuild/run-root/sys/main.dol"),
                    target: direct_boot::RuntimeStageTarget {
                        area_index: 5,
                        scenario_index: 4,
                        archive_name: "pinnaBeach4.arc".to_string(),
                    },
                    matching_contexts: 4,
                    size_bytes: 9876,
                    reused: false,
                    logo_bypass_address: 0x800F_9DF4,
                    hook_address: 0x800F_9B4C,
                    movie_hook_address: 0x800F_A000,
                    stub_address: 0x8042_0000,
                },
            }),
        })
        .unwrap();
    let mut app = SmsEditorApp {
        background_receiver: Some(receiver),
        background_label: Some("Preparing and launching current scene".to_string()),
        ..SmsEditorApp::default()
    };

    app.poll_background_task(&egui::Context::default(), None);

    assert!(app.background_receiver.is_none());
    assert!(app.background_label.is_none());
    assert!(app.log.iter().any(|message| {
        message.contains("9876-byte")
            && message.contains("pinnaBeach4.arc")
            && message.contains("runtime area 5, scenario 4")
            && message.contains("logo bypass 0x800F9DF4")
    }));
    assert!(app
        .log
        .iter()
        .any(|message| message.contains("4 runtime contexts")));
    assert!(app
        .log
        .iter()
        .any(|message| message.contains("Dolphin executable is not configured")));
}

#[test]
fn managed_dolphin_exec_keeps_the_extracted_directory_mount_path() {
    let run_root = std::path::Path::new("project.smsbuild/run-root");
    assert!(document_commands::managed_dolphin_exec_is_directory_main(
        run_root,
        std::path::Path::new("project.smsbuild/run-root/sys/main.dol")
    ));
    assert!(!document_commands::managed_dolphin_exec_is_directory_main(
        run_root,
        std::path::Path::new("project.smsbuild/run-root/sys/direct-boot.dol")
    ));
}

#[test]
fn blank_dolphin_user_directory_uses_dolphins_normal_profile() {
    let mut command = Command::new("Dolphin");

    let configured = SmsEditorApp::configure_dolphin_user_directory(&mut command, "  ");

    assert_eq!(configured, None);
    assert!(command.get_args().next().is_none());
}

#[test]
fn configured_dolphin_user_directory_is_forwarded_to_dolphin() {
    let mut command = Command::new("Dolphin");

    let configured = SmsEditorApp::configure_dolphin_user_directory(
        &mut command,
        r"C:\DolphinProfiles\SMS-Modding",
    );

    assert_eq!(
        configured,
        Some(PathBuf::from(r"C:\DolphinProfiles\SMS-Modding"))
    );
    assert_eq!(
        command.get_args().collect::<Vec<_>>(),
        [
            std::ffi::OsStr::new("-u"),
            std::ffi::OsStr::new(r"C:\DolphinProfiles\SMS-Modding")
        ]
    );
}

#[test]
fn play_in_editor_keeps_input_active_and_accelerates_original_disc_loads() {
    let mut command = Command::new("Dolphin");

    SmsEditorApp::configure_play_in_editor_runtime(&mut command);

    assert_eq!(
        command.get_args().collect::<Vec<_>>(),
        [
            std::ffi::OsStr::new("-C"),
            std::ffi::OsStr::new("Dolphin.Interface.PauseOnFocusLost=False"),
            std::ffi::OsStr::new("-C"),
            std::ffi::OsStr::new("Dolphin.Input.BackgroundInput=True"),
            std::ffi::OsStr::new("-C"),
            std::ffi::OsStr::new("Dolphin.Core.FastDiscSpeed=True"),
        ]
    );
}

#[test]
fn dolphin_boot_enables_the_targeted_goop_mod() {
    let mut command = Command::new("Dolphin");

    SmsEditorApp::configure_sms_graphics(&mut command, true);

    assert_eq!(
        command.get_args().collect::<Vec<_>>(),
        [
            std::ffi::OsStr::new("-C"),
            std::ffi::OsStr::new("GFX.Settings.EnableMods=True"),
        ]
    );
}

#[test]
fn dolphin_boot_falls_back_when_the_targeted_goop_mod_is_unavailable() {
    let mut command = Command::new("Dolphin");

    SmsEditorApp::configure_sms_graphics(&mut command, false);

    assert_eq!(
        command.get_args().collect::<Vec<_>>(),
        [
            std::ffi::OsStr::new("-C"),
            std::ffi::OsStr::new("GFX.Hacks.EFBScaledCopy=False"),
        ]
    );
}

#[test]
fn managed_build_cancel_state_logs_once_and_clears_with_the_result() {
    let cancel = Arc::new(AtomicBool::new(false));
    let (sender, receiver) = std::sync::mpsc::channel();
    let mut app = SmsEditorApp {
        background_receiver: Some(receiver),
        background_label: Some("Building managed game".to_string()),
        active_build_cancel: Some(Arc::clone(&cancel)),
        ..SmsEditorApp::default()
    };

    assert!(app.background_label.is_some());
    assert!(app.active_build_cancel.is_some());
    app.cancel_active_build();
    app.cancel_active_build();

    assert!(cancel.load(Ordering::Acquire));
    assert_eq!(
        app.log
            .iter()
            .filter(|message| message.starts_with("Cancelling managed game build"))
            .count(),
        1
    );

    sender
        .send(BackgroundResult::Build(Err(format!(
            "{}; test cancellation",
            managed_build::MANAGED_BUILD_CANCELLED
        ))))
        .unwrap();
    app.poll_background_task(&egui::Context::default(), None);

    assert!(app.background_receiver.is_none());
    assert!(app.background_label.is_none());
    assert!(app.active_build_cancel.is_none());
    assert!(app
        .log
        .iter()
        .any(|message| message.starts_with("Game build cancelled:")));
}

#[test]
fn completed_stage_load_is_discarded_when_the_project_path_changed() {
    let document = test_document(Vec::new());
    let scene = RenderScene::from_document(&document);
    let mut app = SmsEditorApp {
        base_root: "base-root".to_string(),
        project_root: "project-b".to_string(),
        stage_id: "dolpic0".to_string(),
        ..SmsEditorApp::default()
    };
    let loaded = LoadedStage {
        base_root: "base-root".to_string(),
        requested_project_root: "project-a".to_string(),
        project_root: "project-a".to_string(),
        has_scene_index: true,
        archives: Vec::new(),
        registry: None,
        schema_warning: None,
        object_authoring_catalog_key: None,
        object_authoring_catalog: Default::default(),
        object_authoring_catalog_warnings: Default::default(),
        project_warning: None,
        document,
        scene,
        preview: None,
        scene_labels: BTreeMap::new(),
        scene_label_warning: None,
        retail_skyboxes: Vec::new(),
        skybox_warnings: Vec::new(),
        retail_music: Vec::new(),
        retail_sounds: Vec::new(),
        retail_dialogue_voices: Vec::new(),
        retail_stage_audio: Vec::new(),
        music_warning: None,
    };

    app.apply_loaded_stage(loaded);

    assert!(app.document.is_none());
    assert!(app
        .log
        .iter()
        .any(|message| message.contains("superseded project root")));
}

#[test]
fn completed_stage_load_adopts_the_resolved_project_folder() {
    let document = test_document(Vec::new());
    let scene = RenderScene::from_document(&document);
    let mut app = SmsEditorApp {
        base_root: "base-root".to_string(),
        project_root: "sms-editor-project".to_string(),
        stage_id: "dolpic0".to_string(),
        ..SmsEditorApp::default()
    };
    let loaded = LoadedStage {
        base_root: "base-root".to_string(),
        requested_project_root: "sms-editor-project".to_string(),
        project_root: "sms-editor-project-SunshineUSExtract".to_string(),
        has_scene_index: true,
        archives: Vec::new(),
        registry: None,
        schema_warning: None,
        object_authoring_catalog_key: None,
        object_authoring_catalog: Default::default(),
        object_authoring_catalog_warnings: Default::default(),
        project_warning: Some("Project Folder automatically switched.".to_string()),
        document,
        scene,
        preview: None,
        scene_labels: BTreeMap::new(),
        scene_label_warning: None,
        retail_skyboxes: Vec::new(),
        skybox_warnings: Vec::new(),
        retail_music: Vec::new(),
        retail_sounds: Vec::new(),
        retail_dialogue_voices: Vec::new(),
        retail_stage_audio: Vec::new(),
        music_warning: None,
    };

    app.apply_loaded_stage(loaded);

    assert_eq!(app.project_root, "sms-editor-project-SunshineUSExtract");
    assert!(app
        .log
        .iter()
        .any(|message| message.contains("automatically switched")));
}

#[test]
fn completed_stage_load_is_discarded_when_the_selected_stage_changed() {
    let document = test_document(Vec::new());
    let scene = RenderScene::from_document(&document);
    let mut app = SmsEditorApp {
        base_root: "base-root".to_string(),
        project_root: "project".to_string(),
        stage_id: "bianco0".to_string(),
        ..SmsEditorApp::default()
    };
    let loaded = LoadedStage {
        base_root: "base-root".to_string(),
        requested_project_root: "project".to_string(),
        project_root: "project".to_string(),
        has_scene_index: true,
        archives: Vec::new(),
        registry: None,
        schema_warning: None,
        object_authoring_catalog_key: None,
        object_authoring_catalog: Default::default(),
        object_authoring_catalog_warnings: Default::default(),
        project_warning: None,
        document,
        scene,
        preview: None,
        scene_labels: BTreeMap::new(),
        scene_label_warning: None,
        retail_skyboxes: Vec::new(),
        skybox_warnings: Vec::new(),
        retail_music: Vec::new(),
        retail_sounds: Vec::new(),
        retail_dialogue_voices: Vec::new(),
        retail_stage_audio: Vec::new(),
        music_warning: None,
    };

    app.apply_loaded_stage(loaded);

    assert!(app.document.is_none());
    assert!(app
        .log
        .iter()
        .any(|message| message.contains("superseded stage")));
}

#[test]
fn missing_decomp_source_uses_the_shipped_schema_bundle() {
    let root = tempfile::tempdir().unwrap();
    let missing_decomp = root.path().join("missing-decomp");

    let selection = generate_editor_schema(&missing_decomp).unwrap();

    assert!(!selection.registry.objects.is_empty());
    assert!(!selection.registry.npc_actors.is_empty());
    assert!(!selection.registry.enemy_actors.is_empty());
    assert!(!selection.registry.bgm_wave_scenes.is_empty());
    assert!(!selection.registry.stage_audio_areas.is_empty());
    assert_eq!(
        selection.registry.dialogue_voices.len(),
        sms_formats::SMS_TALK_SOUND_LIMIT
    );
    let music = index_retail_music(&selection.registry, root.path(), &BTreeMap::new()).unwrap();
    assert!(
        music.len() >= 40,
        "bundled schema exposed only {} music choices",
        music.len()
    );
    let voices = index_retail_dialogue_voices(&selection.registry, &[]).unwrap();
    assert_eq!(voices.len(), sms_formats::SMS_TALK_SOUND_LIMIT);
    assert!(selection.status.contains("bundled object entries"));
    assert!(selection.status.contains("source was unavailable"));
}

#[test]
fn object_authoring_catalog_cache_identity_tracks_retail_inventory_and_registry() {
    let root = tempfile::tempdir().unwrap();
    let archive_path = root.path().join("files/data/scene/dolpic0.szs");
    std::fs::create_dir_all(archive_path.parent().unwrap()).unwrap();
    std::fs::write(&archive_path, [0_u8]).unwrap();
    let retail_archive = SceneArchiveInfo {
        stage_id: "dolpic0".to_string(),
        group: "dolpic".to_string(),
        path: archive_path,
        relative_path: PathBuf::from("files/data/scene/dolpic0.szs"),
        size_bytes: 1,
    };
    let registry = ObjectRegistry::default();
    let original_key = object_authoring_catalog_cache_key(
        root.path(),
        std::slice::from_ref(&retail_archive),
        &registry,
    );

    let authored_archive = SceneArchiveInfo {
        stage_id: "custom0".to_string(),
        group: "custom".to_string(),
        path: root.path().join("files/data/scene/custom0.szs"),
        relative_path: PathBuf::from("files/data/scene/custom0.szs"),
        size_bytes: 0,
    };
    let with_authored_key = object_authoring_catalog_cache_key(
        root.path(),
        &[retail_archive.clone(), authored_archive],
        &registry,
    );
    assert_eq!(original_key, with_authored_key);

    let mut changed_archive = retail_archive.clone();
    changed_archive.size_bytes = 2;
    let changed_archive_key =
        object_authoring_catalog_cache_key(root.path(), &[changed_archive], &registry);
    assert_ne!(original_key, changed_archive_key);

    let changed_registry = ObjectRegistry {
        moving_collision_vertex_limit: Some(1),
        ..ObjectRegistry::default()
    };
    let changed_registry_key = object_authoring_catalog_cache_key(
        root.path(),
        std::slice::from_ref(&retail_archive),
        &changed_registry,
    );
    assert_ne!(original_key, changed_registry_key);

    let preview_registry = ObjectRegistry {
        object_previews: vec![sms_schema::ObjectPreviewDefinition {
            factory_name: "Mario".to_string(),
            runtime_archive_path: "/data/mario.arc".to_string(),
            model_path: "/mario/bmd/ma_mdl1.bmd".to_string(),
            load_flags: 0x1010_0000,
            idle_bck_path: "/mario/bck/ma_wait.bck".to_string(),
            idle_btp_path: Some("/mario/btp/ma_wink_tx.btp".to_string()),
            idle_playback_rate_numerator: 1,
            idle_playback_rate_denominator: 2,
            hidden_shape_indices: vec![10],
            tev_k_color_alpha_overrides: vec![sms_schema::ObjectPreviewTevKColorAlphaOverride {
                register: 0,
                alpha: 0,
            }],
            source_files: vec!["src/Player/MarioDraw.cpp".to_string()],
        }],
        ..ObjectRegistry::default()
    };
    let preview_registry_key =
        object_authoring_catalog_cache_key(root.path(), &[retail_archive], &preview_registry);
    assert_ne!(original_key, preview_registry_key);
}

#[test]
fn completed_base_game_index_is_reused_for_later_level_loads_only_on_the_same_base() {
    let base = tempfile::tempdir().unwrap();
    let other_base = tempfile::tempdir().unwrap();
    let archive = SceneArchiveInfo {
        stage_id: "dolpic0".to_string(),
        group: "Delfino Plaza".to_string(),
        relative_path: PathBuf::from("files/data/scene/dolpic0.szs"),
        path: base.path().join("files/data/scene/dolpic0.szs"),
        size_bytes: 42,
    };
    let app = SmsEditorApp {
        last_scanned_base_root: base.path().to_string_lossy().into_owned(),
        scene_archives: vec![archive.clone()],
        ..SmsEditorApp::default()
    };

    let reused = app
        .reusable_scene_scan(base.path())
        .expect("same-base level switches should reuse the completed index");
    assert_eq!(reused.archives, [archive]);
    assert!(app.reusable_scene_scan(other_base.path()).is_none());
}

#[test]
fn object_authoring_catalog_cache_reuses_the_immutable_payload() {
    let root = tempfile::tempdir().unwrap();
    let registry = ObjectRegistry::default();
    let key = object_authoring_catalog_cache_key(root.path(), &[], &registry);
    let catalog = Arc::new(ObjectAuthoringCatalog::default());
    let warnings = Arc::new(Vec::new());
    let app = SmsEditorApp {
        object_authoring_catalog_cache_key: Some(key),
        object_authoring_catalog: Arc::clone(&catalog),
        object_authoring_catalog_warnings: Arc::clone(&warnings),
        ..SmsEditorApp::default()
    };

    let reused = app
        .reusable_object_authoring_catalog_cache(root.path(), Some(&registry))
        .unwrap();
    assert!(Arc::ptr_eq(&catalog, &reused.catalog));
    assert!(Arc::ptr_eq(&warnings, &reused.warnings));

    let changed_registry = ObjectRegistry {
        moving_collision_vertex_limit: Some(1),
        ..ObjectRegistry::default()
    };
    assert!(app
        .reusable_object_authoring_catalog_cache(root.path(), Some(&changed_registry))
        .is_none());
}

#[test]
fn schema_refresh_updates_derived_preview_metadata_without_marking_the_document_dirty() {
    let root = tempfile::tempdir().unwrap();
    let archive_path = root.path().join("files/data/mario.szs");
    std::fs::create_dir_all(archive_path.parent().unwrap()).unwrap();
    let mut builder = sms_formats::RarcBuilder::new(b"mario".to_vec()).unwrap();
    for (path, bytes) in [
        (b"bmd/ma_mdl1.bmd".as_slice(), b"body".as_slice()),
        (b"bck/ma_wait.bck".as_slice(), b"wait".as_slice()),
        (b"btp/ma_wink_tx.btp".as_slice(), b"wink".as_slice()),
    ] {
        builder.insert_file(path, bytes.to_vec()).unwrap();
    }
    std::fs::write(&archive_path, builder.build().unwrap().to_bytes().unwrap()).unwrap();

    let object = SceneObject::new("mario", "Mario");
    let mut document = test_document(vec![object.clone()]);
    document.base_root = root.path().to_path_buf();
    let mut app = SmsEditorApp {
        base_root: root.path().display().to_string(),
        document: Some(document),
        saved_objects: vec![object],
        ..SmsEditorApp::default()
    };
    let registry = ObjectRegistry {
        object_previews: vec![sms_schema::ObjectPreviewDefinition {
            factory_name: "Mario".to_string(),
            runtime_archive_path: "/data/mario.arc".to_string(),
            model_path: "/mario/bmd/ma_mdl1.bmd".to_string(),
            load_flags: 0x1010_0000,
            idle_bck_path: "/mario/bck/ma_wait.bck".to_string(),
            idle_btp_path: Some("/mario/btp/ma_wink_tx.btp".to_string()),
            idle_playback_rate_numerator: 1,
            idle_playback_rate_denominator: 2,
            hidden_shape_indices: vec![10],
            tev_k_color_alpha_overrides: vec![sms_schema::ObjectPreviewTevKColorAlphaOverride {
                register: 0,
                alpha: 0,
            }],
            source_files: vec!["src/Player/MarioDraw.cpp".to_string()],
        }],
        ..ObjectRegistry::default()
    };
    let (sender, receiver) = std::sync::mpsc::channel();
    sender
        .send(BackgroundResult::Schema(Box::new(Ok(LoadedSchema {
            registry,
            object_authoring_catalog_cache: None,
            status: "Loaded test schema.".to_string(),
        }))))
        .unwrap();
    app.background_receiver = Some(receiver);

    app.poll_background_task(&egui::Context::default(), None);

    assert!(!app.is_dirty());
    assert_eq!(app.document.as_ref().unwrap().objects, app.saved_objects);
    let hint = app.saved_objects[0]
        .asset_hints
        .iter()
        .find(|hint| hint.role == AssetRole::InferredPreviewModel)
        .expect("saved Mario exact preview hint");
    assert!(hint
        .path
        .replace('\\', "/")
        .ends_with("mario.szs!/bmd/ma_mdl1.bmd"));
}

#[test]
fn transform_transaction_creates_one_undo_entry() {
    let object = SceneObject::new("obj-1", "coin");
    let mut app = SmsEditorApp {
        document: Some(test_document(vec![object.clone()])),
        saved_objects: vec![object],
        selected_object_id: Some("obj-1".to_string()),
        ..SmsEditorApp::default()
    };

    app.begin_undo_transaction();
    let mut transform = app.selected_object().unwrap().transform;
    transform.translation[0] = 10.0;
    app.update_selected_transform(transform);
    transform.translation[0] = 20.0;
    app.update_selected_transform(transform);
    assert!(
        app.document.as_ref().unwrap().changed_files.is_empty(),
        "transaction deltas must not serialize the full editor overlay"
    );
    app.commit_undo_transaction("Moved object");

    assert_eq!(app.undo_stack.len(), 1);
    assert!(matches!(
        app.undo_stack.back().unwrap().deltas.as_slice(),
        [ObjectDelta::Update { before, after }]
            if before.transform.translation[0] == 0.0
                && after.transform.translation[0] == 20.0
    ));
    assert!(
        app.document.as_ref().unwrap().changed_files.is_empty(),
        "committing an edit must defer the full overlay until save"
    );
    app.undo();
    assert_eq!(app.selected_object().unwrap().transform.translation[0], 0.0);
    app.redo();
    assert_eq!(
        app.selected_object().unwrap().transform.translation[0],
        20.0
    );
    let document = app.document.as_mut().unwrap();
    document.queue_editor_overlay_change().unwrap();
    assert_eq!(document.changed_files.len(), 1);
}

fn test_document(objects: Vec<SceneObject>) -> StageDocument {
    StageDocument {
        stage_id: "dolpic0".to_string(),
        base_root: PathBuf::from("."),
        assets: Vec::new(),
        objects,
        changed_files: BTreeMap::new(),
        stage_archive: None,
        stage_archive_source_path: None,
        archive_edits: sms_scene::StageArchiveEdits::default(),
        registry: None,
        route_authoring: None,
        goop_authoring: None,
        dialogue_authoring: None,
        dialogue_library: Default::default(),
        load_issues: Vec::new(),
        lighting: Default::default(),
        death_barrier: None,
        actor_previews: BTreeMap::new(),
        loaded_project: None,
    }
}

fn synthetic_particle_effect() -> Vec<u8> {
    let texture = sms_formats::BtiFile {
        allocation_size: 0x40,
        format: 0,
        transparency: 0,
        width: 8,
        height: 8,
        wrap_s: 1,
        wrap_t: 1,
        palette_enabled: 0,
        palette_format: 0,
        palette_entries: Vec::new(),
        palette_offset: 0,
        mipmap_enabled: 0,
        edge_lod: 0,
        bias_clamp: 0,
        max_anisotropy: 0,
        min_filter: 1,
        mag_filter: 1,
        min_lod: 0,
        max_lod: 0,
        mipmap_count: 1,
        reserved_19: 0,
        lod_bias: 0,
        image_offset: 0x20,
        encoded_mip_levels: vec![vec![0xff; 32]],
    }
    .encode()
    .expect("encode synthetic particle texture");
    let emitter_size = 0x90usize;
    let shape_size = 0x98usize;
    let texture_size = 0x20 + texture.len();
    let declared_size = 0x20 + emitter_size + shape_size + texture_size;
    let mut bytes = vec![0; declared_size];
    bytes[..8].copy_from_slice(b"JEFFjpa1");
    bytes[8..12].copy_from_slice(&(declared_size as u32).to_be_bytes());
    bytes[12..16].copy_from_slice(&3u32.to_be_bytes());

    let emitter = 0x20;
    bytes[emitter..emitter + 4].copy_from_slice(b"BEM1");
    bytes[emitter + 4..emitter + 8].copy_from_slice(&(emitter_size as u32).to_be_bytes());
    bytes[emitter + 0x18..emitter + 0x1c].copy_from_slice(&5.0f32.to_be_bytes());
    bytes[emitter + 0x44..emitter + 0x46].copy_from_slice(&30u16.to_be_bytes());

    let shape = emitter + emitter_size;
    bytes[shape..shape + 4].copy_from_slice(b"BSP1");
    bytes[shape + 4..shape + 8].copy_from_slice(&(shape_size as u32).to_be_bytes());
    bytes[shape + 0x18..shape + 0x1c].copy_from_slice(&1.0f32.to_be_bytes());
    bytes[shape + 0x1c..shape + 0x20].copy_from_slice(&1.0f32.to_be_bytes());
    bytes[shape + 0x64..shape + 0x68].copy_from_slice(&[255; 4]);

    let texture_block = shape + shape_size;
    bytes[texture_block..texture_block + 4].copy_from_slice(b"TEX1");
    bytes[texture_block + 4..texture_block + 8]
        .copy_from_slice(&(texture_size as u32).to_be_bytes());
    bytes[texture_block + 0x0c..texture_block + 0x12].copy_from_slice(b"funsui");
    bytes[texture_block + 0x20..].copy_from_slice(&texture);
    bytes
}

#[test]
fn particle_only_actors_render_without_placeholder_models() {
    let unique = format!(
        "graffito-particle-only-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("system clock")
            .as_nanos()
    );
    let root = std::env::temp_dir().join(unique);
    let particle_path = root
        .join("scene")
        .join("map")
        .join("map")
        .join("ms_bia_funsui.jpa");
    std::fs::create_dir_all(particle_path.parent().expect("particle parent"))
        .expect("create particle fixture folder");
    std::fs::write(&particle_path, synthetic_particle_effect()).expect("write particle fixture");

    let mut object = SceneObject::new("bianco-fountain", "EffectBiancoFunsui");
    object.transform.translation = [100.0, 200.0, 300.0];
    let mut document = test_document(vec![object.clone()]);
    document.base_root = root.clone();
    document.assets.push(StageAsset {
        path: particle_path,
        kind: StageAssetKind::Particle,
    });
    document.registry = Some(ObjectRegistry {
        objects: vec![ObjectDefinition {
            factory_name: "EffectBiancoFunsui".to_string(),
            class_name: "TEffectBiancoFunsui".to_string(),
            category: "Enemy".to_string(),
            source: sms_schema::SchemaSource::MarNameRefGen,
            display_name: None,
            preview_model: None,
            hidden: false,
            unsafe_to_edit: false,
        }],
        particle_resources: vec![sms_schema::ParticleResourceDefinition {
            effect_id: 0x1A9,
            path: "/scene/map/map/ms_bia_funsui.jpa".to_string(),
            source_file: "src/Enemy/effectObj.cpp".to_string(),
        }],
        actor_particle_bindings: vec![sms_schema::ActorParticleBinding {
            class_name: "TEffectBiancoFunsui".to_string(),
            effect_id: 0x1A9,
            target: ParticleBindingTarget::ActorOrigin,
            source_file: "src/Enemy/effectObj.cpp".to_string(),
        }],
        ..ObjectRegistry::default()
    });

    let preview = SmsEditorApp::build_model_preview(
        &document,
        PreviewVisibility {
            environment: false,
            goop: false,
            effects: true,
        },
    )
    .expect("build particle-only actor preview");
    let model_index = preview.object_model_indices[&object.id];

    assert_eq!(preview.loaded_models, 1);
    assert_eq!(preview.actor_particles.len(), 1);
    assert_eq!(preview.actor_particles[0].model_index, Some(model_index));
    assert_eq!(
        preview.actor_particles[0].bind_transform.translation,
        object.transform.translation
    );
    assert!(preview.triangles.iter().any(|triangle| {
        triangle.model_index == model_index && triangle.render_layer == PreviewRenderLayer::Particle
    }));

    std::fs::remove_dir_all(root).expect("remove particle fixture folder");
}

#[test]
fn model_preview_failures_are_deduplicated_and_detail_bounded() {
    let mut failed_assets = BTreeSet::new();
    let mut failures = Vec::new();
    for index in 0..(MAX_MODEL_FAILURE_DETAILS + 3) {
        record_model_preview_failure(
            &mut failed_assets,
            &mut failures,
            &format!("stage.szs!/map/model-{index}.bmd"),
            format!("parse error {index}"),
        );
    }
    record_model_preview_failure(
        &mut failed_assets,
        &mut failures,
        "STAGE.SZS!/MAP/MODEL-0.BMD",
        "duplicate error".to_string(),
    );

    assert_eq!(failed_assets.len(), MAX_MODEL_FAILURE_DETAILS + 3);
    assert_eq!(failures.len(), MAX_MODEL_FAILURE_DETAILS);
    assert_eq!(failures[0].error, "parse error 0");
}

#[test]
fn a_failure_only_preview_retains_actionable_asset_details() {
    let mut document = test_document(Vec::new());
    document.assets.push(StageAsset {
        path: PathBuf::from("definitely-missing-preview-model.bmd"),
        kind: StageAssetKind::Model,
    });

    let preview = SmsEditorApp::build_model_preview(
        &document,
        PreviewVisibility {
            environment: true,
            goop: true,
            effects: true,
        },
    )
    .expect("failure details survive without decoded geometry");

    assert_eq!(preview.failed_models, 1);
    assert_eq!(preview.model_failures.len(), 1);
    assert!(preview.model_failures[0]
        .asset_path
        .contains("definitely-missing-preview-model.bmd"));
    assert!(preview.model_failures[0].error.contains("read asset"));
}

#[test]
fn renderer_validation_names_only_framebuffer_dependent_logic_materials() {
    let mut issues = Vec::new();
    append_gpu_blend_validation_issue(&mut issues, 7, "logic-xor", 2, 6);
    append_gpu_blend_validation_issue(&mut issues, 8, "logic-copy", 2, 3);

    assert_eq!(issues.len(), 1);
    assert_eq!(issues[0].code, "renderer-gx-logic-op-unsupported-7");
    assert!(issues[0].message.contains("logic-xor"));
    assert!(issues[0].message.contains("operation 6"));
}

#[test]
fn renderer_validation_reports_unsupported_texture_lod_flags() {
    let mut preview = preview_for_texture_alpha(true, true);
    preview.textures[0].do_edge_lod = true;
    preview.textures[0].bias_clamp = true;
    let issues = validation_issues_for_preview(&test_document(Vec::new()), Some(&preview));

    let issue = issues
        .iter()
        .find(|issue| issue.code == "renderer-gx-texture-lod-unsupported-0")
        .expect("LOD fidelity warning");
    assert!(issue.message.contains("edge LOD"));
    assert!(issue.message.contains("LOD bias clamp"));
}

#[test]
fn software_grid_segments_respect_geometry_depth() {
    let black = egui::Color32::BLACK;
    let mut image = egui::ColorImage::filled([3, 1], black);
    let geometry_depth = [5.0; 3];
    let segment = |depth: f32, x: f32| ProjectedVertex {
        x,
        y: 0.0,
        depth,
        inv_depth: 1.0 / depth,
    };
    let grid_color = egui::Color32::from_rgba_unmultiplied(206, 82, 82, 128);

    rasterize_depth_tested_segment(
        &mut image,
        &geometry_depth,
        segment(10.0, 0.0),
        segment(10.0, 2.0),
        grid_color,
    );
    assert_eq!(image.pixels, vec![black; 3]);

    rasterize_depth_tested_segment(
        &mut image,
        &geometry_depth,
        segment(4.0, 0.0),
        segment(4.0, 2.0),
        grid_color,
    );
    assert!(image.pixels.iter().all(|pixel| *pixel != black));
}

#[test]
fn camera_focus_easing_is_clamped_and_symmetric() {
    assert_eq!(ease_camera_focus(0.0), 0.0);
    assert_eq!(ease_camera_focus(1.0), 1.0);
    assert_eq!(ease_camera_focus(-1.0), 0.0);
    assert_eq!(ease_camera_focus(2.0), 1.0);
    assert!((ease_camera_focus(0.5) - 0.5).abs() < 1e-6);
    // Smoothstep leaves the start slowly, so early progress trails linear.
    assert!(ease_camera_focus(0.25) < 0.25);
    assert!(ease_camera_focus(0.75) > 0.75);
}

#[test]
fn camera_distance_interpolates_geometrically() {
    assert!((interpolate_camera_distance(100.0, 10_000.0, 0.0) - 100.0).abs() < 1e-3);
    assert!((interpolate_camera_distance(100.0, 10_000.0, 1.0) - 10_000.0).abs() < 1e-1);
    // Halfway through a hundred-fold approach sits at the geometric mean,
    // not the arithmetic midpoint of 5050.
    assert!((interpolate_camera_distance(100.0, 10_000.0, 0.5) - 1_000.0).abs() < 1e-1);
    // A degenerate start distance must not produce NaN.
    assert!(interpolate_camera_distance(0.0, 500.0, 0.5).is_finite());
}

#[test]
fn camera_focus_target_centers_bounds_and_scales_with_size() {
    let (focus, distance) =
        camera_focus_target_for_bounds([[-100.0, 0.0, -100.0], [100.0, 400.0, 100.0]]);
    assert!((focus[0] - 0.0).abs() < 1e-3);
    assert!((focus[1] - 200.0).abs() < 1e-3);
    assert!((focus[2] - 0.0).abs() < 1e-3);
    assert!(distance >= CAMERA_FOCUS_DISTANCE_MIN);

    let (_, far) = camera_focus_target_for_bounds([[-5_000.0; 3], [5_000.0; 3]]);
    assert!(
        far > distance,
        "a larger object must be framed from farther"
    );
}

#[test]
fn camera_focus_target_without_geometry_frames_the_origin() {
    let (focus, distance) = camera_focus_target_from_bounds(None, [10.0, 20.0, 30.0]);
    assert_eq!(focus, [10.0, 20.0, 30.0]);
    assert_eq!(
        distance,
        (CAMERA_FOCUS_FALLBACK_RADIUS * CAMERA_FOCUS_RADIUS_SCALE)
            .clamp(CAMERA_FOCUS_DISTANCE_MIN, CAMERA_FOCUS_DISTANCE_MAX)
    );
}

#[test]
fn camera_focus_animation_lands_exactly_on_its_target() {
    let mut app = camera_app();
    app.viewport_pan = egui::vec2(40.0, -25.0);
    app.viewport_zoom = 0.35;
    app.begin_camera_focus_animation([500.0, 100.0, -250.0], 8_000.0);

    let mut steps = 0;
    while app.advance_camera_focus_animation(1.0 / 60.0) {
        steps += 1;
        assert!(steps < 600, "the glide must terminate");
    }

    assert!(steps > 1, "the glide must take more than one frame");
    let camera = app.renderer.camera();
    assert_eq!(camera.focus, [500.0, 100.0, -250.0]);
    assert!((camera.distance - 8_000.0).abs() < 1e-1);
    assert_eq!(app.viewport_pan, egui::Vec2::ZERO);
    assert_eq!(app.viewport_zoom, 1.0);
}

#[test]
fn camera_focus_never_frames_closer_than_the_navigation_floor() {
    // Fly speed scales with the orbit distance and the distance is persisted
    // with the stage camera, so framing must not strand the user crawling.
    let mut app = camera_app();
    app.begin_camera_focus_animation([0.0, 0.0, 0.0], 10.0);
    while app.advance_camera_focus_animation(1.0 / 60.0) {}
    assert_eq!(app.renderer.camera().distance, CAMERA_FOCUS_DISTANCE_MIN);

    // Exercise the real speed formula rather than the constants: framing must
    // leave fly speed clear of its own lower clamp, or navigation is pinned to
    // minimum speed after every framing command.
    app.camera_speed = 1.0;
    assert!(
        app.viewport_fly_speed() > 300.0,
        "framing left the camera at minimum fly speed"
    );
}

#[test]
fn manual_camera_movement_cancels_an_in_flight_glide() {
    let mut app = camera_app();
    app.begin_camera_focus_animation([9_000.0, 0.0, 0.0], 400.0);
    app.advance_camera_focus_animation(1.0 / 60.0);

    app.translate_camera([10.0, 0.0, 0.0]);
    assert!(
        !app.advance_camera_focus_animation(1.0 / 60.0),
        "panning or flying must hand control back to the user"
    );

    // Orbiting is deliberately allowed to run alongside the glide.
    app.begin_camera_focus_animation([9_000.0, 0.0, 0.0], 400.0);
    app.orbit_camera(egui::vec2(5.0, 0.0));
    assert!(app.advance_camera_focus_animation(1.0 / 60.0));
}

#[test]
fn framing_commands_supersede_a_running_glide() {
    let mut app = camera_app();
    app.begin_camera_focus_animation([9_000.0, 0.0, 0.0], 400.0);
    app.stop_camera_fly();
    assert!(!app.advance_camera_focus_animation(1.0 / 60.0));
}

#[test]
fn camera_focus_animation_rejects_non_finite_targets() {
    let mut app = camera_app();
    app.begin_camera_focus_animation([f32::NAN, 0.0, 0.0], 400.0);
    assert!(!app.advance_camera_focus_animation(1.0 / 60.0));
    app.begin_camera_focus_animation([0.0, 0.0, 0.0], f32::INFINITY);
    assert!(!app.advance_camera_focus_animation(1.0 / 60.0));
}

#[test]
fn select_tool_is_reachable_and_raises_no_gizmo() {
    // Q switches to Select from every transform tool, matching W/E/R.
    for tool in [EditorTool::Move, EditorTool::Rotate, EditorTool::Scale] {
        assert_eq!(
            tool.after_keyboard_shortcut(egui::Key::Q),
            EditorTool::Select
        );
    }
    // Select still hands off to the other tools.
    assert_eq!(
        EditorTool::Select.after_keyboard_shortcut(egui::Key::W),
        EditorTool::Move
    );
    assert_eq!(
        EditorTool::Select.after_keyboard_shortcut(egui::Key::G),
        EditorTool::Select
    );
    // Goop keeps owning its own shortcuts.
    assert_eq!(
        EditorTool::Goop.after_keyboard_shortcut(egui::Key::Q),
        EditorTool::Goop
    );

    let mut app = camera_app();
    app.tool = EditorTool::Select;
    assert!(
        !app.tool_supports_transform_gizmo(),
        "the Select tool must not raise a transform gizmo"
    );
    for tool in [EditorTool::Move, EditorTool::Rotate, EditorTool::Scale] {
        app.tool = tool;
        assert!(app.tool_supports_transform_gizmo());
    }
}

#[test]
fn frame_selected_glides_instead_of_snapping() {
    let mut app = camera_app();
    let mut object = SceneObject::new("actor", "Toad");
    object.transform.translation = [4_000.0, 250.0, -1_500.0];
    app.document = Some(test_document(vec![object]));
    app.selected_object_id = Some("actor".to_string());

    let before_focus = app.renderer.camera().focus;
    app.frame_selected();

    // The camera must not have jumped on the command frame.
    assert_eq!(app.renderer.camera().focus, before_focus);
    assert!(
        app.advance_camera_focus_animation(1.0 / 60.0),
        "F must start a glide"
    );

    while app.advance_camera_focus_animation(1.0 / 60.0) {}
    assert_eq!(app.renderer.camera().focus, [4_000.0, 250.0, -1_500.0]);
    // No preview geometry, so the object frames at the fallback distance.
    assert_eq!(
        app.renderer.camera().distance,
        (CAMERA_FOCUS_FALLBACK_RADIUS * CAMERA_FOCUS_RADIUS_SCALE)
            .clamp(CAMERA_FOCUS_DISTANCE_MIN, CAMERA_FOCUS_DISTANCE_MAX)
    );
}

#[test]
fn frame_selected_without_a_selection_does_nothing() {
    let mut app = camera_app();
    let before_focus = app.renderer.camera().focus;
    let before_distance = app.renderer.camera().distance;
    app.frame_selected();
    assert!(!app.advance_camera_focus_animation(1.0 / 60.0));
    assert_eq!(app.renderer.camera().focus, before_focus);
    assert_eq!(app.renderer.camera().distance, before_distance);
}

#[test]
fn clicking_the_active_toolbar_tool_returns_to_select() {
    // Clicking an inactive tool activates it.
    assert_eq!(
        EditorTool::Select.after_toolbar_click(EditorTool::Move),
        EditorTool::Move
    );
    assert_eq!(
        EditorTool::Move.after_toolbar_click(EditorTool::Scale),
        EditorTool::Scale
    );

    // Clicking the active tool toggles it off, back to Select.
    for tool in [EditorTool::Move, EditorTool::Rotate, EditorTool::Scale] {
        assert_eq!(tool.after_toolbar_click(tool), EditorTool::Select);
    }

    // Select is the neutral state, so it has nothing to toggle off to.
    assert_eq!(
        EditorTool::Select.after_toolbar_click(EditorTool::Select),
        EditorTool::Select
    );
}

#[test]
fn framing_does_not_change_viewport_fly_speed() {
    // Fly speed used to read camera.distance directly, so framing an actor
    // shrank it and the camera crawled afterwards. Worse, the shrunken value
    // was persisted with the stage camera, so the slowdown survived reloads.
    let mut app = camera_app();
    app.camera_speed = 8.0;
    app.camera_navigation_distance = 7000.0;
    let before = app.viewport_fly_speed();

    app.begin_camera_focus_animation([0.0, 0.0, 0.0], 426.0);
    while app.advance_camera_focus_animation(1.0 / 60.0) {}

    assert!(
        (app.renderer.camera().distance - 426.0).abs() < 1.0,
        "framing still sets the orbit distance"
    );
    assert_eq!(
        app.viewport_fly_speed(),
        before,
        "framing must not change navigation speed"
    );
}

#[test]
fn resetting_the_camera_re_establishes_navigation_speed() {
    let mut app = camera_app();
    app.camera_speed = 8.0;
    app.camera_navigation_distance = 426.0;
    let slow = app.viewport_fly_speed();
    app.reset_camera();
    assert!(
        app.viewport_fly_speed() > slow,
        "Reset Camera has to recover speed from a poisoned project"
    );
    assert_eq!(app.camera_navigation_distance, 7000.0);
}

#[test]
fn preview_bounds_skip_every_non_world_space_layer() {
    // The framing bounds originally excluded only Sky, while the existing
    // bounds path also excludes MirrorScene and Heatwave. Those are
    // reprojected effect passes and do not say where an object sits.
    for layer in [
        PreviewRenderLayer::Sky,
        PreviewRenderLayer::MirrorScene,
        PreviewRenderLayer::Heatwave,
    ] {
        assert!(
            !preview_layer_is_world_space(layer),
            "{layer:?} is not world-space geometry"
        );
    }
    assert!(preview_layer_is_world_space(PreviewRenderLayer::Main));
}

#[test]
fn framing_bounds_ignore_billboards_and_effect_layers() {
    // A Shine's glow and Petey's effects are billboards and particles. Their
    // stored vertices are not world positions, so counting them inflated the
    // framing box and the camera pulled back a long way from the model.
    let mut solid = textured_blended_triangle();
    solid.billboard = None;
    solid.render_layer = PreviewRenderLayer::Main;
    assert!(preview_triangle_frames_object(&solid));

    let mut billboard = solid;
    billboard.billboard = Some(J3dBillboard {
        mode: sms_formats::J3dBillboardMode::Full,
        center: [0.0, 0.0, 0.0],
        axes: [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]],
        offsets: [[0.0; 3]; 3],
        normals: None,
    });
    assert!(!preview_triangle_frames_object(&billboard));

    for layer in [
        PreviewRenderLayer::Particle,
        PreviewRenderLayer::Heatwave,
        PreviewRenderLayer::Sky,
        PreviewRenderLayer::MirrorScene,
    ] {
        let mut effect = solid;
        effect.render_layer = layer;
        assert!(
            !preview_triangle_frames_object(&effect),
            "{layer:?} must not contribute to framing bounds"
        );
    }
}

#[test]
fn k_frames_the_origin_grid_from_a_lost_camera() {
    let mut app = camera_app();
    {
        let camera = app.renderer.camera_mut();
        camera.focus = [250_000.0, 90_000.0, -180_000.0];
        camera.pitch_degrees = 60.0;
        camera.distance = 400_000.0;
    }

    app.frame_world_origin();
    while app.advance_camera_focus_animation(1.0 / 60.0) {}

    let camera = app.renderer.camera();
    assert_eq!(camera.focus, [0.0, 0.0, 0.0]);
    assert!(
        camera.pitch_degrees <= -5.0,
        "an upward camera must be tipped down so the ground plane is visible"
    );
    assert!(camera.distance > crate::viewport_ui::WORLD_GRID_HALF_EXTENT);
    assert!(camera.distance < 100_000.0);
}

#[test]
fn k_keeps_a_downward_camera_angle_alone() {
    let mut app = camera_app();
    app.renderer.camera_mut().pitch_degrees = -45.0;
    app.frame_world_origin();
    assert_eq!(app.renderer.camera().pitch_degrees, -45.0);
}

#[test]
fn the_tools_menu_toggles_goop_off_again() {
    // Picking the active tool a second time drops back to Select, so the goop
    // tool can be left without hunting for another one.
    assert_eq!(
        EditorTool::Move.after_toolbar_click(EditorTool::Goop),
        EditorTool::Goop
    );
    assert_eq!(
        EditorTool::Goop.after_toolbar_click(EditorTool::Goop),
        EditorTool::Select
    );
}

#[test]
fn terrain_asset_tools_share_their_undo_history() {
    assert!(EditorTool::VertexPaint.uses_terrain_asset_undo());
    assert!(EditorTool::Boolean.uses_terrain_asset_undo());
    assert!(!EditorTool::Move.uses_terrain_asset_undo());
}

#[test]
fn goop_keeps_its_keys_while_active() {
    // Goop owns the keyboard while it is the active tool, so a grab or a tool
    // switch cannot fire underneath it mid-paint.
    for key in [
        egui::Key::Q,
        egui::Key::W,
        egui::Key::E,
        egui::Key::R,
        egui::Key::G,
    ] {
        assert_eq!(
            EditorTool::Goop.after_keyboard_shortcut(key),
            EditorTool::Goop
        );
    }
}

#[test]
fn triangle_height_lookup_covers_its_own_column() {
    // Flat quad corner at y = 40 spanning the origin.
    let flat = [[0.0, 40.0, 0.0], [100.0, 40.0, 0.0], [0.0, 40.0, 100.0]];
    assert_eq!(
        crate::viewport_ui::triangle_height_at_xz(flat, 10.0, 10.0),
        Some(40.0)
    );
    // Outside the triangle, so nothing underfoot.
    assert_eq!(
        crate::viewport_ui::triangle_height_at_xz(flat, 90.0, 90.0),
        None
    );
    // A ramp interpolates rather than picking a corner.
    let ramp = [[0.0, 0.0, 0.0], [100.0, 100.0, 0.0], [0.0, 0.0, 100.0]];
    let height = crate::viewport_ui::triangle_height_at_xz(ramp, 50.0, 10.0).expect("on the ramp");
    assert!((height - 50.0).abs() < 0.001, "got {height}");
    // A vertical face has no column to stand on.
    let wall = [[0.0, 0.0, 0.0], [0.0, 100.0, 0.0], [0.0, 0.0, 100.0]];
    assert_eq!(
        crate::viewport_ui::triangle_height_at_xz(wall, 10.0, 10.0),
        None
    );
}

#[test]
fn escape_leaves_placement_mode() {
    let mut app = SmsEditorApp {
        tool: EditorTool::Place,
        active_placement: Some(ActivePlacement::Object {
            factory_name: "Amenbo".to_string(),
        }),
        ..SmsEditorApp::default()
    };

    assert!(app.cancel_active_placement());
    assert!(app.active_placement.is_none());
    // Place is a mode, so leaving it has to land somewhere usable.
    assert_eq!(app.tool, EditorTool::Select);

    // Nothing to cancel, so Escape stays available to whatever else wants it.
    assert!(!app.cancel_active_placement());
}
/// Hue rotation recolours without relighting.
///
/// The matrix constants are written out by hand rather than derived, so the
/// property that justifies them is worth pinning: spinning hue must leave
/// luminance where it was, or grading a bake would quietly change its shading.
#[test]
fn rotating_hue_holds_luminance_and_leaves_grey_alone() {
    let luma = |color: [f32; 4]| 0.213 * color[0] + 0.715 * color[1] + 0.072 * color[2];
    let mut settings = crate::vertex_paint::VertexPaintGradeSettings::default();

    for degrees in [-180.0, -90.0, -33.0, 0.0, 45.0, 120.0, 180.0] {
        settings.hue = degrees;
        // Graded against a reference of 1.0, so these read as shaded and the
        // rotation is mixed in at full strength.

        // Kept away from the edge of the gamut on purpose. A saturated colour
        // rotates to a negative channel, and clamping that back into range is
        // what moves its luminance -- the rotation is exact, staying in gamut
        // is not.
        for start in [
            [0.60, 0.45, 0.50, 1.0],
            [0.45, 0.55, 0.50, 1.0],
            [0.35, 0.35, 0.35, 1.0],
        ] {
            let mut color = start;
            settings.apply(&mut color, [1.0; 3]);
            assert!(
                (luma(color) - luma(start)).abs() < 0.005,
                "hue {degrees} moved luminance of {start:?} to {color:?}"
            );
        }

        // Grey has no hue to turn, so it has to come back untouched.
        let mut grey = [0.5, 0.5, 0.5, 1.0];
        settings.apply(&mut grey, [1.0; 3]);
        for channel in grey.iter().take(3) {
            assert!(
                (channel - 0.5).abs() < 0.01,
                "hue {degrees} tinted grey: {grey:?}"
            );
        }
    }
}

/// Reports why the retail census dropped a factory. Local diagnostic:
/// `GRAFFITO_PROBE_BASE_ROOT=<extracted game> GRAFFITO_PROBE_FACTORY=kuri
/// cargo test probe_authoring_census -- --ignored --nocapture`
#[test]
#[ignore]
fn probe_authoring_census() {
    let Ok(base_root) = std::env::var("GRAFFITO_PROBE_BASE_ROOT") else {
        return;
    };
    let needle = std::env::var("GRAFFITO_PROBE_FACTORY")
        .unwrap_or_default()
        .to_ascii_lowercase();
    let base_root = std::path::Path::new(&base_root);
    let archives =
        sms_formats::discover_scene_archives(base_root).expect("discover retail scene archives");
    let retail = archives
        .iter()
        .filter(|archive| archive.size_bytes > 0)
        .cloned()
        .collect::<Vec<_>>();
    println!("retail archives: {}", retail.len());

    let registry = sms_schema::bundled_object_registry()
        .expect("bundled registry")
        .registry;
    let build =
        sms_scene::ObjectAuthoringCatalog::build_with_base_root(&retail, &registry, base_root);
    println!("templates: {}", build.catalog.len());
    println!("warnings: {}", build.warnings.len());

    println!("\n--- templates matching {needle:?} ---");
    for (name, template) in build.catalog.iter() {
        if name.to_ascii_lowercase().contains(&needle) {
            println!("  present: {name} (from {})", template.source_stage);
            for resource in &template.resources {
                println!("    resource: {}", resource.source_asset_path.display());
            }
        }
    }

    println!("\n--- warnings matching {needle:?} ---");
    for warning in build.warnings.iter() {
        if warning.message.to_ascii_lowercase().contains(&needle) {
            println!("  [{}] {}", warning.source_stage, warning.message);
        }
    }
}

/// Lists which retail stages actually place a factory. Local diagnostic:
/// `GRAFFITO_PROBE_BASE_ROOT=<extracted game> GRAFFITO_PROBE_FACTORY=kuri
/// cargo test probe_factory_stages -- --ignored --nocapture`
#[test]
#[ignore]
fn probe_factory_stages() {
    let Ok(base_root) = std::env::var("GRAFFITO_PROBE_BASE_ROOT") else {
        return;
    };
    let needle = std::env::var("GRAFFITO_PROBE_FACTORY")
        .unwrap_or_default()
        .to_ascii_lowercase();
    let base_root = std::path::Path::new(&base_root);
    let archives =
        sms_formats::discover_scene_archives(base_root).expect("discover retail scene archives");

    for archive in archives.iter().filter(|archive| archive.size_bytes > 0) {
        let Ok(assets) = sms_formats::mount_scene_archive(&archive.path) else {
            continue;
        };
        let mut hits: Vec<String> = Vec::new();
        let mut asset_names: Vec<String> = Vec::new();
        for asset in &assets {
            let name = asset.path.to_string_lossy().to_ascii_lowercase();
            if !name.ends_with(".bin") {
                continue;
            }
            let Ok(bytes) = sms_formats::read_stage_asset_bytes(&asset.path) else {
                continue;
            };
            let Ok(records) = sms_formats::parse_jdrama_object_records(&bytes) else {
                continue;
            };
            for record in &records {
                let matches = match std::env::var("GRAFFITO_PROBE_STAGE") {
                    Ok(stage) => archive.stage_id.eq_ignore_ascii_case(&stage),
                    Err(_) => record.type_name.to_ascii_lowercase().contains(&needle),
                };
                if matches && !hits.contains(&record.type_name) {
                    hits.push(record.type_name.clone());
                    let file = asset
                        .path
                        .file_name()
                        .map(|name| name.to_string_lossy().to_string())
                        .unwrap_or_default();
                    if !asset_names.contains(&file) {
                        asset_names.push(file);
                    }
                }
            }
        }
        if !hits.is_empty() {
            println!(
                "{} [{}]: {}",
                archive.stage_id,
                asset_names.join(" "),
                hits.join(", ")
            );
        }
    }
}

/// Prints StageEnemyInfo entries from every retail tables.bin. Local
/// diagnostic: `GRAFFITO_PROBE_BASE_ROOT=<extracted game> cargo test
/// probe_enemy_tables -- --ignored --nocapture`
#[test]
#[ignore]
fn probe_enemy_tables() {
    fn walk(stage: &str, record: &sms_formats::JDramaRecord) {
        if record.type_name == "StageEnemyInfo" {
            let fields = match &record.payload {
                sms_formats::JDramaRecordPayload::Fields { fields } => fields.as_slice(),
                sms_formats::JDramaRecordPayload::Actor { fields, .. } => fields.as_slice(),
                sms_formats::JDramaRecordPayload::Group { fields, .. } => fields.as_slice(),
                sms_formats::JDramaRecordPayload::Empty => &[],
            };
            let read = |name: &str| {
                fields
                    .iter()
                    .find(|field| field.name == name)
                    .map(|field| format!("{:?}", field.value))
                    .unwrap_or_default()
            };
            println!(
                "{stage}: name={:?} manager={} flags={} weight={}",
                record.name,
                read("manager_name"),
                read("flags"),
                read("weight")
            );
        }
        if let sms_formats::JDramaRecordPayload::Group { children, .. } = &record.payload {
            for child in children {
                walk(stage, child);
            }
        }
    }

    let Ok(base_root) = std::env::var("GRAFFITO_PROBE_BASE_ROOT") else {
        return;
    };
    let base_root = std::path::Path::new(&base_root);
    let archives = sms_formats::discover_scene_archives(base_root).expect("discover archives");
    for archive in archives.iter().filter(|archive| archive.size_bytes > 0) {
        let Ok(assets) = sms_formats::mount_scene_archive(&archive.path) else {
            continue;
        };
        for asset in &assets {
            if !asset
                .path
                .to_string_lossy()
                .to_ascii_lowercase()
                .ends_with("tables.bin")
            {
                continue;
            }
            let Ok(bytes) = sms_formats::read_stage_asset_bytes(&asset.path) else {
                continue;
            };
            let Ok(document) = sms_formats::parse_jdrama_document(&bytes) else {
                println!(
                    "{}: tables.bin did not parse as a document",
                    archive.stage_id
                );
                continue;
            };
            walk(&archive.stage_id, &document.root);
        }
    }
}

/// Dumps a named record from a stage's scene.bin as the editor parses it.
/// `GRAFFITO_PROBE_BASE_ROOT=... GRAFFITO_PROBE_STAGE=bianco0
/// GRAFFITO_PROBE_TYPE=NameKuriManager cargo test probe_scene_record --
/// --ignored --nocapture`
#[test]
#[ignore]
fn probe_scene_record() {
    fn walk(record: &sms_formats::JDramaRecord, wanted: &str) {
        if wanted == "*" {
            println!("{} ({})", record.type_name, record.name);
        } else if record.type_name == wanted {
            println!("{:#?}", record);
        }
        if let sms_formats::JDramaRecordPayload::Group { children, .. } = &record.payload {
            for child in children {
                walk(child, wanted);
            }
        }
    }
    let Ok(base_root) = std::env::var("GRAFFITO_PROBE_BASE_ROOT") else {
        return;
    };
    let stage = std::env::var("GRAFFITO_PROBE_STAGE").unwrap_or_default();
    let wanted = std::env::var("GRAFFITO_PROBE_TYPE").unwrap_or_default();
    let base_root = std::path::Path::new(&base_root);
    let archives = sms_formats::discover_scene_archives(base_root).expect("discover archives");
    for archive in archives
        .iter()
        .filter(|archive| archive.stage_id.eq_ignore_ascii_case(&stage))
    {
        let Ok(assets) = sms_formats::mount_scene_archive(&archive.path) else {
            continue;
        };
        for asset in &assets {
            let name = asset.path.to_string_lossy().to_ascii_lowercase();
            let suffix =
                std::env::var("GRAFFITO_PROBE_FILE").unwrap_or_else(|_| "scene.bin".to_string());
            if !name.ends_with(&suffix) {
                continue;
            }
            let Ok(bytes) = sms_formats::read_stage_asset_bytes(&asset.path) else {
                continue;
            };
            let Ok(document) = sms_formats::parse_jdrama_document(&bytes) else {
                continue;
            };
            walk(&document.root, &wanted);
        }
    }
}

/// Byte-compares encode(parse(tables.bin)) against retail for every stage.
/// `GRAFFITO_PROBE_BASE_ROOT=... cargo test probe_tables_roundtrip -- --ignored
/// --nocapture`
#[test]
#[ignore]
fn probe_tables_roundtrip() {
    let Ok(base_root) = std::env::var("GRAFFITO_PROBE_BASE_ROOT") else {
        return;
    };
    let base_root = std::path::Path::new(&base_root);
    let archives = sms_formats::discover_scene_archives(base_root).expect("discover archives");
    let mut same = 0usize;
    let mut different = 0usize;
    for archive in archives.iter().filter(|archive| archive.size_bytes > 0) {
        let Ok(assets) = sms_formats::mount_scene_archive(&archive.path) else {
            continue;
        };
        for asset in &assets {
            if !asset
                .path
                .to_string_lossy()
                .to_ascii_lowercase()
                .ends_with("tables.bin")
            {
                continue;
            }
            let Ok(bytes) = sms_formats::read_stage_asset_bytes(&asset.path) else {
                continue;
            };
            let Ok(document) = sms_formats::parse_jdrama_document(&bytes) else {
                println!("{}: parse failed", archive.stage_id);
                continue;
            };
            match sms_formats::encode_jdrama_document(&document) {
                Ok(encoded) if encoded == bytes => same += 1,
                Ok(encoded) => {
                    different += 1;
                    if different <= 3 {
                        println!(
                            "{}: DIFFERENT ({} vs {} bytes); first divergence at {:?}",
                            archive.stage_id,
                            encoded.len(),
                            bytes.len(),
                            encoded
                                .iter()
                                .zip(bytes.iter())
                                .position(|(left, right)| left != right)
                        );
                    }
                }
                Err(error) => {
                    different += 1;
                    println!("{}: encode failed: {error}", archive.stage_id);
                }
            }
        }
    }
    println!("identical: {same}  different: {different}");
}

/// Lists resources in an exported stage archive matching a filter.
/// `GRAFFITO_PROBE_SZS=<path> GRAFFITO_PROBE_MATCH=pollution cargo test
/// probe_szs_contents -- --ignored --nocapture`
#[test]
#[ignore]
fn probe_szs_contents() {
    let Ok(path) = std::env::var("GRAFFITO_PROBE_SZS") else {
        return;
    };
    let needle = std::env::var("GRAFFITO_PROBE_MATCH")
        .unwrap_or_default()
        .to_ascii_lowercase();
    let assets = sms_formats::mount_scene_archive(std::path::Path::new(&path)).expect("mount szs");
    let mut shown = 0usize;
    for asset in &assets {
        let name = asset.path.to_string_lossy().replace('\\', "/");
        if needle.is_empty() || name.to_ascii_lowercase().contains(&needle) {
            println!("{name}");
            shown += 1;
        }
    }
    println!("({shown} of {} assets matched)", assets.len());
}

/// Hashes one resource across several retail stages.
/// `GRAFFITO_PROBE_BASE_ROOT=... GRAFFITO_PROBE_RES=hamukuri/default.bmd
/// cargo test probe_resource_hashes -- --ignored --nocapture`
#[test]
#[ignore]
fn probe_resource_hashes() {
    let Ok(base_root) = std::env::var("GRAFFITO_PROBE_BASE_ROOT") else {
        return;
    };
    let wanted = std::env::var("GRAFFITO_PROBE_RES")
        .unwrap_or_default()
        .to_ascii_lowercase();
    let base_root = std::path::Path::new(&base_root);
    let archives = sms_formats::discover_scene_archives(base_root).expect("discover");
    for archive in archives.iter().filter(|archive| archive.size_bytes > 0) {
        let Ok(assets) = sms_formats::mount_scene_archive(&archive.path) else {
            continue;
        };
        for asset in &assets {
            let name = asset
                .path
                .to_string_lossy()
                .replace('\\', "/")
                .to_ascii_lowercase();
            if !name.ends_with(&wanted) {
                continue;
            }
            let Ok(bytes) = sms_formats::read_stage_asset_bytes(&asset.path) else {
                continue;
            };
            let mut hash = 0xcbf2_9ce4_8422_2325u64;
            for byte in &bytes {
                hash ^= u64::from(*byte);
                hash = hash.wrapping_mul(0x0000_0100_0000_01B3);
            }
            println!(
                "{}: {} bytes, fnv {:016x}",
                archive.stage_id,
                bytes.len(),
                hash
            );
        }
    }
}

/// Bakes the stain into the real retail Stu model and round-trips it.
/// `GRAFFITO_PROBE_BASE_ROOT=... cargo test probe_stain_bake -- --ignored
/// --nocapture`
#[test]
#[ignore]
fn probe_stain_bake() {
    let Ok(base_root) = std::env::var("GRAFFITO_PROBE_BASE_ROOT") else {
        return;
    };
    let base_root = std::path::Path::new(&base_root);
    let archives = sms_formats::discover_scene_archives(base_root).expect("discover");
    let bianco = archives
        .iter()
        .find(|archive| archive.stage_id == "bianco0")
        .expect("bianco0");
    let assets = sms_formats::mount_scene_archive(&bianco.path).expect("mount");
    let find = |suffix: &str| {
        assets
            .iter()
            .find(|asset| {
                asset
                    .path
                    .to_string_lossy()
                    .to_ascii_lowercase()
                    .ends_with(suffix)
            })
            .map(|asset| sms_formats::read_stage_asset_bytes(&asset.path).expect("read"))
            .expect(suffix)
    };
    let model_bytes = find("hamukuri/default.bmd");
    let stain_bytes = find("h_ma_rak.bti");

    let mut model = sms_formats::J3dRebuildDocument::parse(&model_bytes).expect("parse model");
    let stain = sms_formats::BtiFile::parse(&stain_bytes).expect("parse stain");

    for section in &model.sections {
        let sms_formats::J3dRebuildSectionData::Materials(materials) = &section.data else {
            continue;
        };
        if let Some(names) = &materials.names {
            for (index, entry) in names.entries.iter().enumerate() {
                if entry.name == "_mat_body_top1" {
                    println!("material index {index}");
                }
            }
        }
        for (index, record) in materials.material_init_records.iter().enumerate() {
            println!(
                "record {index}: alpha {:?} color {:?}",
                record.tev_konst_alpha_selectors, record.tev_konst_color_selectors
            );
        }
    }
    let replaced = model
        .replace_named_texture_from_bti("H_ma_rak_dummy", &stain)
        .expect("replace texture");
    let pinned = model
        .pin_material_konst_alpha_half("_mat_body_top1")
        .expect("pin alpha");
    println!("replaced {replaced} texture slot(s), pinned {pinned} TEV stage(s)");
    assert!(
        replaced > 0,
        "the dummy slot must exist in the retail model"
    );
    assert!(pinned > 0, "the stain material must use K0 alpha");

    let encoded = model.to_bytes().expect("encode");
    let reparsed = sms_formats::J3dRebuildDocument::parse(&encoded).expect("reparse");
    let repinned = {
        let mut copy = reparsed.clone();
        copy.pin_material_konst_alpha_half("_mat_body_top1")
            .expect("pin twice")
    };
    assert_eq!(repinned, 0, "a second pin must find nothing left to pin");

    // The toggle's off state: unpinning must restore the pristine selectors
    // exactly, since the retail material carries no 0x04 of its own.
    let mut unbaked = reparsed.clone();
    let unpinned = unbaked
        .pin_material_konst_alpha_half("_mat_body_top1")
        .map(|_| 0usize)
        .unwrap_or(0)
        + unbaked
            .unpin_material_konst_alpha_half("_mat_body_top1")
            .expect("unpin");
    assert_eq!(unpinned, pinned, "unpin must reverse exactly what pin did");
    let pristine = sms_formats::J3dRebuildDocument::parse(&model_bytes).expect("pristine");
    for (section, original) in unbaked.sections.iter().zip(pristine.sections.iter()) {
        let (
            sms_formats::J3dRebuildSectionData::Materials(edited),
            sms_formats::J3dRebuildSectionData::Materials(reference),
        ) = (&section.data, &original.data)
        else {
            continue;
        };
        for (record, reference_record) in edited
            .material_init_records
            .iter()
            .zip(reference.material_init_records.iter())
        {
            assert_eq!(
                record.tev_konst_alpha_selectors,
                reference_record.tev_konst_alpha_selectors
            );
        }
    }
}
