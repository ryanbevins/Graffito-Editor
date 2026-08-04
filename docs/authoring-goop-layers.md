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
| Lit surfaces blow out, dark ones fine | Coating is added rather than composed |
| Everything vanishes below ~0.3 coverage | Layer claimed a konst register the material already reads |
| Pattern mirrored top to bottom | Coating authored with `v` flipped |
| Matches at 0 and 1, wrong between | Preview and wash comparing in opposite directions |
| Thin strips, coordinate jumping per vertex | Coordinates written in a different order than they were posed |
| One part twists, rest correct | `0xFFFF` matrix slots — hold across packets |
| Smearing everywhere, no coverage matches | Coordinate is generated rather than stored |

## Still open

- **Reimport UV** and **Export glTF** are scaffolds.
- Authoring only reaches actors whose model lives in the stage archive.
  HamuKuri loads per spawn from his manager and is not reachable.
- `add_goop_layer` claims one free konst register per material and will refuse
  on a material already using all four.
- Nothing drives the konst at runtime, so a baked layer is permanent. Washing it
  needs game code — the enemy's own class does this in retail.
