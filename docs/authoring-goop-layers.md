# Authoring a goop layer onto an actor

Notes from giving BossPakkun a washable goop layer he never shipped with, and
getting the runtime to match the Mask Tool exactly. Most of the work was not
writing the layer — it was finding the seven separate reasons the result did
not match. Each had a distinct look, so the table at the end is the fastest way
in if a new actor misbehaves.

## What a goop layer actually is

Retail wires washable goop as a **TEV comparison**, not as a shader or an
effect. On a wired actor such as StayPakkun the material carries:

- a **mask texture** (a `polmask`), small and single-channel;
- a **stored goop coordinate** — a second texture coordinate set in the vertex
  data, not a generated one;
- a **comparison stage** testing the mask against a konst register;
- **stages after it** that put the coating where the comparison says.

Reading StayPakkun's own records is what settled the details. His comparison is:

```
color_args  a=TEXA  b=KONST  c=ONE  d=ZERO
color_op    0x0a (GR16_GT)   bias 3   scale 1   -> register 2
```

so `C2 = (mask > K0_A) ? 1 : 0`, and the stage after it adds the coating where
`C2` is zero. **The coating therefore shows where `mask <= K0_A`**, and the
konst rises with coverage. That matches the decomp, which drives the same konst
from hit points — full health, fully coated — and it is the opposite of what
most people assume from the name "threshold".

The bias of three and scale of one are how GX encodes a comparison. They were
copied from a shipping material rather than derived, which was the right call:
deriving them would have been guesswork.

## Why the coordinate must be stored

This is the single most important thing.

A **generated** coordinate (texgen sourced from position, through a texture
matrix) is handed the vertex position **in whichever joint's space that vertex
lives**. An actor whose parts sit in different joints — which is most of them —
cannot be served by one matrix, because the material carries exactly one.

The symptom is unmistakable once seen: the coating **smears and swirls** across
the model, worst on large multi-joint parts, and matches the preview at no
coverage value. It is not an offset that can be tuned out.

A **stored** coordinate travels with the vertex, so it does not care which joint
the vertex belongs to. That is why retail stores it, and why
`store_front_projection_texcoord` exists.

Note the editor's own renderer hides this problem: `j3d.wgsl` receives vertices
with joint transforms already baked in (`var world_position = input.position`),
so a position-sourced texgen means *world* space there and looks correct. The
GPU on console does not. **The stage viewport agreeing is not evidence.**

## The failure modes, in the order they were found

### Geometry disappears, leaving only interior surfaces

Two different causes produced this, and it is worth telling them apart because
the fixes are unrelated.

**Cause one: the comparison stage zeroed the alpha.** StayPakkun's comparison
carries alpha arguments of all `ZERO`, writing an alpha of zero into the shared
register. In his material that stage runs *first* and later stages rebuild the
alpha before the alpha test sees it. An **appended** layer runs last, so the
zero survives, every surface with an alpha test is discarded, and what remains
is whatever is not gated — BossPakkun's mouth interior, seen from outside.

Fix: the appended comparison passes the previous alpha through (`d = APREV`).

**Cause two: the model grew too large to load whole.** The TEX1 repack gave
every texture record its own copy of image data that retail *shares* between
records, roughly doubling the section — 135 KB to 288 KB. The actor then failed
to load fully and rendered as the same mouth interior.

This one was mistaken for a texture-memory limit and chased through a smaller
coating and a colour/mask split, neither of which helped, because the
duplication dominated the size either way. **If lowering the coating resolution
does not change it, look at the model's total size, not the texture's.**

Fix: `canonicalize_texture_layout` places each distinct run once. Note that a
**mip chain must be shared whole or not at all** — sharing a single level out of
the middle scatters the chain and later levels are read from the wrong place.

### Lit surfaces blow out to white, dark ones look right

The stage applying the coating **adds** it to what is underneath. That is
correct where it sits in the material it was copied from, but appended last it
saturates anything already bright. The giveaway is that the coating reads
correctly *inside a mouth or under an overhang* — anywhere the surface behind it
is near black, where an add and a blend come to the same thing.

Fix: compose with two adds rather than one blend —

```
clear:  ZERO + ZERO*(1-C2) + CPREV*C2   ->  surface * C2
apply:  CPREV + goop*(1-C2)             ->  surface*C2 + goop*(1-C2)
```

