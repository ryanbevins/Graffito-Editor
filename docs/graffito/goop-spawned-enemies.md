# Goop-spawned enemies (Goobles / Stus)

Everything below was measured against the decomp and the extracted retail game,
not inferred. Where something is unverified it says so.

## Start here: the one root cause

**Goobles are `NameKuri`.** `NameKuriManager` is placed in bianco0-7, dolpic0/5/7/8,
mare0, monte2/3 and ricco0/8 — the goop stages — which is the correlation that
makes them the goop enemy.

`NameKuri` produces neither an authoring template nor a census warning. Every
other failure in this investigation announced itself with a reason; this one is
absent entirely, so it never becomes a census candidate. Managers carry no
placement or transform, and the census collects candidates from records that
have them, so a manager-only enemy is invisible to it end to end.

That single assumption — that an authorable object has a transform — is behind
all three symptoms:

1. Managers missing from the hierarchy (`crates/sms-scene/src/lib.rs:2345`
   drops records with no transform).
2. Managers unspawnable (`can_spawn_factory`'s first branch requires
   `placement.is_some()`).
3. Manager-only enemies missing from the census, silently.

Fix that and Goobles become placeable. The two census fixes recorded further
down are real bugs and worth keeping, but they addressed the Stu family, not
this. Do not repeat the mistake that produced them: "Goobles" was mapped onto
`DoroHaneKuri` by reading the Japanese roots rather than by asking, and a lot of
work went into the wrong enemy as a result.

## The mechanism is real

`TConductor::genEnemyFromPollution()` — `src/Enemy/conductor.cpp:258`:

```cpp
TStageEnemyInfo* info = unkF0->getMatchedInfo(0x1);        // entries with flag bit 0x1
TEnemyManager* mgr = (TEnemyManager*)getManagerByName(info->mManagerName);
// random point around Mario between mGenerateRadiusMin and mGenerateRadiusMax
targetPos.y = gpMap->checkGround(targetPos, &data) + 1.0f;
if (!gpPollution->isPolluted(targetPos.x, targetPos.y, targetPos.z))
    return;                                               // the goop gate
// probability gate: mGenerateProp, or TAreaCylinder::unk24 if inside one
TSpineEnemy* enemy = mgr->getFarOutEnemy();
enemy->resetToPosition(targetPos);
```

It fires every `mGenerateTime` frames. `getMatchedInfo` picks randomly among
matching entries weighted by `mWeight` (`src/Enemy/enemytable.cpp:22`).

Searching for `getPollutionType` finds nothing in `src/Enemy` — the read is
`isPolluted`. That mistake cost an hour; do not repeat it.

## The data

`TStageEnemyInfo` — `include/Enemy/EnemyTable.hpp:6`:

| field           | meaning                                          |
| --------------- | ------------------------------------------------ |
| `mManagerName`  | which enemy manager supplies the actors           |
| `mFlags`        | bit `0x1` = may be generated from pollution       |
| `mWeight`       | relative frequency among matching entries         |

The table registers itself on load: `gpConductor->registerEnemyInfoTable(this)`
(`src/Enemy/enemytable.cpp:19`). Both types come from the ordinary name-ref
factory (`src/System/MarNameRefGen.cpp:267`), so they are normal JDrama records.

**The editor already parses this** — `crates/sms-formats/src/jdrama.rs:595`
gives `StageEnemyInfo` the three fields with correct types.

## Where it lives

`tables.bin`, beside `scene.bin` in each stage archive. Measured: essentially
every retail stage has a `StageEnemyInfoHeader` there, `bianco0` included. It is
**not** in `scene.bin`, which is why a scene-only search finds nothing.

## Why nothing shows in the editor today

Two independent faults, neither of them the object index. The registry is fine:
`HamuKuri`, `HaneHamuKuri` and `HamuKuriManager` are all present and unhidden in
`crates/sms-schema/generated/object-registry.json`.

### 1. Non-spatial records are dropped on load

`crates/sms-scene/src/lib.rs:2345`:

```rust
let Some(transform) = record.transform else {
    continue;
};
```

Managers have no transform, so they never become `SceneObject`s. `bianco0`
really does contain `HamuKuriManager`, and the hierarchy cannot show it. This
also blocks `can_spawn_factory`, whose first branch looks for an existing object
with `placement.is_some()`.

