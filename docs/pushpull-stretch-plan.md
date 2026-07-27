# Push/Pull: retract a solid's face by stretching its walls

## Context

Pulling a face outward and then pushing it back leaves the side walls behind. The retracted
face lands back at the original height, but the walls it created stay at full height — the
solid looks unchanged, with z-fighting on the sides.

Root cause: `push_pull_impl` (`src-tauri/src/geometry/pushpull.rs:32`) only ever *adds*
geometry. It mints brand-new vertices for the offset loop (there is no vertex welding
anywhere in the codebase), adds one wall quad per boundary edge, and removes exactly one
face — the source. Nothing merges or deletes pre-existing faces. And in attached mode
`extruding_forward = attached || distance >= 0.0` (`pushpull.rs:38`) never flips winding, so
pushing a cap back down builds four *inward-facing* walls coincident with the original
outward ones, plus a cap at the base: a 9-face, zero-volume double shell that still passes
the id-based manifold check.

Intended outcome: SketchUp behavior. A face on a solid's boundary *moves*, and the faces
sharing its loop stretch with it — in both directions. Pushed flush with the surface behind
it, the walls disappear and the solid collapses back to a flat sketch face. Pushed past that
surface, the move clamps at the collapse rather than producing an inside-out solid.

Two decisions confirmed with the user:
- **Symmetric**: outward pulls stretch too. This removes the false horizontal seam edge that
  currently rings a solid after every re-pull, and stops face/vertex counts growing per drag.
- **Clamped**: overshoot stops at the collapse; no inverted solids.

The wall-creation path must survive untouched for the greeble workflows (inset panel pushed
inward, rectangle drawn on a wall pushed in) — there the pushed face's neighbour across the
shared loop is a *coplanar* face, and moving the shared vertices would drag it instead of
carving.

## Approach

Add a third path to `Document::push_pull`, chosen when the face is on a solid boundary **and**
none of its vertex-sharing neighbours is coplanar with it. Otherwise the existing
attached/standalone wall-creation paths run exactly as today.

Key property that makes this safe: a pure vertex translation cannot break the id-based
manifold check, and neither can a global id remap plus cyclic dedup — every directed edge
`(a,b)` and its twin `(b,a)` are rewritten identically, and a face collapsed to two distinct
vertices contributes `a→b` and `b→a`, which cancel. So no manifold repair pass is needed.

### 1. New module `src-tauri/src/geometry/stretch.rs` (register in `geometry/mod.rs`)

Mesh-only, so it unit-tests without a `Document` and keeps `document.rs` (already ~1860 lines)
from growing. **`pushpull.rs` is not modified** — all its tests keep passing unchanged.

```rust
pub struct StretchOutcome {
    /// Faces removed from the mesh: side walls that fully degenerated, plus a
    /// coincident reversed twin if the shell collapsed to zero volume. The
    /// caller must drop its own bookkeeping for each.
    pub removed_faces: Vec<FaceId>,
    /// The stretched face landed on an opposite-wound face over the same vertex
    /// set - no volume left, so the caller clears its solid-boundary flag.
    pub collapsed_to_flat: bool,
    /// What was actually applied - smaller than requested when clamped.
    pub applied_distance: f64,
}

/// Criterion (b'): the face has at least one vertex-sharing neighbour and none
/// of them is coplanar with it. Criterion (a) - solid-boundary membership - is
/// the caller's, since it owns `solid_face_ids`.
pub fn can_stretch(mesh: &Mesh, face_id: FaceId) -> bool;

/// Returns None leaving the mesh exactly as found when the planarity guard
/// rejects the move; the caller then falls back to `push_pull_attached`.
pub fn stretch_face(mesh: &mut Mesh, face_id: FaceId, distance: f64) -> Option<StretchOutcome>;
```