A single blend stage (`a=TEXC b=CPREV c=C2 d=ZERO`) is arithmetically identical
and **made every surface vanish**. Colour arguments cannot hide geometry, so
something about that stage is still not understood. Until it is, use the two
adds; they are known to render.

### Coverage below about a third hides everything

The wash level lives in a konst register's alpha. If the layer claims a register
the material's **own** stages already read, lowering coverage lowers whatever
that stage takes from it — and once it falls under the material's alpha-compare
reference, every surface is discarded. High coverage masks the bug entirely.

Fix: scan the material's existing konst selectors and claim a free register.
`add_goop_layer` reports which one it took.

### The pattern is mirrored, or matches only at full coverage

Two independent polarity bugs, both worth checking:

**Vertical flip.** The stored coordinate puts `v = 0` at the model's foot, and
GX reads `v = 0` from the **first row written**. Authoring the coating with
`v = 1 - y/res` has the model read the goop map upside down.

**Comparison direction.** The preview coated where `mask > threshold`; the wash
coats where `mask <= level`. These agree at full and empty coverage and select
**complementary halves of the mask** everywhere between — so a mid value looks
wrong while the extremes look perfect. The game's direction is correct; the
preview was changed to match.

### Thin strips, coordinates jumping between neighbouring vertices

The coordinates are posed by walking each shape and the draws it names, but were
written back by walking the **draw array in its own order**. Those are not the
same order, so a vertex receives the coordinate posed for another. The counts
agree either way, which is why every numeric check passed — it shows only in the
picture.

Fix: write in the same traversal the posing used.

### Twisting confined to one part of the model

The last and subtlest. A packet's matrix table lists the matrices its vertices
reach, and an entry of `0xFFFF` means **"this slot keeps the matrix an earlier
packet loaded into it"** — a slot saying nothing, not a slot naming nothing.

Two wrong readings were tried before the right one:

1. treating it as *absent* left those vertices in their joint's local space;
2. reaching **sideways** to the nearest valid entry in the same packet handed
   them a matrix belonging to a different joint.

Both pose a minority of vertices somewhere the model never puts them, and the
projection bends around exactly those parts. On BossPakkun it was his lower
body — 150 of 1714 vertices, 8.8%, and everything else was already perfect.

Fix: hold each slot's matrix across packets, updating only when a packet names a
new one.

## Finding a layer an actor already has

Before authoring one, check whether the actor carries a wash already — and do
not use the preview's per-triangle mask fields to decide. `mask_tex_coords` and
`mask_texture_index` come from a parser heuristic that **never fires** for
StayPakkun, BossGesso or HamuKuri: they read `0` of `N` triangles on all three,
which is what made every early attempt conclude "no authored mask" and coat them
with a borrowed one.

The binding that cannot be absent is the wash itself. Scan the material's TEV
stages for a comparison — `color_op >= 8 || alpha_op >= 8` — and read its order:

- its **texture map** resolves through `material.texture_indices[map]` to the
  mask the actor ships;
- its **coordinate slot** is the set the wash reads.

Measured this way, the three wired actors give up their layers immediately:

| Actor | Mask | Coordinate | Coverage |
| --- | --- | --- | --- |
| StayPakkun (`pakun.bmd`) | `H_ma_polmask1_i4_32` | stored UV1 | all 738 triangles |
| HamuKuri (`hamukuri/default.bmd`) | `H_ma_polmask1_i4_hamu` | stored UV1 | 132 cap triangles only |
| BossGesso (`bgeso_body.bmd`) | `N_bgeso_yogore2` 64² | coord 2, generated from stored UV1 | 578 of 682 |
| BossPakkun | none — no comparison stage at all | — | — |

HamuKuri is worth noting: his wash lives on his **cap material alone**, which is
why a whole-body coating looks wrong on him. On a wired actor, a surface without
the layer should stay clean rather than fall back to a projection.

BossGesso shows the other trap. His comparison names coordinate **2**, but his
material *generates* that coordinate from stored UV1 through a texture matrix.
Looking for a stored set at index 2 finds nothing. Follow the chain: comparison
coordinate → `material.tex_gens[coord].source` → if the source is `4..11` it is
stored set `source - 4`.

## Getting the actor into the tool

