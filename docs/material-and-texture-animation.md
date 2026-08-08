# The Material Library Tool

What a surface does in Super Mario Sunshine — scrolling water, a gleaming roof,
a flickering sign — is decided in two places, and telling them apart is the
whole basis for a tool that offers presets.

**Material state** lives in the model's MAT3 section. Texgens, texture matrices,
colour channels, TEV stages. It ships inside the BMD and needs no other file.

**Animation** lives beside the model as its own resource: BTK, BTP, BRK, BCK.
The game loads it by name and plays it against the material it targets.

A preset that makes a surface shine edits material state. A preset that makes it
scroll writes an animation file. Same row in the same UI, different output.

Everything below was measured against the retail game rather than assumed;
where something is inference it says so.

---

## The formats

| Format | What it animates | In `sms-formats` |
|---|---|---|
| **BTK** | Texture scroll, scale, rotate per material (`J3DAnmTextureSRTKey`) | read + rebuild |
| **BTP** | Texture pattern — a flipbook swapping which texture a slot samples | read + rebuild |
| **BCK** | Joint animation | read + rebuild |
| **BRK** | TEV register / konst colour over time | read + rebuild |
| **BPK** | Colour animation | read + rebuild |
| **BVA** | Per-shape visibility | **not supported** |

`crates/sms-formats/src/j3d_anim.rs` parses; `j3d_anim_rebuild.rs` writes. The
rebuild side carries `J3dTextureSrtReconstructionMetadata`, so a BTK can be
round-tripped byte-exactly rather than merely re-encoded — the same discipline
the BMD work needed.

Useful entry points:

- `J3dTextureSrtAnimation::parse`, and `J3dTextureSrtBinding::sample(frame)`
  returning a `J3dTextureSrt`. **Sample is the runtime semantics.** A preview
  should feed its output to the renderer as a texture matrix rather than
  reimplementing scrolling, or the tool and the game will disagree and neither
  will be obviously wrong.
- `J3dTexturePatternAnimation::parse`, `texture_index(frame)`.
- `playback_frame(elapsed_seconds)` on both, which is how a frame is derived
  from wall-clock time.

All five are parsed and encoded by `j3d_anim_rebuild.rs`, BRK and BPK included.
Saying otherwise earlier came from reading `j3d_anim.rs`, which only *samples*
joint, pattern and SRT for the preview, and assuming the write side matched. The
harvest settles it: the retail game carries 163 BRK and 123 BPK animations, and
every one of them parses.

---

## Where an animation ships

`StageResourceDocument` already has an `Animation(J3dAnimationRebuildDocument)`
variant, and `StageArchive::insert_resource` can add a resource that does not
exist yet. So the whole path exists:

1. build a BTK in memory
2. wrap it as `StageResourceDocument::Animation`
3. `insert_resource(b"map.btk")`

and it ships through the same archive-edit and Build pipeline the Mask Tool
uses. **No runtime work at all** — no DOL patch, no hook, no class support. The
game loads `map.btk` next to `map.bmd` by convention. Everything the goop wash
needed a hooked stub for, this gets for free.

That convention is also the constraint: an animation is found by name beside its
model. Authoring one for an arbitrary object only animates it if that object's
class looks for one.

### A BTK can be written from scratch

Checked rather than assumed, because it decides whether the library is viable:
`J3dAnimationRebuildDocument` is a plain struct with public fields, and
`stage_archive.rs` already builds one with a struct literal. There is no
parse-then-modify constraint, so a preset writes a new file rather than hunting
for one to copy.

What a scroll needs filling in:

```rust
J3dAnimationRebuildDocument {
    header_tag: J3dAnimationHeaderTag::AllFf,
    layout: J3dAnimationLayout { /* sizes, padding */ },
    section: J3dAnimationSection::TextureSrt(J3dTextureSrtAnimationSection {
        attribute, rotation_shift, max_frame,
        primary, post,              // the SRT tracks
        reconstruction: Default::default(),
        // plus the material name table that says which materials these target
    }),
}
```

`reconstruction` exists only to reproduce a particular creator's byte layout, so
a file authored here leaves it default. That was the other worry and it is not
one.

---

## The reference implementation: Ricco Harbor's fountain