`can_stretch` — coplanar test must be `|n_a · n_b| >= 1 - 1e-6` **and** every vertex of the
other face's outer loop within `1e-6` of this face's plane. Do **not** reuse
`is_coplanar` (`document.rs:863`): it requires same-direction normals, and the anti-parallel
case is reachable — a standalone `push_pull` reuses the source vertex ids for its base cap, so
a box's downward base cap shares vertices with a leftover coplanar upward sketch. Also require
at least one neighbour, or a lone solid face would degenerate into a Move.

`stretch_face` steps:

1. `moved = face.outer ∪ face.holes`. Bail `None` on a ~zero normal.
2. `ring` = every face referencing any `v ∈ moved` (includes `face_id`);
   `anchors = (vertices of ring faces) − moved`.
3. **Clamp.** For each `v ∈ moved`, each `u ∈ anchors`: `w = pos(u) − pos(v)`,
   `s = w·n / distance`; accept when `s ∈ (1e-9, 1]` and `(w − n·distance·s).length() <= 1e-6`.
   `t* = min(1, accepted s)`; `applied = distance · t*`. No-op `Some` if `|applied| < 1e-9`.
4. Apply `pos(v) += n · applied` for `v ∈ moved`.
5. **Planarity guard.** Over `ring`, skipping fully-moved faces: recompute the Newell normal
   from new positions; a ~zero normal is fine (the weld will remove it), otherwise require
   max off-plane deviation `<= 1e-6 · extent` and `n_new · n_old > 0`. On any failure subtract
   the translation back and return `None` — no regression, just no fix, on exotic topology
   (e.g. a vertically subdivided wall column in a project file saved before this change).
6. **Weld.** For each `v ∈ moved` in sorted key order, find the nearest `u ∈ anchors` within
   `1e-6` → `remap[v] = u`. Rewrite every loop of every `ring` face through `remap`; remove the
   remapped ids from `mesh.vertices`. Scoping candidates to the one-ring is both sufficient
   (the faces that must collapse always bridge moved↔fixed vertices) and minimal — by the mesh's
   own invariant (`mesh.rs:27-31`) an unrelated solid, even a `duplicate_faces` copy at identical
   coordinates, shares no vertex ids, so it can never be welded into.
7. **Degenerate cleanup** over `ring`: cyclically dedup consecutive equal ids in every loop
   (handle wrap-around); drop hole loops left under 3 distinct ids; `remove_face` and record any
   face whose outer loop has under 3 distinct ids, a repeated id (pinched), or a zero normal.
8. **Twin collapse.** Scan **all** faces, not just `ring` — in the full-collapse case the base
   cap references only anchor vertices and is *not* in the pre-weld ring. Match when `outer` is
   the same id set and the hole loops are the same multiset of sets (mirror the shape of
   `matches_any_loop`, `document.rs:850`) and the normals oppose. Remove it, record it, set
   `collapsed_to_flat`.
9. `mesh.recompute_normal` for every surviving `ring` face.

### 2. `src-tauri/src/scene/document.rs`

Split `erase_face` (`document.rs:390`) into a reusable bookkeeping half:

```rust
/// Drops every document-level record of `face_id` (group membership, selection,
/// solid-boundary flag) without touching the mesh - for callers that removed the
/// face themselves.
fn forget_face(&mut self, face_id: FaceId)
```

with `erase_face = forget_face + mesh.remove_face`.

In `push_pull` (`document.rs:337`), insert the stretch branch **above** the
`face_to_group.remove` / `solid_face_ids.remove` lines at 357-358 — otherwise a stretched face
silently loses its group and its `solid` bit:

```rust
if self.solid_face_ids.contains(&face_id) && stretch::can_stretch(&self.mesh, face_id) {
    if let Some(outcome) = stretch::stretch_face(&mut self.mesh, face_id, distance) {
        for fid in outcome.removed_faces { self.forget_face(fid); }
        if outcome.collapsed_to_flat { self.solid_face_ids.remove(&face_id); }
        return vec![face_id];   // face survives: keeps group, solid flag, selection
    }
}
// ...existing bookkeeping + wall-creation paths, unchanged...
```

