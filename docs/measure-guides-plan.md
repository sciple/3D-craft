# Measure tool leaves snappable guides

## Context

Today the Measure tool (`src/tools/measure-tool.ts`) is purely transient: it draws an amber
`PreviewLine` between two clicked points, writes a distance to the HUD, and throws everything away on
the next measurement or tool switch. It issues no backend commands at all.

That makes measuring a dead end. You can find out that a bolt hole belongs 12.5 mm along an edge, but
you then have to re-derive that position by eye or by typed coordinates when you actually draw the
circle. The measurement produces knowledge the modeling tools can't consume.

**Goal:** a completed measurement leaves a persistent *guide* — the two endpoints as marks plus the
measured segment itself — and the draw tools snap to it. A rectangle's corner or a circle's centre can
then land exactly on a point you measured, including anywhere *along* the measured path, not just at
its ends.

Decisions already settled with the user:

1. A guide is the segment. Snap targets: both endpoints, the midpoint, and any point along it.
2. Guides live in the Rust `Document` — they persist in the project file and participate in Ctrl+Z.
3. Guides accumulate. A **Clear Guides** button in the outliner removes them all in one undo step.
   `Esc` in the Measure tool still only cancels the in-progress measurement.

The payoff of decision 2 is that `findPlanarSnap`/`findSnap3d` already receive a whole
`DocumentSnapshot`, so snapping integration needs **no signature change and no edits at any of the
seven call sites** — including zero changes to `draw-tool.ts`.

> **This work happens on a branch.** It touches the document model, the project-file format, and the
> snapping code every draw tool depends on — all pre-existing, working behaviour. Branch first
> (`git checkout -b measure-guides` off `main`), validate there, and only fold back to `main` once the
> feature works and nothing regressed. §7 turns this into a standing rule in CLAUDE.md.

---

## 1. Rust data model