Ricco's fountain is not an object. There is no fountain in `ricco0`'s map
objects, and no `Funsui` effect anywhere in the stage. It is **geometry inside
`map.bmd`**, animated by `map.btk`, which is **1,920 bytes** and animates three
materials:

```
_0009mizu_1        water
_0010sibuki1_1     shibuki -- spray, splash
_0011hunsuimizu_1  hunsui mizu -- fountain water
```

The fountain is three materials but two things you would point at, which only
clustering the triangles into connected bodies shows:

```
_0009mizu_1       body 0:   2 tris  x -12726       y 383    elsewhere entirely
                  body 1:  24 tris  x 1501..2210   y 1623   the basin
_0010sibuki1_1    body 0:  32 tris  x 1511..2200   y 1625   foam, two units above
_0011hunsuimizu_1          72 tris  x 1597..2114   y 1623..2129   the dome
```

So the water surface is two sheets stacked two units apart, and the dome rises
500 units out of them. Named as three, seen as two.

The part that matters for a tool: `_0009mizu_1` is **shared**. Twenty-four of its
triangles are the fountain basin and the other two are a small pool fourteen
thousand units away. An effect dropped on that slot animates both. A material is
not an object, and the only way to know what one covers is to look.

**Naming trap.** Nintendo romanises 噴水 as **`hunsui`** in material names but
`Funsui` in class names (`TEffectPinnaFunsui`, `TEffectBiancoFunsui`). Searching
for one will not find the other — this cost a false "Ricco has no fountain".

Ricco's other animated water, for scale: `sea.btk`, `seaindirect.btk`,
`riccoseapollutions0.btk`, `shimmerlow.btk`, plus the `columwater` model which
carries BMD + BCK + BTK + BRK + BPK **and** its own JPA particles.

### The other kind of fountain

Delfino's and Pinna's fountains are **not** this. They are effect objects —
`TEffectPinnaFunsui`, `TEffectBiancoFunsui`, `effect_id` 424 and 425, from
`src/Enemy/effectObj.cpp` — which sit at a point and emit a **JPA** particle
effect (`map/map/ms_pinna_funsui.jpa` in `dolpic0`). Visually similar, built
completely differently.

`sms-formats` has `jpa.rs` and `jpa_rebuild.rs`, so JPA has read and rebuild
support too, should particle presets ever be wanted.


### Picking a material off the surface

With the tool open, the viewport becomes a material picker. Click any surface and
the slot it belongs to is selected in the panel — its material name, what it is
currently wearing, which mechanism that used, and what it refuses.

That matters because material names are the one part of this the author cannot
guess. `_0011hunsuimizu_1` and `_m_mado_tekari` say what they are only if you
already read Japanese and know the romanisation Nintendo used; `_m52_env` says
almost nothing. Reading a name off a list means matching it to geometry by trial.
Clicking the geometry removes that step entirely.

It works both ways, in two colours that mean two different things:

- **Orange** -- the slot you have selected. Every surface using that material
  lights up, so a material shared across a model is visible as shared before
  anything is dropped on it. One small part and half the stage look identical in
  a list and nothing alike on the model.
- **Purple** -- what a drop would land on. While an effect is being dragged over
  the map, the surface under the cursor takes the colour if it would receive the
  effect, and does not if it would be refused. The answer arrives before the
  mouse is released rather than as an error afterwards.

Keeping them distinct matters because they answer different questions: orange is
*where is this material*, purple is *what am I about to change*. A single colour
doing both leaves the author guessing which one they are looking at.

The map model makes this sharper still. Delfino's `map.bmd` has 41 materials and
Ricco's has 32,259 triangles across its own set; picking is the only sane way to
find the one under a particular roof.


### What this map already has

Open the tool on a stage and the first thing it shows is what is already there:
every material that carries an animation today, read out of the stage's own BTK
and BTP files, and every material with a shine already configured in MAT3.

Ricco's map answers with three animated materials in a 1,920-byte `map.btk`,
Delfino's with specular on nearly all 41 of its materials and reflection on six.
That is the baseline an author is adding to, and it is invisible otherwise --
nothing in the editor currently says "this surface is already animated", so the
first sign of a clash is two effects fighting in the game.