**Verified: nothing is being lost today.** Export patches the existing archive
rather than regenerating it — `stage_export.rs` addresses `map/scene.bin`
specifically and edits it through `archive.resource_mut(b"scene.bin")`. Records
the loader drops, and whole files it never reads such as `tables.bin`, are
carried through untouched.

The consequence for the work below: the moment `tables.bin` is loaded into the
document, writing it back correctly becomes the editor's responsibility, and it
is not today. That is why step 1 is a round-trip test before any UI.

### 2. The retail census discards factories

`crates/sms-scene/src/object_authoring.rs` builds authoring templates by
censusing retail stages. It used to pick one winning candidate and drop the
factory entirely if that candidate failed to resolve. Now fixed: it ranks the
candidates and takes the first that resolves, reporting every rejection only
when all of them fail. Templates went 237 to 238.

That was not enough for the Kuri family. Both remaining causes look like real
bugs and are the next thing to look at:

- `required stage-local HamuKuri runtime texture H_ma_rak_dummy
  "/scene/map/pollution/H_ma_rak.bti" was not found in source stage coro_ex5`.
  The path is shared, so the check probably needs to consult common stage assets
  rather than only the stage the candidate came from.
- `HamuKuriManager manager model "default.bmd" matched 2 resources in source
  stage dolpic_ex0; an exact source cannot be selected safely`. A generic
  filename colliding across resource folders; likely wants disambiguating by the
  manager's own folder rather than refusing.

Either would probably land `HamuKuri`; both would land the family.

## Manager selection in Graffito

The goop inspector lists both compatible managers already in the stage and
manager bundles that Graffito can add safely. An absent manager is offered only
when the decomp-derived schema proves the actor/`TEnemyManager` relationship and
the retail authoring census provides an exact manager dependency with its full
resource closure.

Checking an absent manager imports that retail-backed bundle and creates a
pool-only editor handle. The handle owns the manager, character registrations,
runtime table dependencies, graphs, and resources, but its actor record is
omitted from export. The same undo step writes bit `0x1` on the matching
`StageEnemyInfo`. No enemy instance has to be placed in the world first.

Existing managers are reused. Unchecking a manager clears the goop-spawn flag;
if its pool-only handle has no other level dependency, Graffito removes the
handle and prunes only catalog-managed resources that no remaining authored
object needs. Imported managers and still-referenced pools are preserved.
Legacy projects receive the same reconciliation when opened. Export also clears
stale flags whose named manager no longer exists.

The decomp identifies two otherwise compatible managers with extra runtime
gates. `TPoiHanaManager::createEnemyInstance` returns null outside area `0x38`,
and `TConductor::init` excludes HinoKuri2 from ordinary reusable pool creation.
When either is actually needed, the managed build finds the corresponding PPC
instruction sequence in the selected game DOL and removes only that gate. This
keeps the manager picker data-driven while making the selected pool valid in a
custom stage.

## Which stages actually place Kuri objects

Measured from every retail `scene.bin`:

```
bianco0                 HamuKuriManager, NameKuriManager
bianco1-7               NameKuriManager
coro_ex4                HamuKuriManager, HamukuriLauncherManager, HamukuriLauncher
coro_ex5                HamuKuriManager, HaneHamuKuriManager, HamuKuri, HaneHamuKuri2
dolpic_ex0              HamuKuriManager, HaneHamuKuriManager, HamuKuri, HaneHamuKuri
monte_ex0               HamuKuriManager, HamukuriLauncherManager, HamukuriLauncher
pinnaBeach0/1/3/4       FireHamuKuri*, HamuKuriManager, HaneHamuKuri*, DoroHaneKuri*
pinnaParco0-7           DangoHamuKuriManager, BossDangoHamuKuriManager, BossDangoHamuKuri
test11                  HamuKuriManager, NameKuriManager, HinoKuri2Manager, HinoKuri2
dolpic0/5/7/8, mare0, monte2/3, ricco0/8   NameKuriManager
```

## Probes

Three `#[ignore]` tests in `apps/sms-editor/src/tests.rs`, driven by
environment variables. They are diagnostics rather than assertions; keep or
strip as preferred.

