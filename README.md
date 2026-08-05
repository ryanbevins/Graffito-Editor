<div align="center">

# Graffito Editor

**A data-driven authoring environment for _Super Mario Sunshine_.**

[![CI](https://github.com/ryanbevins/Graffito-Editor/actions/workflows/ci.yml/badge.svg?branch=main)](https://github.com/ryanbevins/Graffito-Editor/actions/workflows/ci.yml)
[![License: MIT](https://img.shields.io/badge/License-MIT-yellow.svg)](LICENSE)
[![Discord](https://img.shields.io/badge/Discord-Join-5865F2?logo=discord&logoColor=white)](https://discord.gg/JhPr3fWuy)

[Join the Discord community](https://discord.gg/JhPr3fWuy) ·
[Project format](docs/project-format.md) ·
[Contributing](CONTRIBUTING.md) ·
[Security](SECURITY.md)

</div>

> [!WARNING]
> Graffito is an experimental development preview.
> The project has no official binary or public release.
> File formats and workflows can change before the first release.
> Keep backups and test each build in Dolphin.

Graffito is a Rust application for authoring, building, and testing
_Super Mario Sunshine_ content.

Graffito can inspect retail stages, create source-free stages, author game
content, build a separate game copy, and open stages in Dolphin.

The user must supply a legally obtained game extraction. Graffito does not
include Nintendo assets or change the base extraction. Each project stores its
editable data and builds in separate locations.

Graffito uses the
[Super Mario Sunshine decompilation project](https://github.com/doldecomp/sms)
and Nintendo JSystem/GX behavior as technical references. It uses typed metadata
instead of hardcoded object lists or unverified binary patches.

## AI-assisted development

AI coding tools help with research, code, tests, documentation, and iteration.

AI output does not define game formats or behavior. The SMS decompilation and
JSystem/GX behavior remain the technical sources of truth. The maintainer
reviews the project design and each published change.

## Current support

### Projects and stages

- Create, open, and reopen `.sms` projects.
- Browse retail stages by localized area name.
- Import older Graffito project folders.
- Create minimal stages without copying retail stage files.
- Store project-owned runtime mappings for new stages.

### Content Browser

- Browse project stages and models with read-only game content.
- Browse game stages, objects, skyboxes, music, sounds, and raw files.
- Search, filter, sort, favorite, and preview items.
- Use grid or list views, breadcrumbs, history, and context actions.

### Scene authoring

- Place cataloged actors, enemies, NPCs, map objects, and Mario.
- Create objects from typed templates derived from retail data.
- Edit transforms with viewport gizmos, snapping, and inspector controls.
- Duplicate or delete objects, and use undo or redo.
- Copy required dependencies and resources with each object.

### Models, terrain, and collision

- Import rigid `.gltf` or `.glb` geometry into `.smsmodel` assets.
- Edit supported GX material and collision settings.
- Export models as separate runtime objects.
- Replace terrain or skybox models.
- Use stock resource replacements that the decompilation confirms.

### Routes

- Inspect Sunshine rail graphs in the viewport.
- Create, duplicate, rename, assign, connect, split, reverse, or disconnect routes.
- Edit one-way and two-way links.
- Bake Bezier handles into runtime nodes.

### NPC dialogue

- Edit dialogue for placed actors with supported retail talk routes.
- Create per-instance dialogue edits.
- Confirm edits that affect shared dialogue.
- Edit text, known controls, choices, page breaks, voices, and balloons.
- Generate supported talk routes.

### Goop

- Inspect retail pollution layers.
- Generate floor layers and depth data from the final terrain.
- Select styles and behavior from retail data.
- Paint, erase, or fill connected areas in the viewport.
- Rebuild stale resources after terrain changes.
- Preserve retail wall and wave layers as read-only data.

### Sky, lighting, and audio

- Apply a full retail skybox bundle.
- Use an authored model as a skybox.
- Edit stage lights and ambient colors.
- Assign stage music.
- Inspect point, rail, and volume audio helpers.
- Preview supported JAudio music and sounds from the selected game data.

### Viewport

- Preview BMD and BDL models through `wgpu`.
- Preview BMT and BTI materials and textures.
- Preview supported animations, particles, water, goop, grass, and wires.
- Display collision and supported placed actors.
- Use selection, camera, gizmo, view, and overlay controls.

### Build and play

- Save editable drafts while validation issues remain.
- Use **Build Game** to validate and build the project.
- Create an independent runnable `run-root`.
- Use **Launch in Editor** to embed Dolphin on Windows.
- Use **Launch in Dolphin** to start Dolphin separately.
- Direct-boot the open stage without changing the base extraction.
- Restart the same area and scenario after losing a life in a direct-boot session.

## Command-line tools

The `sms-cli` package exposes lower-level inspection and automation commands.

It supports model import, model compilation, stage creation, schema generation,
asset discovery, validation, archive rebuilds, project exports, and Dolphin
launch. It can also verify route data across retail stage archives.

List all commands:

```powershell
cargo run --locked -p sms-cli -- --help
```

## Project and build layout

A Graffito project uses a small `.sms` descriptor.

```text
My Project.sms          Project identity and paths
My Project.smsdata/     Editable scenes and authored content
My Project.smsbuild/    Protected build output
  run-root/             Runnable game directory
```

Use this workflow:

1. Create or open a project.
2. Select a legally obtained extracted game directory.
3. Open a retail stage or create a source-free stage.
4. Edit content through the browser, viewport, outliner, and inspector.
5. Save the project at any time.
6. Fix all build validation errors.
7. Select **Build Game**.
8. Test the `run-root` through Dolphin.

Saving updates the editable project data. It does not create a runnable mod.

**Build Game** validates the stage and creates a separate game copy. This copy
contains user-owned game data. Do not commit or distribute it.

Ownership markers, path checks, atomic file replacement, and rollback protect
the project and its read-only base data.

See the [project format documentation](docs/project-format.md) for the
descriptor schema, source-free stage layout, and build structure.

## Current limitations

- Graffito is pre-1.0 software.
- The project has no compatibility guarantee or supported installer.
- Graffito supports Windows 10 and 11 as its primary desktop targets.
- Linux CI checks the core crates and desktop compilation.
- **Launch in Editor** works only on Windows.
- The viewport approximates J3D/GX rendering. It is not an emulator.
- Unsupported render states, animations, and actor behavior can differ from the game.
- Model import supports rigid and static geometry only.
- The importer rejects skins, skeletal animation, and morph targets.
- Some modern material inputs produce diagnostics instead of GX mappings.
- Object placement requires a safe typed template and known dependencies.
- Graffito does not guess unknown factories or runtime-linked fields.
- Dialogue tools do not edit general events, cutscenes, or SPC scripts.
- Dialogue routing and presentation conditions remain read-only.
- Audio tools use supported retail music and sound data.
- Graffito does not support custom audio import or full JAudio emulation.
- Goop tools edit floor layers only.
- Retail wall and wave layers remain read-only.
- Automated tests do not verify final graphics or gameplay.
- Runtime changes still need manual Dolphin tests.

## Build from source

### Requirements

- Git and Rustup.
- Rust 1.95.0 with Clippy and rustfmt.
- Windows 10 or 11 for the primary desktop workflow.
- A current Vulkan, DirectX 12, or OpenGL driver.
- A legally obtained _Super Mario Sunshine_ extraction.
- Dolphin for playtests.
- `nodtool` for optional command-line disc extraction.

Schema development also needs a local checkout of the SMS decompilation.
Normal builds use the versioned metadata in this repository.

Clone the repository and start Graffito:

```powershell
git clone https://github.com/ryanbevins/Graffito-Editor.git graffito-editor
cd graffito-editor
cargo run --locked --profile fast-release -p graffito-editor
```

Build Graffito without starting it:

```powershell
cargo build --locked --profile fast-release -p graffito-editor
```

The executable appears at:

```text
target\fast-release\graffito-editor.exe
```

The `fast-release` profile uses Thin LTO and incremental compilation. Use it
for normal development.

Graffito uses a configured decompilation checkout when one is available. This
lets schema developers test extractor changes.

Other builds use this versioned metadata file:

```text
crates/sms-schema/generated/object-registry.json
```

Refresh and verify the metadata from a clean decompilation checkout:

```powershell
cargo schema-bundle --decomp-root ..
cargo schema-bundle --decomp-root .. --check
```

For a source archive without `.git`, pass an explicit `--source-revision`.
The generated artifact records that revision and a source-content fingerprint.

Build a fully optimized executable for local distribution:

```powershell
cargo build --locked --release -p graffito-editor
```

The release executable appears at:

```text
target\release\graffito-editor.exe
```

This repository does not publish official binaries yet.

Open a project descriptor directly:

```powershell
cargo run --locked --profile fast-release -p graffito-editor -- `
  "C:\Mods\My Project.sms"
```

## Development and testing

Run the complete code-only repository checks:

```powershell
cargo regression --code-only
```

Add the retail archive tests with an unmodified US game extraction:

```powershell
cargo regression --base-root "C:\Games\SunshineUSExport"
```

The full check covers generated glTF fixtures, formatting, Clippy, workspace
tests, a release build, and source-free archive rebuilds.

With a US extraction, it rebuilds all 108 retail stage archives and compares
their bytes. The tests read assets from the supplied path and never copy them
into the repository.

Run the individual CI commands:

```powershell
cargo fmt --all -- --check
cargo clippy --locked --workspace --all-targets -- -D warnings
cargo test --locked --workspace
cargo build --locked --release -p graffito-editor
```

Report automated test results separately from editor and Dolphin tests.

## Workspace

| Path | Purpose |
| --- | --- |
| `apps/sms-editor` | Desktop UI, viewport, authoring tools, builds, and Dolphin support |
| `apps/sms-cli` | Inspection, conversion, validation, export, and automation commands |
| `apps/xtask` | Repository tests and generated-fixture tasks |
| `crates/sms-authoring` | glTF import, model and collision authoring, and scene merging |
| `crates/sms-formats` | Checked big-endian readers and semantic writers |
| `crates/sms-schema` | Registries and metadata derived from the SMS decompilation |
| `crates/sms-scene` | Scene data, authoring, persistence, validation, and export |
| `crates/sms-render` | Scene, camera, selection, and viewport support types |

## Community and contributions

Join the [Graffito Discord community](https://discord.gg/JhPr3fWuy) for project
updates, questions, development discussion, and test feedback.

Read [CONTRIBUTING.md](CONTRIBUTING.md) before you submit a change. Run the
repository checks before you submit it.

Do not commit extracted game files, retail assets, disc images, managed game
trees, copyrighted project data, or caches.

Report security problems privately through the process in
[SECURITY.md](SECURITY.md).

## Credits

Thank you to the developers and contributors of the
[Super Mario Sunshine decompilation project](https://github.com/doldecomp/sms).

Their research and documentation support Graffito’s work on game formats,
scenes, rendering, and runtime behavior.

## Legal

Graffito Editor is an unofficial fan project. Nintendo does not sponsor or
endorse it.

_Super Mario Sunshine_ and related names are trademarks of their respective
owners. Users must supply their own legally obtained game data.

Graffito Editor uses the [MIT License](LICENSE).