It is also the fastest way to learn the vocabulary. A preset library says what
Nintendo did in the abstract; a survey of the open stage says what Nintendo did
*here*, next to the geometry, in names the author can click on.

Each row should carry what the effect covers, not just that it exists:

- how many separate bodies of geometry share the material, and where they sit
- which file the animation lives in, and how many tracks that file holds
- whether the material is also used somewhere the author is not looking

The last one is what stops a Surface effect on the fountain basin quietly
animating a pool at the other end of the harbour.


### The library is harvested, not written

The effects come from the game. Walk every `scene/*.szs`, read each model with
its BTK and BTP files, and record which materials are animated and how -- an
index built ahead of time, the way `object-registry.json` already is, around
forty archives done once and shipped with the editor.

That beats a hand-written preset list on every count. It cannot drift from retail
because it *is* retail. It covers effects nobody thought to write a preset for.
And its parameters are measured rather than guessed -- "slow drift" becomes the
actual units per frame that Nintendo shipped.

What stays hand-written is the arrangement: which harvested materials belong to
one concept, and what to call them in words an author recognises.
`_0011hunsuimizu_1` is a fine key and a poor label. So the categories stop being
the library and become a way to organise a harvest, grouped partly by the
vocabulary the names already give -- `mizu`, `sibuki`, `hunsui`, `env`, `tekari`
-- and partly by hand.

It also means an author can apply an effect from any stage to any stage. A Ricco
fountain on a Pianta rooftop is the same operation as a Ricco fountain in Ricco.


### What the harvest actually found

Run over 108 archives, nothing skipped:

```
pattern   8449   flipbooks
scroll     945   texture scroll, scale, rotate
register   163   TEV register colour over time
colour     123   material colour over time
```

Two things worth taking from that. Flipbooks dominate by an order of magnitude,
so a library that treats them as an afterthought has the emphasis backwards. And
9,680 effects touch only **204 distinct materials** -- the same surfaces recur
across stages, so deduplicating by material name turns the harvest into a
browsable library rather than a haystack. Ricco's fountain trio appears eighteen
times, once per archive carrying that map; `_mizubashira2`, *water pillar*, turns
up 146 times.

---

## Shine: two mechanisms that look alike

### Specular lighting

GX computes a highlight per vertex from the half-angle. Configured on a colour
channel:

```
light_mask:          0x04
diffuse_function:    1     GX_DF_SIGN
attenuation_function: 0    GX_AF_SPEC
```

**This is what retail scenery uses.** Measured on `dolpic0`'s `map.bmd`: of 41
materials, essentially all of them are specular — walls (`_m00kabe`,
`_m06kabe`, `_m52_white_wall`), floors (`_m00yuka`), sand (`_m00suna`), the
lighthouse (`_m52_todai`), the stairs (`_m52_kaidan`).

The editor already authors this: `conservative_specular_tev_stage()` in
`apps/sms-editor/src/model_assets.rs` sets the channel above, adds a second TEV
stage on a free slot, and takes konst selectors `0x0c` / `0x1c`. It refuses
materials whose program is not the canonical conservative base, or that have no
free second stage.

**Counting trap.** `color_channel_count` counts *channels*, but the array holds
four *infos* — COLOR0, ALPHA0, COLOR1, ALPHA1. A two-channel material still uses
index 2, which is exactly where a specular config lives. Iterating with
`.take(color_channel_count)` silently hides every specular material in the game.
That produced a confident, wrong "retail uses no specular anywhere".

**Per-vertex.** GX lighting is computed at vertices and interpolated, so the
highlight is a broad wash rather than a tight glint, and its quality depends on
tessellation. A big flat roof of two triangles gets a flat sheen that pops as
the camera turns. This matters most for imported models: geometry authored for
a modern normal-mapped renderer is usually far less tessellated than SMS
geometry, which is the wrong density for vertex-lit specular.

### Environment mapping

A texgen sourced from the vertex **normal** (`GX_TG_NRM`, source `0x01`) through
a texture matrix, sampling a shine texture. No lights involved, so it works
anywhere.

