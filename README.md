# SMS Level Editor

A native Super Mario Sunshine level editor and mod kit built with Rust,
`egui`, and `wgpu`.

The editor opens stages from a legally obtained, extracted copy of Super Mario
Sunshine, derives object knowledge from the matching decompilation project, and
writes changed files to a separate filesystem mod folder. Base game files are
never modified.

## Current Features

- Content browser for discovered scene archives and mounted stage assets
- J3D BMD/BDL/BMT model loading with GX-style TEV material preview
- BTI textures, RARC archives, Yaz0 data, and SMS collision parsing
- Placement object hierarchy, selection, inspector, transforms, and gizmos
- Unreal-style fly, orbit, pan, focus, and variable-speed viewport controls
- Collision overlays, object bounds, validation, undo, and redo
- Decomp-derived object and parameter schema generation
- Filesystem mod export and isolated Dolphin launch integration
- CLI diagnostics for scenes, assets, objects, formats, and renderer previews

Terrain and model topology authoring are intentionally left to external tools
for now. The editor focuses on stage assembly, object editing, faithful preview,
validation, and lossless asset handling.

## Requirements

- Windows 10 or later with a modern Vulkan, DirectX 12, or OpenGL-capable GPU
- Current stable Rust toolchain
- An extracted Super Mario Sunshine game root supplied by the user
- A local checkout of the [SMS decompilation project](https://github.com/doldecomp/sms)
  for schema generation and source-of-truth reference
- Dolphin Emulator for test launches, if desired

No Nintendo game data is included in this repository.

## Build

```powershell
git clone https://github.com/ryanbevins/sms-level-editor.git
cd sms-level-editor
cargo build --release -p sms-editor
```

The desktop executable is written to
`target\release\sms-editor.exe`.

## Run

Launch with the project and stage picker:

```powershell
cargo run --release -p sms-editor
```

Or open a known extracted stage directly:

```powershell
cargo run --release -p sms-editor -- `
  --repo-root C:\path\to\sms-decomp `
  --base-root C:\path\to\extracted-game `
  --stage dolpic0
```

The left-side content browser scans the extracted root for scene archives.
Select a stage there to load its map, collision, assets, and placement objects.

## CLI

```powershell
cargo run -p sms-cli -- scenes --base-root C:\path\to\extracted-game
cargo run -p sms-cli -- open-stage --base-root C:\path\to\extracted-game --stage dolpic0
cargo run -p sms-cli -- validate --base-root C:\path\to\extracted-game --stage dolpic0
cargo run -p sms-cli -- preview-stats --base-root C:\path\to\extracted-game --stage dolpic0 --materials
```

Run `cargo run -p sms-cli -- --help` for the complete command list.

## Workspace

| Package | Responsibility |
| --- | --- |
| `sms-editor` | Desktop application, editor panels, interactions, and GPU viewport |
| `sms-cli` | Automation, extraction, validation, export, launch, and diagnostics |
| `sms-formats` | Big-endian SMS/GameCube binary formats and lossless asset access |
| `sms-schema` | Object and parameter metadata generated from decomp source |
| `sms-scene` | Editable documents, validation, manifests, and mod-folder export |
| `sms-render` | Renderer-facing scene and viewport support types |

## Development

```powershell
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
```

Format work should preserve unknown data byte-for-byte. Renderer work should be
grounded in J3D/GX behavior and the decomp source rather than model-specific
visual exceptions.

## Legal

This is an unofficial fan-made development tool. It is not affiliated with or
endorsed by Nintendo. Super Mario Sunshine and related names are trademarks of
their respective owners. Users must provide their own legally obtained game
data.

Licensed under the [MIT License](LICENSE).