- `probe_authoring_census` — `GRAFFITO_PROBE_BASE_ROOT`, `GRAFFITO_PROBE_FACTORY`.
  Prints template count, warning count, and which factories resolved or were
  omitted and why.
- `probe_factory_stages` — same, plus optional `GRAFFITO_PROBE_STAGE` to dump
  every record type in one stage. Scans every `.bin`, not just `scene.bin`.

`GRAFFITO_PROBE_BASE_ROOT` for this machine is the extracted game at
`Documents/graffito/SMS-Extracted`.

Beware `| tail` on their output: it is alphabetical, and truncating it is how
"no Bianco stage has Kuri objects" got asserted twice when bianco0 has one.

## Status after the census fixes

`object_authoring.rs` now does two things it did not:

- Ranks every candidate and takes the first that resolves, instead of dropping
  the factory when one fails. All rejection reasons are reported only when
  nothing resolves.
- Resolves a fully qualified runtime texture from any stage that has it, via
  `add_shared_stage_reference`. Deliberately separate from `add_stage_reference`:
  a graph is stage-local data that shares a path between stages, and borrowing
  one binds the wrong object. `missing_stage_local_graph_does_not_fall_back_to_another_stage`
  exists to catch exactly that, and it caught a first attempt that widened the
  rule for everything.

Result: templates 237 to 242, with `HamuKuri`, `HaneHamuKuri`, `HaneHamuKuri2`,
`DoroHaneKuri` and `HinoKuri2` now present. The managers and actors are
placeable. They still do not spawn from goop.

## Step 1 blocker, located

`crates/sms-scene/src/lib.rs:2279` filters stage assets to one file:

```rust
.filter(|asset| asset.path...ends_with("/map/scene.bin"))
```

`tables.bin` is never read into the document, which is why the flag is invisible
even though `jdrama.rs` parses it correctly. Extending this filter is the first
real code change for the toggle.

**Likely but unverified:** because the editor emits `StageArchiveEdits` against
the base archive, a file it never reads is probably passed through untouched, so
nothing is being lost today. Confirm before changing the loader, because once
`tables.bin` is loaded it becomes the editor's responsibility to write it back
correctly.

## The Stu goop-stain investigation (hat on head)

Symptom: editor preview shows the goop stain on HamuKuri's cap; the user's
custom stage shows clean caps at runtime.

Mechanism, from the decomp: `THamuKuri::setMActorAndKeeper` binds a KColor to
`_mat_body_top1` with alpha 0x80, loads `/scene/map/pollution/H_ma_rak.bti`,
swaps it over `H_ma_rak_dummy`, and clears the alpha to 0 only when the file is
missing. The launcher sets no special state -- it pulls `getFarOutEnemy()` from
the same pool -- so "only launcher Stus have the stain" is a retail data
correlation, not a mechanism: launcher stages (bianco, pinna) ship the texture,
placed-Stu stages (dolpic episodes, corona, monte cave) do not.

Verified, in order, all healthy in the user's stage:

- texture present in every export, including the live project's
- `hamukuri/default.bmd` byte-identical across all 12 retail stages (fnv
  4bd49281056942c5), killing the wrong-variant-model theory
- RARC file flags 0x11 (MRAM preload), root named `scene` (test-asserted)
- pool enemies run `setMActorAndKeeper` via `TEnemyManager::createEnemies`
  -> `enemy->init(this)`
- the runtime directory walk reaches `map/pollution/`: goop models load from
  that directory via `initJointModel("scene/map/pollution", ...)` and the
  user's painted goop renders in Dolphin

Open lead: `H_ma_rak.bti` is NOT one texture. At least two variants exist
(ricco/mare fnv 116993acef3ab783, bianco fnv 345ba4fc9f798e55). The shared
borrow picks the first source in sort order rather than a canonical variant, so
the user's stage likely carries airport's stain, which may read as no stain.
Next step: eyeball the cap up close in Dolphin; if a faint or off-colour
overlay exists, pin the borrow to bianco's variant. If the cap is truly clean,
every static suspect is exhausted and it needs a breakpoint on JKRGetResource.

The census fixes that came out of this chase, both committed: native-stage
resolution is preferred over borrowing, and manager model references are
qualified by the candidate's own character folders, which newly resolves
HamukuriLauncher.