Measured on `dolpic0`: six materials, and they name themselves — `_m52_env`,
`_m_dokan_g_env`, `_m_dokan_r_env`, `_m_env`, `_m_mado_tekari` (*mado* window,
*tekari* 照り gleam), `_m_underpass`. All six are **also** specular. Ricco has
its own: `_h_env_1`, `_m_ship2`, `_m_tras0`, plus `_m_underpass` using
`GX_TG_POS`.

So retail's look is specular across scenery generally, with reflection mapping
added on the few surfaces that want a highlight that tracks the camera.

**The importer does neither.** `crates/sms-authoring/src/import.rs` writes
exactly one texgen per material:

```rust
gx.tex_gen_count = 1;
gx.tex_gens[0] = Some(GxTexCoordGen {
    function: 1,
    source: 4 + binding.tex_coord,  // GX_TG_TEX0 + n, a stored UV
    matrix: 60,                      // identity
});
```

Source is always a stored UV, matrix always identity. Environment mapping is
those two fields set differently — `source: 1` and a texture matrix in env-map
mode. `GxTexMatrix` already models `projection` and `mapping_mode` (bit seven
being the Maya convention), so the format layer can express it; nothing
authors it.

---

## Texgen source values

Worth having to hand, since these are what a probe prints:

| Value | Source |
|---|---|
| `0x00` | `GX_TG_POS` |
| `0x01` | `GX_TG_NRM` |
| `0x02`–`0x03` | binormal, tangent |
| `0x04`–`0x0B` | `GX_TG_TEX0`..`TEX7` — a **stored** UV set |
| `0x0C`–`0x12` | `GX_TG_TEXCOORD0`.. |
| `0x13` | `GX_TG_COLOR0` |
| `0x14` | `GX_TG_COLOR1` |

A texgen whose source is `0x04 + n` reads stored slot `n`. Anything below `0x04`
is computed from geometry, which is why such a material has no UV to inspect.

---

## Proposed tool

Rows are the material / texture slots of the authored stage model. Each row
takes a preset:

- **Scroll** — direction and rate. Writes a BTK.
- **Flipbook** — a texture list and a rate. Writes a BTP.
- **Shine** — the existing specular preset. Edits MAT3, no file.
- **Reflect** — env-map texgen. Edits MAT3, no file. *Not yet implemented.*

The value is in naming the effects, not in exposing raw tracks. Ricco's entire
fountain is 1,920 bytes and mostly a couple of keys on translate V — a generic
keyframe editor would be more work and less useful.

Accumulate every row's animation into **one BTK per stage model**, inserted
beside it. That is exactly retail's structure — three materials in one file —
rather than an invention.

### Things that will bite

- **One convention, applied everywhere.** BTK is UV space. The Mask Tool needed
  the same rule in four separate places and got it wrong in each independently.
  The rule that worked: *flip what a projection produced; never flip what a
  model stores.* Decide it once here.
- **Preview through `sample(frame)`**, not a reimplementation.
- **Shared models.** A stage archive carries its own copy of each actor folder,
  but two classes can share one model — Cataquack ships as `TPoiHana` and
  `TPoiHanaRed` wearing the same BMD. Anything authored into a model affects
  every actor wearing it.
- **Editing `map.bmd` touches shared geometry.** Authoring a new BTK against
  your own model is far less invasive than retiming Ricco's fountain.

---

## The shape of it: a preset library

Not a material editor. A **library of named effects**, ordered by what a surface
is meant to be, dropped onto a material slot.

Everything here targets a material. Shine is material state, written into MAT3;
scroll and flipbook are animation files -- but those are keyed by material name
too, so a BTK track says "this material's texture matrix, over time". Ricco's
fountain is three materials named in one file. One concept, two mechanisms. The author picks
"water, falling" — not "translate V, -0.04 per frame, on texgen 1".

Every preset is the same underlying record: which mechanism it needs (BTK, BTP,
or MAT3 alone), what it requires of the material to apply cleanly, and a small
set of parameters worth exposing. That uniformity is what makes drag-and-drop
work, and what lets a preset refuse rather than corrupt.

Each category below names the retail material it was taken from, so a preset is
a reproduction rather than an invention.

### Water