One type, `Guide`, reused across `Document`, `DocumentSnapshot` and `ProjectFile`. `ProjectFile` has
parallel types only because slotmap ids aren't portable across runs
([project_file.rs:3-9](../src-tauri/src/io/project_file.rs#L3-L9)); a `Guide` holds no ids, so a mirror
type would be pure duplication. `glam` already has the `serde` feature
([Cargo.toml:25](../src-tauri/Cargo.toml#L25)), and serializes `DVec3` as a 3-element array — the same
shape the frontend already sends for `planeOrigin`/`delta`.

In [document.rs](../src-tauri/src/scene/document.rs), next to `Group` (~:29):

```rust
/// A construction guide left behind by the Measure tool: the measured segment
/// itself. Its two endpoints, its midpoint, and any point along it are snap
/// targets, so a primitive can be built exactly on a distance you just
/// measured. Guides are reference-only annotations - they never enter `Mesh`,
/// so they're automatically invisible to triangulation, STL export,
/// `check_model`, bounding boxes and `connected_components`.
///
/// Deliberately world-fixed: no transform command moves them (see §3).
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub struct Guide {
    pub a: DVec3,
    pub b: DVec3,
}
```

Field names `a`/`b` follow the existing `ProblemEdge { a, b }` precedent
([document.rs:987](../src-tauri/src/scene/document.rs#L987)), which already has a TS mirror. No
`#[serde(rename_all)]` — field names cross IPC verbatim in this codebase.

| Location | Change |
| --- | --- |
| [document.rs:40-53](../src-tauri/src/scene/document.rs#L40-L53) `struct Document` | `pub guides: Vec<Guide>,` after `pub selection` |
| [document.rs:56-64](../src-tauri/src/scene/document.rs#L56-L64) `Document::new()` | `guides: Vec::new(),` |
| [document.rs:975-980](../src-tauri/src/scene/document.rs#L975-L980) `DocumentSnapshot` | `pub guides: Vec<Guide>,` (stays f64 — `vertices` is f32 only because it feeds a GPU buffer) |
| [document.rs:779-784](../src-tauri/src/scene/document.rs#L779-L784) snapshot literal | `guides: self.guides.clone(),` |
| [document.rs:819](../src-tauri/src/scene/document.rs#L819) `to_project_file` | `ProjectFile { vertices, faces, groups, guides: self.guides.clone() }` |
| [document.rs:833-882](../src-tauri/src/scene/document.rs#L833-L882) `from_project_file` | `doc.guides = project.guides.iter().copied().filter(\|g\| g.a.is_finite() && g.b.is_finite()).collect();` — mirrors the `pos.is_finite()` filter at :845; this fn **must stay total** (see its :822-832 comment) |
| [project_file.rs:11-15](../src-tauri/src/io/project_file.rs#L11-L15) | `#[serde(default)] pub guides: Vec<Guide>,` + import |

`#[serde(default)]` is **mandatory**. `ProjectFile` has no schema-version field and no defaults today,
so without it every already-saved `project.json` fails with `missing field 'guides'` inside
`load_project` ([commands.rs:341-349](../src-tauri/src/commands.rs#L341-L349)).

Add two `Document` methods next to `group_faces` — trivial bodies, but they give the doc comment a
home and are what the tests call:

```rust
pub fn add_guide(&mut self, a: DVec3, b: DVec3) { self.guides.push(Guide { a, b }); }
pub fn clear_guides(&mut self) { self.guides.clear(); }
```

## 2. Commands

In [commands.rs](../src-tauri/src/commands.rs) after `select_faces` (:324), following the established
lock → validate → `record()` → mutate → `snapshot()` shape:

```rust
/// One measurement = one guide = one undo step. Guards run *before*
/// `record()` (same as `array_faces`): a degenerate or non-finite segment
/// must not leave a no-op undo step behind, and a NaN would poison the
/// renderer's bounding sphere.
#[tauri::command]
pub fn add_guide(state: State<AppState>, a: DVec3, b: DVec3) -> DocumentSnapshot {
    let mut history = state.0.lock().unwrap();
    if !a.is_finite() || !b.is_finite() || a.distance_squared(b) < 1e-12 {
        return history.document.snapshot();
    }
    history.record();
    history.document.add_guide(a, b);
    history.document.snapshot()
}

/// Removes every guide in one undo step. No-op-safe: with nothing to clear,
/// recording anyway would make Ctrl+Z silently step over the user's previous
/// *real* edit.
#[tauri::command]
pub fn clear_guides(state: State<AppState>) -> DocumentSnapshot {
    let mut history = state.0.lock().unwrap();
    if history.document.guides.is_empty() {
        return history.document.snapshot();
    }
    history.record();
    history.document.clear_guides();
    history.document.snapshot()
}
```

Register both in [lib.rs:13-42](../src-tauri/src/lib.rs#L13-L42). Neither returns `Result` — a zero-length
measurement isn't worth a dialog, and the frontend guards it too.

Frontend wrappers in [document-store.ts](../src/state/document-store.ts) after `selectFaces` (:380), using
`applyEdit` (guides are persisted content, so the close-guard must see `dirty`):

```ts
addGuide(a: Vec3, b: Vec3) {
  return this.enqueue(async () => {
    this.applyEdit(await invoke<DocumentSnapshot>("add_guide", { a, b }));
  });
}

clearGuides() {
  return this.enqueue(async () => {
    this.applyEdit(await invoke<DocumentSnapshot>("clear_guides"));
  });
}
```

Accepted wrinkle: `applyEdit` also calls `showModelProblems(null)`
([:127-135](../src/state/document-store.ts#L127-L135)), so taking a measurement clears a Check Model
highlight even though no geometry changed. Not worth a third `apply` variant.

TS mirror near `ProblemEdge` ([:41](../src/state/document-store.ts#L41)):

```ts
/// A Measure-tool guide segment - see the Rust `Guide`. Its endpoints,
/// midpoint, and any point along it are snap targets for the draw tools.
export interface Guide {
  a: [number, number, number];
  b: [number, number, number];
}
```

Add `guides: Guide[];` to `DocumentSnapshot` **and to `EMPTY_SNAPSHOT`**
([:71](../src/state/document-store.ts#L71)).

## 3. Guides are world-fixed (deliberate)

`translate_faces` / `rotate_faces` / `scale_faces` / `drop_to_plate` / `arrange_for_print` are **not
touched**. Guides never move.

The primary workflow depends on this: *measure a hole spacing on part A → move part A aside → build
part B on the marks.* A guide that chased part A would destroy that. There is also no non-arbitrary
rule for "which guides belong to this face set" (endpoint within eps of a moved vertex? both
endpoints? bbox overlap?), and every candidate rule is wrong half the time — exactly the generality
CLAUDE.md says to skip. SketchUp agrees: guides move only when selected, and this app has no notion of
selecting a guide.

Document the limitation in the `Guide` doc comment, in CLAUDE.md, and in the README: after a
Move/Rotate/Scale, existing guides stay where they were measured. Clear Guides and re-measure. Test 3
(§9) pins it so a future "fix" has to consciously delete a test.

## 4. Snapping — the core of the feature

[src/tools/snapping.ts](../src/tools/snapping.ts). One new `SnapKind`, no new tier.

**One kind for all three guide hit types:**

```ts
export type SnapKind = "endpoint" | "midpoint" | "edge" | "guide";
// SNAP_COLORS gains: guide: GUIDE_COLOR   // 0xff5ecb, matching the drawn guide
```

The indicator colour's only job is "model, or the mark I left?" — endpoint-vs-midpoint-vs-along is
already obvious from *where* the dot sits on a visible two-point segment. `SnapKind` is a pure label
(`nearestWithin` never branches on it), which is what makes one kind across three tiers work.

**Guides merge into the existing tiers** rather than adding a fourth:

| Tier | Model candidates | Guide candidates |
| --- | --- | --- |
| 1 (early return) | vertices → `endpoint` | guide endpoints → `guide` |
| 2 (early return) | edge midpoints → `midpoint` | guide midpoints → `guide` |
| 3 (return) | closest point on each boundary edge → `edge` | closest point on each guide segment → `guide` |

Within a tier, nearest wins — a model corner and a guide endpoint are equally deliberate targets. A
*fourth*, lower tier would let a distant model edge point beat a much nearer guide-interior point,
wrong exactly when the guide is what you're aiming at.

**The one structural change** — `nearestWithin` takes pre-labelled candidates. Apply the identical
edit in **both** functions or the tier orders silently diverge:

```ts
const nearestWithin = (candidates: SnapResult[]): SnapResult | null => {
  let best: SnapResult | null = null;
  let bestDist = tolerance;
  for (const c of candidates) {
    const d = c.point.distanceTo(planePoint); // `anchor` in findSnap3d
    if (d < bestDist) { bestDist = d; best = c; }
  }
  // Clone on the way out (findSnap3d already did; findPlanarSnap didn't).
  return best && { point: best.point.clone(), kind: best.kind };
};
```

**`findPlanarSnap` plane filtering** — guides aren't in `snapshot.vertices`, so the index-based
`onPlane` ([:41](../src/tools/snapping.ts#L41)) becomes point-based and serves both sources:

```ts
// Must clone: `sub` mutates in place, and these vectors are also pushed as
// snap candidates. (The current index-based onPlane gets away with it only
// because positionOf allocates a throwaway per call.)
const pointOnPlane = (p: THREE.Vector3) =>
  Math.abs(p.clone().sub(plane.origin).dot(plane.normal)) < PLANE_EPS;
```

Parse guides once per call, then feed the tiers. A guide contributes a midpoint/along-point only when
it lies **wholly** on the sketch plane — the same rule model edges already use
([:70](../src/tools/snapping.ts#L70)); a guide that merely crosses the plane has an off-plane midpoint,
and snapping to it would drag the drawn shape off-plane. Endpoints are tested individually.

`findSnap3d` gets the identical tier merge with no plane gate, so you can measure guide-to-guide.

Precision is fine: a guide endpoint that snapped to a model vertex was read from an f32 snapshot,
promoted to f64 over JSON, and stored as that exact f64 — agreeing with the vertex to within the f32
grid, far inside `PLANE_EPS = 1e-4`.

**Not in v1** (note as follow-ups, don't build): snapping to where a guide *pierces* the sketch plane
(needs segment-plane intersection; the primary workflow has both endpoints coplanar), and memoizing
parsed guide vectors (guides are few; the per-call allocation is what keeps the clone rule safe).

**No `draw-tool.ts` changes.** Rectangle's first click is a corner, Circle/Ngon/Arc's is a centre, and
all five call `findPlanarSnap` directly on that click — "corners or centres snap to the marks" falls
out for free.

## 5. Rendering — new `src/viewport/guide-renderer.ts`

A new file, not an extension of `MeshRenderer`. That class is documented as "the document's
triangulated geometry" and is built around one shared `positions` buffer indexed by
`snapshot.vertices`; guides carry their own world coordinates and need per-mark *points*, which it has
no precedent for. The whole requirement is that guides read as **not geometry** — separate file,
separate concept.

```ts
export const GUIDE_COLOR = 0xff5ecb;

export class GuideRenderer {
  readonly lines: THREE.LineSegments;  // dashed segments
  readonly marks: THREE.Points;        // endpoint + midpoint marks
  constructor(scene: THREE.Scene) { ... }
  update(snapshot: DocumentSnapshot) { ... }
}
```

- **Lines**: `LineDashedMaterial({ color: GUIDE_COLOR, dashSize: 1.5, gapSize: 1.5, transparent: true,
  opacity: 0.9, depthTest: true })`, `renderOrder = 5`. Dashed is the "not geometry" signal, and it's
  the *only* one available — `linewidth` is ignored by ANGLE on Windows (already documented at
  [mesh-renderer.ts:64-68](../src/viewport/mesh-renderer.ts#L64-L68)), so only colour and dash pattern
  remain. `depthTest: true` so a guide behind a solid is occluded rather than cluttering the viewport.
- **`geometry.computeLineDistances()` after every `setAttribute("position", …)`**, not once in the
  constructor. Miss it and the dashes render solid — i.e. guides look exactly like model edges,
  defeating the point.
- **Marks**: `THREE.Points` + `PointsMaterial({ color: GUIDE_COLOR, size: 6, sizeAttenuation: false,
  depthTest: false })`, `renderOrder = 6`. Three points per guide: `a`, `b`, **and the midpoint** —
  drawing the midpoint is what makes that snap discoverable. `sizeAttenuation: false` gives a constant
  6px square at any zoom and sidesteps the linewidth problem. `depthTest: false` is deliberately
  asymmetric with the lines: the mark is the interactive thing and the snap functions aren't
  occlusion-aware, so a hidden-but-snappable mark would be worse than a visible one.
- **renderOrder 5/6** sit in the free 5..998 band — above `problemEdges` (4), below `SnapIndicator`
  (999) so the live snap dot always wins.
- **Empty state**: early-out when `snapshot.guides.length === 0` (both objects `visible = false`),
  mirroring `showProblems` ([:83-86](../src/viewport/mesh-renderer.ts#L83-L86)) and avoiding a NaN radius
  from `computeBoundingSphere()` on a zero-length buffer.
- No `dispose()` — constructed once, lives forever, same as `MeshRenderer`.

Wiring in [main.ts](../src/main.ts): construct after :29, and fold into the existing subscription at :73:

```ts
documentStore.subscribe((snapshot) => {
  meshRenderer.update(snapshot);
  guideRenderer.update(snapshot);
});
```

`snapping.ts` imports `GUIDE_COLOR` so the indicator dot matches the guide it's snapping to
(`tools/ → viewport/` already exists via [types.ts:2](../src/tools/types.ts#L2)).

## 6. Measure tool

[measure-tool.ts](../src/tools/measure-tool.ts) — three edits plus the doc comment.

**Second click commits the guide** (replacing :46-52):

```ts
} else {
  const start = this.first;
  const end = pick.point.clone();
  this.showReadout(start, end);
  // Reset synchronously, before the queued async invoke: a second click
  // landing while the round trip is pending must already see clean state.
  // Fire-and-forget, exactly like every draw tool's commit().
  this.first = null;
  // The finished segment is now a persistent guide drawn by GuideRenderer off
  // the next snapshot - drop the transient preview so it isn't drawn twice.
  this.line.clear(ctx.scene);
  if (start.distanceTo(end) > 1e-6) {
    void documentStore.addGuide([start.x, start.y, start.z], [end.x, end.y, end.z]);
  }
}
```

There's a sub-frame gap between clearing the preview and the snapshot arriving. Accepted — holding the
preview until a store subscription fires means the tool subscribing to the store, real machinery for an
invisible flicker.

**Remove the `line.clear` on first click** ([:43](../src/tools/measure-tool.ts#L43)) — now dead
(`onPointerMove` only touches the line when `first` is set), and its comment about "clearing any
previous persisted segment" becomes a lie once the segment is a guide.

**`reset()` ([:59-63](../src/tools/measure-tool.ts#L59-L63)) is unchanged** — add a comment that `Esc`
cancels the *in-progress* measurement and must never touch committed guides, so a future reader doesn't
"helpfully" extend it.

**Rewrite the class doc comment** ([:7-16](../src/tools/measure-tool.ts#L7-L16)) — "it issues no backend
commands at all" is now false. New framing: creates no *geometry*, but records the measured segment as
a guide via `add_guide` — undoable, saved with the project, snappable.

## 7. UI and docs

**Clear Guides button** in [outliner.ts](../src/ui/outliner.ts), appended to `actionsRow` at :69. Not
selection-gated like its neighbours — guides aren't geometry and aren't selectable; enabled purely by
whether any exist. Label it with the count (the only signal that off-screen guides exist), wired into
the **existing** subscription at :147-171, no new subscription:

```ts
const guideCount = snapshot.guides.length;
clearGuidesButton.textContent = guideCount > 0 ? `Clear Guides (${guideCount})` : "Clear Guides";
clearGuidesButton.disabled = guideCount === 0;
```

No CSS needed — `.outliner-actions-row` is already `flex-wrap: wrap` with `flex: 1` buttons
([styles.css:186-196](../src/styles.css#L186-L196)), so the 5th wraps onto its own line. No `icons.ts`
change (the outliner uses text buttons).

**README.md** (mandatory per CLAUDE.md's README rule — this is user-facing):
- Rewrite the Measure row ([:64](../README.md#L64)): now leaves a persistent guide that draw tools snap to
  (endpoints, midpoint, anywhere along); `Esc` still only cancels the in-progress measurement; each
  measurement is one `Ctrl+Z`; guides save with the project; **guides don't move when you move
  geometry**.
- Add **Clear Guides** to the Outliner bullet ([:72-74](../README.md#L72-L74)).
- A short line under "Other controls" describing the measure → snap workflow, since that's the point.

**CLAUDE.md** — architecture updates: `Document` now carries `guides: Vec<Guide>` (world-fixed, never
enters `Mesh`, hence free exclusion from export/manifold/bbox); `snapping.ts` guides participate in all
three tiers under one `"guide"` kind; `project_file.rs` — the `#[serde(default)]` field and the general
rule it establishes (no schema version → every new field must default); add
`viewport/guide-renderer.ts` to the frontend list.

**CLAUDE.md — new standing rule**, added near the top alongside the "Keep README.md up to date" rule so
it governs all future work, not just this feature:

> **Branch large or risky features.** Any change big enough to disrupt pre-existing, working behaviour
> — a new document-model collection, a project-file format change, edits to shared geometry or snapping
> code, a new tool touching existing ones — must be built and validated on its own git branch first.
> Only fold it back into `main` once it works and the existing behaviour it touches is verified
> unbroken. Small, self-contained changes can go straight to `main`.

## 8. Git workflow

Branch **before step 1**:

```
git checkout -b measure-guides
```

Everything below happens there. Fold back to `main` only after the full verification walkthrough
passes — in particular the regression checks (steps 9-11: old project files still open, STL export
unchanged, existing draw-tool snapping still behaves). The CLAUDE.md rule above is what makes this the
default for future features rather than a one-off.

## 9. Tests

All Rust inline, per CLAUDE.md. No frontend test runner exists — `npx tsc --noEmit` is the frontend gate.

In [document.rs](../src-tauri/src/scene/document.rs) `mod tests` (:1013), beside the existing
`project_file_round_trip_preserves_*` tests at :1812/:1885:

1. `guides_round_trip_through_the_project_file` — two guides, `to_project_file()` →
   `from_project_file()`, exact coordinate equality (f64 throughout, so exact is legitimate).
2. `a_project_file_written_before_guides_existed_still_loads` — **the highest-value test**; it protects
   every already-saved file. Must go through serde, not a struct literal (a literal always has the
   field): `serde_json::from_str(r#"{"vertices":[],"faces":[],"groups":[]}"#)`, assert
   `project.guides.is_empty()` and that `from_project_file` yields zero guides. `serde_json` is a normal
   dependency.
3. `guides_are_not_moved_by_geometry_transforms` — draw a rect, add a guide, `translate_faces(+10 Z)`,
   assert the guide is unchanged. Pins §3.

In [commands.rs](../src-tauri/src/commands.rs) `mod tests` (:412) — these drive `History` directly with no
Tauri runtime, replicating the `record()` + mutate shape like the existing tests:

4. `clearing_guides_undoes_in_one_step` — three record+add pairs, then one record+clear; a single
   `undo()` restores all three. Mirrors `an_array_of_copies_undoes_in_one_step` (:462).
5. `undo_after_a_measurement_removes_just_that_guide`.

Not tested: the command-level guards, which need `State<AppState>`. `array_faces`'s guards are equally
untested today — matching the existing pattern is the consistent choice.

## 10. Build order

0. **`git checkout -b measure-guides`** (see §8) — everything below happens on that branch.
1. Rust data model — `Guide`, `Document.guides`, `new()`, the two methods, snapshot, `ProjectFile` +
   `#[serde(default)]`, `to_project_file`/`from_project_file`. `cargo build`.
2. Document tests 1-3. `cargo test`.
3. Commands + `lib.rs` registration + tests 4-5. `cargo test`.
4. TS mirror — `Guide`, `DocumentSnapshot.guides`, `EMPTY_SNAPSHOT`, `addGuide`/`clearGuides`.
   `npx tsc --noEmit`.
5. `GuideRenderer` + `main.ts` wiring.
6. Measure tool commits the guide → visible end-to-end for the first time.
7. Snapping — `nearestWithin` refactor, `pointOnPlane`, guide candidates in both functions,
   `SnapKind`/`SNAP_COLORS`.
8. Outliner Clear Guides button.
9. README + CLAUDE.md.

Steps 5 and 7 are independent, but seeing guides before snapping to them makes 7 much easier to verify
by hand.

### Sharp edges

- **Two existing `ProjectFile { … }` struct literals** at
  [document.rs:1845](../src-tauri/src/scene/document.rs#L1845) and
  [:1870](../src-tauri/src/scene/document.rs#L1870) stop compiling the moment the field is added — they
  need `guides: vec![]`. `#[serde(default)]` is a serde attribute, not a Rust `Default`. This is the
  first error you'll hit in step 1; expected, not a sign of trouble.
- **`EMPTY_SNAPSHOT`** — miss it and `snapshot.guides` is `undefined` until the first `refresh()`
  resolves; `GuideRenderer.update`, both snapping functions and the outliner all read `.length` and
  throw. Fix the constant; do **not** paper over it with `?? []` in the readers.
- **`#[serde(default)]`** — without it, 100% of existing saved projects fail to load. Test 2 guards it.
- **`pointOnPlane` must clone** before `.sub()` — it's handed vectors that are also pushed as
  candidates, unlike today's `onPlane(i)` which mutates a throwaway. Silent-wrong-answer bug if missed.
- **`computeLineDistances()`** after every rebuild, or guides render solid.
- **`Guide` must derive `Clone`/`Copy`** or `Document`'s derived `Clone` — which `History::record`
  depends on — stops compiling.
- Guides never enter `Mesh`, which is what makes STL export, `is_manifold`, `check_model` and
  `connected_components` correct for free. A future "promote a guide to a real edge" is a new command,
  not a change to this storage decision.

## Verification

Automated: `cd src-tauri && cargo test` (5 new tests pass, none regress) and `npx tsc --noEmit` clean.

Manual, in `npm run tauri dev` — this is the acceptance walkthrough:

1. **Mark placement** — Measure (`E`), click a box corner, click a second corner. A dashed magenta
   segment with three square marks (both ends + midpoint) stays after the readout. Take a second
   measurement elsewhere: both guides remain.
2. **Corner snap** — Rectangle (`R`), hover a guide endpoint: the snap dot turns magenta. Click; the
   corner lands exactly there.
3. **Centre snap** — Circle (`C`), hover the guide's midpoint mark: magenta dot; the circle centres on
   it.
4. **Along-the-path snap** — hover partway along a guide with any draw tool: magenta dot tracking the
   segment. This is the "primitive built along the measured path" case.
5. **Priority** — hover where a guide endpoint sits near a model vertex: the nearer one wins, and the
   dot colour tells you which.
6. **Undo** — `Ctrl+Z` after a measurement removes exactly that guide, leaving earlier ones.
7. **Clear Guides** — the outliner button reads `Clear Guides (2)`; clicking empties the viewport of
   marks, and one `Ctrl+Z` brings them all back. With none, the button is disabled.
8. **Persistence** — Save, reload the app, Open: guides return and still snap.
9. **Backwards compatibility** — open a project file saved *before* this change: it loads with no
   guides and no error.
10. **World-fixed** — select a solid, Move it: the guides stay where they were measured (expected, and
    documented).
11. **Not geometry** — Export STL on a model carrying guides: succeeds, and the STL is unchanged.

Steps 9-11 are the regression checks that matter most — they cover the pre-existing behaviour this
feature reaches into. Only once all of the above passes on `measure-guides` does the branch fold back
into `main`.