The Mask Tool draws through the stage viewport's own renderer rather than a
second one. That decision came late and only after a long detour: a hand-written
CPU rasterizer will keep diverging from the real pipeline, and every actor tried
against it exposed another gap — texmap slot resolution, generated coordinates,
the alpha half of the TEV, wrap modes. If the stage viewport draws an actor
correctly, use it.

Isolating one actor is cheap: clone the stage's `ModelPreview` and
`triangles.retain(|t| t.model_index == index)`. Several things then have to be
right, and each has its own symptom.

**The camera basis must match `camera_frame()`.** It builds
`right = [-cos(yaw), 0, sin(yaw)]`. Deriving `[+cos(yaw), ...]` instead mirrors
the view, which flips every triangle's winding, so backface culling keeps the
**interior** and discards the surfaces facing you. The model renders inside out,
mouths lose their inner faces, and the framing skews.

**The stage holds actors at their placed position.** The loaded BMD's bounds are
in the model's own space, so aiming a camera with them points it at the stage
origin — the actor comes out tiny and off to one side, with any overlay drawn
around the model's own centre sitting somewhere else entirely.

**A model index carries more than the body.** Effect meshes ride along —
PoiHana's sleep Zs, billboards, particle quads — and they float away from the
actor, inflating both the bounds the camera frames and the span a projection
normalises across. Filter by `render_layer`, `billboard` and `particle_type`,
then trim what remains by distance from the dense cluster of body triangles.

**Strip the animation bindings.** The stage animates some models at draw time,
so the renderer poses the actor away from the geometry an overlay was computed
from. Clearing `animated_models`, `animated_flags`, `rotating_models` and
`level_transform_models` on the isolated copy keeps authoring on the rest pose.

**Some actors are not in the stage preview at all.** HamuKuri is spawned by his
manager and loads per spawn, so `object_model_indices` has no entry for him. A
nearest-model fallback is a trap: every placed object sits inside the map's
bounds, so it will happily return a slab of terrain. Build a renderable preview
from the model the tool itself loaded instead.

## Growing MAT3 and TEX1

A model that never carried goop has nowhere to put the records it needs.
BossPakkun's MAT3 had **twenty spare bytes** against roughly a hundred and
thirty required, and his TEX1 thirty two against a whole texture. So before any
of the above could happen, the sections had to be able to grow.

### Why they could not

`encode_section` allocates `vec![0; section.declared_size]` — the size the file
was parsed with — and every encoder writes back at the offsets the records
carry. That is deliberate: it is what lets an untouched retail model round-trip
byte for byte. It also means **anything that grows must be repacked first**.

Both encoders turn out to be purely offset-driven, so repacking is a walk:

- `encode_mat3` writes the count at `0x08`, thirty offsets at `0x0c`, the init
  records at `offsets[0] + i * 0x14c`, the name table at `offsets[2]`, and each
  table at its own `table.offset`.
- `encode_tex1` writes the count at `0x08`, `header_offset` at `0x0c`, the name
  table offset at `0x10`, a `0x20`-byte header per texture, then the image and
  palette blocks at their absolute offsets.

`canonicalize_material_layout` and `canonicalize_texture_layout` recompute those
offsets and the section size from the data. They follow the pattern
`canonicalize_geometry_layout` already used for VTX1 and SHP1.

### Two things that will bite

**`scalar_array_len` returns bytes, not elements.** It multiplies by the element
width internally. Multiplying again gives every table two to four times its real
size, which silently corrupts the layout — `CullMode` went in as three `u32` and
came back as twelve. Cost about an hour to find.

**A table's length is not stored.** The parser infers it from the distance to the
next offset, then trims a suffix matching retail's padding message, the literal
string `"This is padding data to alignment."`. Alignment gaps written as zeros
are therefore read back as **data**. Every gap the repack introduces must carry
that message — see `record_alignment_padding`.

The offset slot order is fixed and given by `MATERIAL_TABLE_KINDS`: slot zero is
the init records, slot one the remap, slot two the names, and slots three
onward the tables in the order that array lists them. Slots for absent tables
are zero.

### Appending to a material

`append_material_bytes`, `append_material_u16` and `append_material_count` push
onto the relevant table and return the **element index**, reusing an identical
element already present rather than duplicating it. Nothing existing is
disturbed: every record is appended and only the named material's own indices
are repointed, so other materials keep their entries.

The material init record is a set of index arrays into those shared tables, and
this is where the sharpest trap lives:

> **A TEV order names the *slots a material binds*, not the table entries behind
> them.** Its coordinate field is the texgen slot and its map field is the
> texture map slot. Writing the table index there resolves to a neighbouring
> coordinate and looks almost right — which is exactly how it presented.

The same distinction applies to `texture_number_indices` and
`tex_coord_indices`: those arrays are indexed **by slot** and hold **table
indices**.

Counts live in their own tables. `tex_gen_count_index` and
`tev_stage_count_index` are indices into `TexGenCount` and `TevStageCount`, so
raising a count means appending a new count value and repointing, never editing
in place — the value is shared with every other material that happens to use it.

### Record layouts, as confirmed against retail

Taken from the parser rather than from documentation, and worth trusting over
memory:

**TEV stage, twenty bytes.** `[1..5]` colour args a,b,c,d; `[5]` colour op;
`[6]` bias; `[7]` scale; `[8]` clamp; `[9]` register; `[0x0a..0x0e]` alpha args;
`[0x0e]` **alpha op**; `[0x0f]` bias; `[0x10]` scale; `[0x11]` clamp; `[0x12]`
register. Bytes `[0]` and `[0x13]` are `0xff`.

The alpha op is at `0x0e`, **not** `0x0b`. Reading it at eleven picks up an
alpha argument instead, and a konst-register scan built on that only works by
luck when the material happens to compare on the colour side.

**TEV order, four bytes.** `[0]` coordinate slot, `[1]` map slot, `[2]` colour
channel, `[3]` padding.

**Texgen, four bytes.** `[0]` type — `1` is `MTX2x4`, `10` is `SRTG`; `[1]`
source — `0` is position, `4..11` are stored sets `TEX0..TEX7`, `19` and `20`
are the lit colour channels; `[2]` matrix — `0x1e` is `TEXMTX0` stepping by
three, `0x3c` is identity; `[3]` padding.

**Texture matrix, one hundred bytes.** `[0]` projection, `[1]` mode with the
top bit meaning Maya, `[4..0x10]` centre, `[0x10..0x18]` scale, `[0x18]`
rotation, `[0x1c..0x24]` translation, `[0x24..0x64]` a four by four effect
matrix. A stored coordinate needs none of this — the layer as it stands reads
the vertex data through the identity matrix, as StayPakkun does.

**Konst selectors.** Colour and alpha selectors sit in the init record at
`0x9c + stage` and `0xac + stage`. Values `12..15` are the four registers'
colour triples; `16..31` are single channels, index `(sel - 16) & 3` and channel
`(sel - 16) >> 2`. So **`0x1c` is K0's alpha**, and the other registers' alphas
follow at `0x1d`, `0x1e`, `0x1f`.

### Writing a stored coordinate

Adding a coordinate set touches vertex data, which is the one place a mistake
corrupts rather than merely looks wrong. It is less work than it sounds because
display lists are already decoded into typed operands:

1. Append the attribute to **every** vertex descriptor set, before the
   terminator, as `GX_INDEX16`.
2. Append one `Index16` operand to every vertex, in the **same traversal** the
   coordinates were posed in.
3. Declare the array in the VAT — attribute `13 + slot`, two components,
   `f32` — keeping the terminator last.
4. Push the coordinates as an `f32` array on VTX1.
5. `canonicalize_geometry_layout`.

Each vertex **occurrence** gets its own entry rather than sharing by position
index, so a position used by two joints cannot pull two projections into one
slot.

Posing a vertex needs the matrix its packet binds: `matrix_table[group
.first_matrix + slot]` indexes the draw matrices from `rest_pose_draw_matrices`,
where the slot comes from the vertex's `PNMTXIDX` operand divided by three, or
zero when the descriptor carries none. Then hold each slot across packets, as
described above.

## Keeping the model small

Size is not a tidiness concern here — it is a correctness one. An actor that
grows too far stops loading whole, and the symptom looks like a rendering bug
rather than a size one. BossPakkun's numbers through the work, all measured:

| State | Size |
| --- | --- |
| Untouched | 134,912 |
| With the stored goop coordinate | 152,096 |
| Layer added, TEX1 duplicating shared runs | 288,416 |
| Same, with colour and mask split into two textures | 292,576 |
| Layer added, shared runs placed once | **156,320** |