| Preset | Does | Mechanism | Retail reference |
|---|---|---|---|
| **Surface** | Slow two-axis drift | BTK | `sea.btk` |
| **Falling** | Fast downward scroll | BTK | `_0011hunsuimizu_1` (Ricco fountain) |
| **Splash** | Faster, shorter loop for spray at the base | BTK | `_0010sibuki1_1` |
| **Shimmer** | Small-amplitude scale/rotate wobble | BTK | `shimmerlow.btk` |
| **Refraction** | Second layer scrolling against the first | BTK | `seaindirect.btk` |

Falling and Surface are the same node graph with different rates, which is the
point: the parameter that matters is *speed and direction*, and the preset
supplies everything else.

### Shine

| Preset | Does | Mechanism | Retail reference |
|---|---|---|---|
| **Lit** | Specular highlight from the stage's lights | MAT3 | `_m00kabe`, `_m52_todai` |
| **Reflective** | Highlight that tracks the camera, no lights needed | MAT3 | `_m_env`, `_h_env_1` |
| **Gleam** | Reflective, masked to a bright band | MAT3 | `_m_mado_tekari` |

Lit exists today as `conservative_specular_tev_stage()`. Reflective and Gleam
are texgen and texture-matrix work that nothing authors yet.

### Surfaces that move

| Preset | Does | Mechanism | Retail reference |
|---|---|---|---|
| **Conveyor** | Steady scroll along one axis | BTK | — |
| **Lava** | Slow scroll with a second layer drifting against it | BTK | — |
| **Clouds** | Very slow drift, usually on sky geometry | BTK | `sky.btk` |
| **Pollution** | Goop crawl | BTK | `map/pollution` |

### Flipbooks

| Preset | Does | Mechanism | Retail reference |
|---|---|---|---|
| **Blink** | Swap between two or three textures on a beat | BTP | eye materials |
| **Sign** | Cycle a list at a fixed rate | BTP | — |

Ricco alone carries 71 BTP files against 14 BTK, so flipbooks are more common in
retail than scrolling is. Worth not treating as an afterthought.

### Colour over time

| Preset | Does | Mechanism | Retail reference |
|---|---|---|---|
| **Pulse** | A colour breathing over time | BRK | 163 in retail |
| **Tint** | A colour held, then changed | BPK | 123 in retail |

---

### What a preset has to declare

The specular preset already establishes the pattern worth copying: it checks the
material has the canonical conservative base program and a free second TEV
stage, and refuses otherwise. Applied generally, a preset declares:

- **the mechanism** — BTK, BTP, or MAT3 only
- **what it needs** — a free TEV stage, a free texgen, a stored UV set, a
  texture of a particular shape
- **what it takes** — rate, direction, amplitude, a texture list
- **what it conflicts with** — two scrolls on one slot are one scroll

and the drop is refused with the reason on the material, rather than applied
half-way. A slot that cannot take a preset should say so before it is dropped on,
the way the reimport buttons now grey out with their reason.

### Where it accumulates

Every scroll and flipbook in the stage lands in **one BTK and one BTP per
model**, written beside it. Adding a preset to a second material adds a track to
the existing file rather than making another. That is retail's structure — one
`map.btk` covering the fountain's three materials — and it means the number of
files stays constant however many surfaces are dressed.

MAT3 presets write into the model itself and produce no file at all.

---

## Tooling that already exists

Written during the goop work, in the session scratchpad, and worth keeping:

- **`vtscan.py`** — which classes inherit a function, by finding its address in
  vtables. Answers "does this class override that?"
- **`callscan.py`** — does function A call B. Scans A's whole body to the next
  symbol, not to the first `blr`, because a function with an early return would
  otherwise look like it never reaches its later calls.
- **`dispatchscan.py`** — virtual call sites through a given vtable slot.
- **`emitscan.py`** — who sends a given message id.
- **`sigfind.py`** — a function's opening instructions and the shortest unique
  signature within them, excluding `bl` (whose displacement moves between
  builds) but allowing relative branches (which do not).

All read `dol-proto/us.map` and `SMS-Extracted/sys/main.dol`.

**PowerPC note that cost real time:** this build dispatches virtual calls
through **LR** (`mtlr` + `blrl`, 5,821 sites), not CTR. There are **zero**
`bctrl` instructions in the entire DOL. A scan looking for `bctrl` finds nothing
and looks like a wrong assumption about the code rather than a wrong assumption
about the encoding.