`stretch_face` already refreshed the affected normals, so `recompute_normals_touching` is not
called here. Update `push_pull`'s doc comment: it now returns "the faces standing in for the
source face", not "the newly created faces".

Consequence to accept deliberately: the face stays in `selection` (today the source is dropped
at `document.rs:377`), so it stays highlighted and repeated pushes on a selection keep working —
consistent with the shared "clicked face is in the selection → operate on the whole selection"
pattern.

### 3. Frontend, snapshot, IO — no changes needed

Verified: `pushpull-tool.ts` clears `targetFaceIds` on pointer-up and rebuilds its preview from
a fresh snapshot on the next pointer-down; `mesh-renderer.ts` rebuilds its triangle→FaceId table
per snapshot; `document-store.ts` applies the whole snapshot. Nothing depends on face ids
changing across a push/pull. `DocumentSnapshot` re-interns everything per call.
`to_project_file` interns only vertices referenced by faces, so welded-away vertices simply
don't serialize and the `solid` bit round-trips (including `false` after a collapse).
`History::record` deep-clones the whole `Document`, so vertex removal and face erasure are
fully reversible.

## Files

- `src-tauri/src/geometry/stretch.rs` — **new**
- `src-tauri/src/geometry/mod.rs` — register the module
- `src-tauri/src/scene/document.rs` — `forget_face` extraction, `push_pull` stretch branch, tests
- `src-tauri/src/geometry/pushpull.rs` — **read only**, fallback path, unmodified
- `README.md:51` — extend the Push/Pull row: a solid's face carries its walls with it, and
  pushing it flush with the surface behind removes them
- `CLAUDE.md:44` — the `pushpull.rs` bullet states the dispatch as "picks the variant from
  `solid_face_ids` membership"; add the third path and `geometry/stretch.rs`, and note that the
  coplanar-neighbour test is what preserves the inset/greeble and sketch-on-wall carving
  workflows. In the cross-cutting invariants (`CLAUDE.md:62-67`) add the weld scoping rule and
  the "global remap + cyclic dedup preserves edge pairing" property.

## Tests

One existing test changes behaviour: `pulling_a_solids_cap_extends_it_into_one_manifold_solid`
(`document.rs:1287`) — rename to `pulling_a_solids_cap_stretches_its_existing_walls`, reword the
comment, `assert_eq!(all_faces.len(), 10)` → `6`, keep the manifold and `top_z ≈ 2.0` asserts,
and add `assert_eq!(doc.mesh.vertices.len(), 8)` — that is what actually pins the new behaviour.

Every other `push_pull` test either pushes a flat sketch (criterion (a) fails) or a face with a
coplanar vertex-sharing neighbour (criterion (b′) fails), so all take the wall path unchanged —
including `recessed_panel_greeble_stays_manifold`, `raised_panel_greeble_stays_manifold`,
`pushing_a_face_drawn_on_a_solid_wall_stays_manifold`, and
`a_second_stud_drawn_on_a_solid_keeps_the_first_stud_watertight`. All `pushpull.rs` tests call
the free functions directly and are unaffected.

New tests in `document.rs`:

| Name | Assertion |
|---|---|
| `pushing_a_pulled_cap_back_down_takes_its_walls_with_it` | The reported bug: box h=1 → pull +2 → push −2 ⇒ 6 faces, 8 vertices, max z ≈ 1.0, manifold |
| `pushing_a_cap_back_partway_shrinks_the_existing_walls` | box h=2 → push −1 ⇒ 6 faces, 8 vertices, max z ≈ 1.0 |
| `pushing_a_cap_flush_with_the_base_leaves_a_single_flat_face` | 1 face, 4 vertices, `solid_boundary_face_ids().is_empty()` |
| `pushing_a_cap_past_the_base_stops_at_the_collapse_instead_of_inverting` | same, plus min z ≈ 0.0 |
| `a_collapsed_box_pulls_back_into_a_solid` | re-pull ⇒ 6 faces, 6 solid ids — proves un-flagging routes back through the two-cap variant |
| `pushing_a_raised_panel_back_down_restores_the_inset_split` | 7 faces, exactly one holed face, panel outer == frame hole vertex set, manifold |
| `pushing_a_recess_floor_back_up_restores_the_flush_face` | 7 faces, manifold |
| `pushing_an_inset_panel_still_carves_a_recess` | routing guard: face count *grows* to 11, recess floor normal +Z |
| `pushing_a_box_wall_moves_it_instead_of_carving_into_the_solid` | 6 faces, x-extent grew by 1, manifold |
| `push_pull_faces_on_two_adjacent_walls_grows_the_box_in_both_axes` | 6 faces, both extents grew, manifold |
| `pushing_a_capped_slab_down_keeps_its_stud_attached` | manifold, slab z dropped by 1, stud top z unchanged |
| `pushing_a_tube_cap_back_to_the_base_leaves_a_flat_ring` | 1 face with 1 hole, no solid ids |
| `stretching_a_face_keeps_it_selected_and_grouped` | face still in its group, in `selection.faces`, in `solid_boundary_face_ids()` |
| `pushing_a_stacked_wall_column_does_not_leave_an_inverted_wall` | build a two-row box via `from_project_file` (a pre-fix save), push through the middle ring; assert manifold and every stored normal agrees with its Newell normal — asserts the invariant, not which path handled it |

New tests in `stretch.rs`: `can_stretch_is_true_for_a_box_cap`;
`can_stretch_is_false_when_a_coplanar_neighbour_shares_a_vertex`;
`can_stretch_is_false_for_an_anti_parallel_coplanar_neighbour`; `can_stretch_is_false_for_a_lone_face`;
`welding_never_reaches_an_unrelated_coincident_solid` (two boxes at identical coordinates via
`duplicate_faces` with `DVec3::ZERO`; collapse one, assert the other still has 6 faces and 8
vertices at the original positions).

New test in `commands.rs`: `undo_restores_a_collapsed_box` — collapse, undo, assert 6 faces and
8 vertices are back.

## Implementation order

1. `forget_face` extraction (mechanical, no behaviour change).
2. `stretch.rs` with `can_stretch` + unit tests, not yet wired.
3. `stretch_face` steps 1-5 + wiring into `push_pull`. Partial retraction, wall pushes, and
   symmetric pull work here; edit `pulling_a_solids_cap_...`.
4. Steps 6-7 (weld + degenerate cleanup). Raised-panel restore and tube cases land here.
5. Step 8 (twin collapse) + `collapsed_to_flat`. Full-collapse and overshoot cases.
6. Remaining tests, then `README.md` + `CLAUDE.md`.

## Verification

- `cd src-tauri && cargo test` — full suite green, including the edited count assertion.
- `cd src-tauri && cargo test stretch -- --nocapture` — the new module in isolation.
- `npx tsc --noEmit` — should be a no-op (no frontend changes), run to confirm.
- `npm run tauri dev`, then manually:
  1. Rectangle on the ground → `P` → pull up 10 (typed). Pull again +5: the box must grow with
     **no horizontal seam line** across the sides.
  2. Push the top cap −5, then −10: walls shrink with it; at −10 the solid becomes a single flat
     rectangle, and File → Export STL then reports nothing printable (no solids left).
  3. Repeat, pushing −30 in one go: must stop flush at the base, not invert.
  4. Regression — greebles: box → `I` inset the top cap → push the inner panel −2. Must still
     **carve a recess** (face count grows), not thin the whole slab. Then push that recess floor
     back +2: the top must return flush and Export STL must still succeed.
  5. Regression — sketch on a wall: draw a rectangle on a box's side, push it in to make a
     porthole, verify Export STL succeeds.
  6. Ctrl+Z after a full collapse restores the box.