The split is the instructive row. It cut the coating from one RGBA8 texture to a
compressed colour map plus a tiny intensity mask — about a thirtieth of the
texture data — and the model still came out **larger**. That is what ruled out a
texture-memory explanation and pointed at the repack: the duplication dominated
either way.

### Share what retail shares

Retail texture records commonly point at the **same** image data — several
records naming one run is normal. Giving each its own copy on the way out
roughly doubles TEX1 and is where nearly all the growth came from.

`canonicalize_texture_layout` keys each run by its bytes and places it once,
letting the records that shared it point at the same offset again, exactly as
they did. Two rules matter:

- **Share a mip chain whole or not at all.** The levels are read as one run, so
  deduplicating a single level out of the middle scatters the chain and every
  level behind it is read from the wrong place. The round-trip test caught this
  on `biancoriver.bmd` immediately.
- Palettes dedupe independently, being single blocks.

### Reuse table elements too

`append_material_bytes` and `append_material_u16` return the index of an
identical element already in the table rather than appending a duplicate. On a
model where several materials take the same layer this matters: the TEV stages,
the orders and the texgen record are byte-identical between them, so they cost
one entry each no matter how many materials use them.

The same applies to the goop texture itself — `append_texture` returns the
existing index when the name is already present, so a coating shared across an
actor's materials is stored once.

### Choose the format, not the resolution

Format dominates. At 256 square:

| Format | Cost | Notes |
| --- | --- | --- |
| RGBA8 | 256 KB | colour and mask in one texture forces this |
| RGB5A3 / RGB565 | 128 KB | no compression artefacts |
| CMPR | 32 KB | four by four blocks; visibly wrecks smooth goop swirls |
| I8 | 64 KB | intensity only — right for a mask |

Retail keeps the coating and the mask apart precisely so each can take a cheap
format: a `polmask` is a single intensity plane at 32 square, about a kilobyte.
That separation is the right structure even though it was not what fixed the
size problem here.

CMPR is worth a warning: it compresses in four-by-four blocks and turns smooth
chocolate swirls blocky. If a compressed coating looks worse than an
uncompressed one at half the resolution, prefer the smaller uncompressed one.

### Only pay for the vertices you use

The stored coordinate gives **each vertex occurrence** its own entry rather than
sharing by position index. That costs more than sharing would — 1714 entries for
BossPakkun — but it is what makes a position used by two joints safe, and eight
bytes per occurrence is cheap against the alternative of a wrong projection.

## The numbers, and how they line up

Every value in this pipeline is a byte or a normalised float, and several of the
bugs above were conversions between them going the wrong way.

**Coverage to konst.** The slider runs `0.0..=1.0`; the wash level written into
the model is `coverage * 255`, rounded. Full coverage is `255`, clean is `0`.
Emphatically **not** its complement — writing `(1 - coverage) * 255` bakes a
clean model at full coverage, which is how that bug presented.

**Mask to coating.** The mask is a byte per texel. The comparison coats where
`mask <= level`, so a texel of `200` clears once the level falls below two
hundred, while one of `40` survives almost to the end. The brightest mask values
therefore clear **first**. Inverting the wash flips the value (`255 - v`) rather
than the comparison, because a baked model carries its comparison fixed once
written — flipping the comparison would change the preview only.

**Position to coordinate.** The projection is a bounds-normalised front
projection over the **posed** model:

```
u = (x - min_x) / (max_x - min_x)
v = (y - min_y) / (max_y - min_y)
```

clamped to `0..=1`, with a degenerate axis collapsing to `0.5`. Bounds are taken
over the posed vertices the display lists actually name, not over the model's
declared bounds, so unreferenced or stray geometry cannot widen them. Retail's
own goop UV measures out to exactly this — StayPakkun's sits on the unit square
with front and back sharing it.

**Coordinate to texture row.** GX reads `v = 0` from the **first row written**,
and the projection puts `v = 0` at the model's foot. So row zero of the coating
must hold what the preview samples at `v = 0`. Authoring with `v = 1 - y/res`
inverts it.

**Texture dimensions.** Sides want to be powers of two; the coating is taken to
the goop map's own resolution and the mask keeps its native size, usually
thirty-two square. Format matters more than dimensions for size: one RGBA8
texture at 256 square is a quarter of a megabyte, which is why retail keeps
colour and mask apart, each in a format that suits it.

**Tolerances used in the tests**, so a future change knows what is considered
agreement:

- posed positions are keyed at eighth-unit resolution (`round(v * 8)`) when
  comparing against the preview, which is tight for model units in the hundreds;
- a stored coordinate must land within `0.02` of the projection of its own
  triangle's corners;
- the projection must span more than `0.8` of the unit square on both axes,
  which catches a projection that collapsed;
- posed vertices are allowed to differ from the preview by under fifteen per
  cent, to permit billboarded geometry; BossPakkun now sits at zero.

## The tests, and what each is for

These caught more than reasoning did, and each earned its place.

**`relayout_preserves_every_model_in_an_archive`** — repacks every model in a
stage archive and compares what the sections *mean*: init records, table
contents, names, texture pixels. Ran over 663 retail models. It caught mip
chains being scattered by deduplication, and the fact that a table's length is
inferred from the gap to the next offset then trimmed of a suffix matching
retail's padding string, so **alignment gaps must carry that string** or they
read back as data.

**`posing_matches_the_preview`** — compares every posed vertex against the
preview's known-good pose. Went 150 to 0 when the matrix-slot fix landed, and
confirmed it before anyone had to look at a screenshot. Billboarded geometry is
allowed to differ: it is turned to face the camera when drawn, somewhere a
stored coordinate cannot follow.

**`storing_a_goop_coordinate_spans_the_model`** — checks every triangle carries
the set, that it spans the unit square, that geometry is unmoved, and — most
usefully — that each triangle's coordinate is the projection of **its own**
posed corners. Range and coverage pass even when coordinates land on the wrong
vertices; only this catches that.

**`authoring_a_goop_layer_gives_a_model_a_wash`** — authors onto a model with no
comparison at all and reads it back through the ordinary preview path: the
comparison present, the coordinate sourced from the stored set, the mask
sampled, the level surviving in the register the layer claimed.

## Quick diagnosis

| What you see | Look at |
| --- | --- |
| Only interior surfaces render | Alpha zeroed by the comparison stage, **or** the model grew too large to load |
| Coating looks blocky in four-by-four patches | CMPR compression; use a smaller uncompressed format |
| Lit surfaces blow out, dark ones fine | Coating is added rather than composed |
| Everything vanishes below ~0.3 coverage | Layer claimed a konst register the material already reads |
| Pattern mirrored top to bottom | Coating authored with `v` flipped |
| Matches at 0 and 1, wrong between | Preview and wash comparing in opposite directions |
| Thin strips, coordinate jumping per vertex | Coordinates written in a different order than they were posed |
| One part twists, rest correct | `0xFFFF` matrix slots — hold across packets |
| Smearing everywhere, no coverage matches | Coordinate is generated rather than stored |
| Model renders inside out, mouths hollow | Camera basis mirrored against `camera_frame()` |
| Actor tiny and off to one side | Camera aimed with the model's own bounds, not its placed ones |
| Coating sits away from the model | Renderer posing an animated actor away from the rest pose |
| Actor resolves to a slab of terrain | Nearest-model fallback; the actor is manager-spawned |
| "No authored mask" on an actor that clearly has one | Parser mask fields; read the comparison stage instead |

## Still open

- **Reimport UV** and **Export glTF** are scaffolds.
- Authoring only reaches actors whose model lives in the stage archive.
  HamuKuri loads per spawn from his manager and is not reachable.
- `add_goop_layer` claims one free konst register per material and will refuse
  on a material already using all four.
- Nothing drives the konst at runtime, so a baked layer is permanent. Washing it
  needs game code — the enemy's own class does this in retail, and an actor that
  never shipped with goop has no such code.

  For BossPakkun the input already exists. `TBossPakkun` accumulates water hits
  in `unk178` and tips into `TNerveBPTumbleIn` once it passes
  `mSLWaterMarkLimit`, which defaults to 600:

  ```cpp
  if (boss->unk178 >= boss->getBossPakkunSaveParam()->mSLWaterMarkLimit.get()) {
      spine->pushAfterCurrent(&TNerveBPTumbleIn::theNerve());
  ```

  So a wash is `K0_A = 255 - (unk178 * 255 / limit)` — the same shape as
  StayPakkun's `mHitPoints * 255 / maxHp`, spraying him thinning the coating and
  stopping holding it. What is missing is code to write that konst each frame:
  a patch hooking his update, resolving his material through `getModel()`, and
  setting the konst alpha the layer claimed. `add_goop_layer` reports which
  register that is.
