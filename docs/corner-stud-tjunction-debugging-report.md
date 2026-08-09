# Debugging report: T-junction split fix (corner-stud regression)

**Audience**: a fresh reviewer with no prior context on this conversation. Your job is to try to
break what's described below — find bugs, hidden corner cases, and gaps between what was *tested*
and what was only *reasoned about*. This is not a request to re-explain what's here; it's a request
to attack it.

**Branch**: `measure-guides` (uncommitted). **Status at the time this report was written**: 99 Rust
tests passing, `tsc --noEmit` clean, one manual GUI pass by the user came back working. Not yet
committed or merged to `main`.

> **A second pass has since been done** and found two more bugs, both now fixed (105 tests passing).
> See [Audit results](#audit-results-second-pass-independent-reviewer) at the end — read it alongside
> this report, since it corrects a few claims made below.

## How we got here (context, not the thing to review)

This branch adds a "Measure tool leaves snappable guides" feature (full spec:
[measure-guides-plan.md](measure-guides-plan.md)). Implementing it touched `Document::resplit_face_with_loops`,
which is also the code path for the ordinary "sketch a shape directly on an existing solid's face"
workflow (studs, portholes, recesses) — snapping onto existing geometry is what guides are *for*, and
that made pre-existing, latent bugs in that shared code easy to trigger for the first time. Two rounds
of user bug reports against that shared path were already investigated and fixed earlier in this
branch (see `measure-guides-plan.md`'s "Post-implementation" and "Follow-up" sections) before the bug
this report is about.

## The bug this report covers

User report (verbatim): *"the changes applied create too much damage, for example, now building a
rectangle aligned to the edges of a rectangular face either produces invalid model or it is refused at
all as operation."*

Root cause: one of the two earlier fixes added a test,
`a_rectangle_sharing_a_corner_with_its_target_face_is_a_safe_no_op_not_a_lost_face`, that **encoded a
real regression as correct behavior**. Sketching a rectangle whose corner lands exactly on an existing
corner of the face it's drawn on — the everyday "stud in the corner of a box top" move, about as basic
as this app's workflow gets — always produces two pairs of exactly-collinear edges at that shared
corner. `face_detect::trace_ccw_loops`'s neighbor-angle sort has no tiebreaker for two edges leaving a
vertex in the same direction, and the fix in place at the time made that tie fail *safely* (reject,
don't corrupt) instead of failing *correctly* (still produce the split the user asked for). A safe
no-op is still wrong when the input was valid — and per the user's report, it wasn't even consistently
a no-op; some inputs came back as an invalid model instead.

## The three fixes (all three are necessary — see "how each was found" below)

### 1. `face_detect::split_edges_at_interior_points` — [face_detect.rs:49](../src-tauri/src/geometry/face_detect.rs#L49)

Called from `detect_faces` before `trace_ccw_loops` builds its neighbor-angle graph. For every edge
`(a, b)` in the input, finds every *other* point that lies on that edge's interior (a T-junction,
tolerance `ON_SEGMENT_TOLERANCE = 1e-3`, [face_detect.rs:54](../src-tauri/src/geometry/face_detect.rs#L54)),
sorts them along the edge, and replaces the one edge with the resulting chain of sub-edges. This turns
"one long edge exactly overlapping one short edge for part of its length" (which the graph has no way
to represent at all — nothing marks where the long edge should stop) into a set of edges that share
real vertices, which the existing tracer already handles correctly.

Tests: `a_rectangle_sharing_a_corner_with_a_larger_one_splits_it_without_a_tie`
([face_detect.rs:356](../src-tauri/src/geometry/face_detect.rs#L356), the corner case — two T-junctions,
one on each of two edges meeting at the shared corner) and
`a_point_partway_along_an_edge_splits_it_even_with_no_shared_vertex`
([face_detect.rs:388](../src-tauri/src/geometry/face_detect.rs#L388), a "slot notch" — two T-junctions on
the *same* edge, no shared vertex at all). Both operate directly on `detect_faces`, bypassing
`Document` entirely — the most precise level to test the graph algorithm at.

### 2. `Document::propagate_boundary_split_to_solid_siblings` — [document.rs:442](../src-tauri/src/scene/document.rs#L442)

Fix (1) alone only updates `face_id`'s own boundary loop. This mesh's faces **do not share topology**
— each face independently owns its own `Vec<VertexId>` loop (see `Mesh`'s struct doc comment,
[mesh.rs:27-33](../src-tauri/src/geometry/mesh.rs#L27-L33)). If `face_id` is already on a solid's boundary
(e.g. it's a box's top cap) and its edge gets split by (1), the *wall* below it — a completely
different `Face`, unrelated to this `resplit_loops` call — still has the old, unsplit edge, and that
edge now has no matching reverse-direction pairing anywhere in the mesh. That's an open edge:
`check_model`/`pushpull::is_manifold` correctly flag it, STL export correctly refuses it. This is
invisible in the flat 2D case (there's no adjacent wall to desync from), which is why it wasn't caught
by fix (1)'s own tests.

The fix: before erasing `face_id` (called at [document.rs:360](../src-tauri/src/scene/document.rs#L360),
right after `resplit_loops` succeeds and right before `erase_face`), find every point from
`extra_loops` that lands on a T-junction of one of `face_id`'s own rings (outer + each hole), then find
every *other* face in the same connected component (`Mesh::connected_components`) whose own ring
contains the matching edge in the opposite direction (this app's faces meeting at a shared edge always
traverse it in opposite directions under the CCW-outward winding convention), and splice the same new
vertex into that sibling's ring at the matching position. `just_created` (this same call's own
replacement faces — already built from the post-split graph) is explicitly excluded, or the function
would re-match against them and insert a duplicate, corrupting vertex into an already-correct boundary
(this was an actual bug hit and fixed during this session — see the empirically-derived debug trace in
the conversation transcript if you want the exact mechanism, not reproduced here).

Test: `pushing_a_corner_stud_sketch_into_a_real_stud_stays_watertight`
([document.rs:2242](../src-tauri/src/scene/document.rs#L2242)) — sketches the corner-flush rectangle,
then actually pushes it into a 3D stud and checks `check_model().broken_part_count == 0`.

### 3. `triangulate::on_segment_interior` — [triangulate.rs:170](../src-tauri/src/geometry/triangulate.rs#L170)

Found *after* (1) and (2) were both verified correct by hand (dumping the exact post-fix mesh state —
every face's vertex list — and manually confirming every directed edge paired with its reverse). Despite
that, `check_model` still reported open edges. Third, unrelated bug: after (2), a wall's boundary is a
pentagon with **three exactly-collinear consecutive vertices** (the new T-junction point sitting between
its two former neighbors). `triangulate.rs`'s ear-clipping decides whether an ear is safe to clip using
`point_in_triangle`, which is **deliberately strict** (non-boundary-inclusive) for an unrelated reason:
hole-bridging duplicates vertices exactly on the boundary by construction, and a boundary-inclusive test
would block every ear near a bridge seam. That same strictness has a second, unrelated failure mode: a
point sitting exactly *on* an ear's new diagonal isn't strictly *inside* the ear triangle, so the strict
test doesn't block the ear — the clipper cuts a diagonal straight past the collinear point instead of
being routed through it, producing a triangle edge that doesn't correspond to any real boundary edge.
That phantom edge is what `check_manifold` was actually reporting as open, even though the polygon's own
boundary loop (`face.outer`) was completely correct the whole time.

The fix is a second, narrower check specific to the ear's *new* `(prev, next)` edge only (not the other
two edges of the ear, which is where bridge duplicates legitimately sit) — added alongside the existing
`contains_other` check as `skips_a_collinear_point` at
[triangulate.rs:103-108](../src-tauri/src/geometry/triangulate.rs#L103-L108).

No dedicated unit test for `triangulate.rs` in isolation — it's covered only indirectly, through fix
(2)'s test seeing `check_model` go green. **This is a real coverage gap** (see checklist below).

## How each fix was actually found (why this took three passes, not one)

1. Fix (1) was found by testing the flat 2D case directly against `face_detect::detect_faces`.
2. Fix (2) was found by extending the *same* scenario one step further: instead of asserting on the
   flat split's face count, actually `push_pull` the result into a real stud and check watertightness.
   Fix (1) alone made that new test fail.
3. Fix (3) was found by adding fix (2), re-running, and getting a *different* failure (open edges) than
   expected — then dumping the full post-fix mesh state, manually verifying every edge pairs correctly
   on paper, and concluding the bug must be one level down, in triangulation rather than topology.

The methodological lesson, if you're auditing this: **each fix's own test only proves that fix's own
layer is correct in isolation.** The bug that motivated fix (3) would have been invisible to any test
that only inspects `face.outer`/`face.holes` — it only shows up when you additionally triangulate and
check edge-pairing. If you're looking for a fourth bug, look for the *next* layer down from these three
(rendering? STL export's own triangle iteration? — both currently just consume
`triangulate_face`'s output, so they *should* inherit the fix for free, but "should" is exactly the
word that was wrong twice already in this chain).

## What is and isn't covered — the actual checklist

Legend: ✅ tested and passing · 🧠 reasoned through but not exercised by a test · ❓ not addressed, flag
for you to judge.

- ✅ Corner-shared rectangle on a flat (non-solid) sketch face — splits correctly, 2 faces, no ties.
- ✅ T-junction with no shared vertex at all ("slot notch") — splits correctly, 2 faces, no ties.
- ✅ Corner-shared rectangle on a solid's boundary face, pushed into an actual 3D stud — watertight.
- ✅ The pre-existing "second stud on the same face" and "studs on two adjacent faces" scenarios
  (`a_second_stud_drawn_on_a_solid_keeps_the_first_stud_watertight`,
  `studs_on_two_adjacent_faces_snapped_to_the_same_corner_stay_watertight`) still pass unmodified.
- ✅ Full existing suite (arrange-for-print, mirror, array, inset, undo/redo, project-file round-trip,
  etc.) — 99/99 passing, no regressions from any of the three fixes.
- 🧠 **`resplit_plane` never calls `propagate_boundary_split_to_solid_siblings`.** Reasoning: `resplit_plane`
  already excludes `solid_face_ids` from its coplanar search
  ([document.rs:197](../src-tauri/src/scene/document.rs#L197)) and erases+rebuilds *every* coplanar face
  it touches together in one `resplit_loops` call, so any T-junction among those loops is self-contained
  — both sides of the split regenerate together, no stale sibling. This was reasoned through, not
  tested. **Worth a dedicated test**: draw two adjacent flat (non-solid) sketches via `resplit_plane`
  such that a third sketch creates a T-junction against the shared boundary, and confirm no open seam.
- ❓ **`propagate_boundary_split_to_solid_siblings`'s own doc comment claims** "a no-op for a `face_id`
  that isn't on a solid's boundary — a flat sketch's own edges aren't shared with anything else"
  ([document.rs:439-441](../src-tauri/src/scene/document.rs#L439-L441)). **This is very likely false.**
  Two flat, non-solid sketches produced by an earlier `resplit_plane` call (e.g. an L-shape and a square,
  from a prior corner-split) *do* share an edge with each other, and nothing stops a later
  `resplit_face_with_loops` call (a live `target_face_id`, e.g. the user clicks squarely on one of
  them) from creating a T-junction against that shared-but-not-solid edge. Since `propagate` early-returns
  for non-solid `face_id`, that sibling wouldn't be updated. Consequence is probably limited to a
  cosmetic hairline crack (check_model only scopes to `solid_boundary_face_ids()`, so this likely
  doesn't block STL export of anything not yet pushed into a solid) — but "probably limited to cosmetic"
  is a guess, not something checked, and the code comment currently overstates confidence. **Please
  verify**: does this scenario actually reproduce, and if so, what's the real blast radius (does it
  survive a later push/pull of that sketch into a solid, at which point it *would* matter)?
- ❓ **T-junction against a *different* face's edge, not `face_id`'s own.** `weld_loop_onto` (point
  coincidence, scoped to the whole connected solid via `connected_component_vertices`) handles a new
  point landing exactly *on* another face's existing vertex anywhere in the solid. But
  `split_edges_at_interior_points`/`propagate_boundary_split_to_solid_siblings` only check T-junctions
  against `face_id`'s **own** rings — never against some *other* face's edge in the same solid. Is this
  reachable? The `FACE_FIT_TOLERANCE` check keeps every new point within `face_id`'s own 2D boundary, so
  for a point to land mid-edge on a *different* face's edge while still being "inside" `face_id`, that
  other face would need to be coplanar with and inside `face_id`'s own footprint — which is exactly
  what a **hole rim** (a previous stud/recess's wall meeting `face_id` at that rim) is. `propagate`
  does include `face_id`'s holes in its own rings, so a T-junction landing on `face_id`'s hole boundary
  *is* covered by the same mechanism. But is there a configuration where the coplanar "other face" is
  neither `face_id` nor a hole-filling face of `face_id` — e.g. a stud built on an *adjacent* face whose
  own cap happens to be coplanar with and overlapping `face_id`'s plane? Work through whether this is
  geometrically reachable at all before spending time on a test; if it's genuinely unreachable given
  this app's draw tools, say so explicitly rather than leaving it as an open question for the next
  person too.
- ❓ **No dedicated `triangulate.rs` unit test** for the collinear-boundary case fix (3) addresses. It's
  only exercised transitively through fix (2)'s `check_model` assertion. Recommend adding a direct
  `triangulate_face` test: a pentagon with 3 exactly-collinear consecutive vertices (like the wall
  produced by this exact bug), asserting the resulting triangles' edges reconstruct the polygon's real
  boundary exactly (matches this report's hand-derivation in the conversation transcript, if useful as
  a reference — not reproduced here to avoid you just copying the conclusion instead of rederiving it).
- ❓ **Tolerance mismatch**: `FACE_FIT_TOLERANCE = 1e-2` ([document.rs:291](../src-tauri/src/scene/document.rs#L291))
  is 10x looser than `ON_SEGMENT_TOLERANCE = 1e-3` (face_detect.rs) and `Mesh::WELD_TOLERANCE = 1e-3`
  (used by both `propagate_boundary_split_to_solid_siblings` and `triangulate::on_segment_interior`).
  A point strictly between 1e-3 and 1e-2 away from an edge passes the "is this near enough to `face_id`
  to even attempt a resplit" check but fails every "is this a T-junction" / "is this the same point"
  check. What actually happens to that draw? Does it degrade gracefully (probably falls through as an
  ordinary new interior vertex, not on any edge, which should just work as a normal near-boundary point)
  or does it hit some other tie/degenerate case? Not tested.
- ❓ **Concave (already-split) target face.** All new tests target a simple rectangular face. Does a
  *third* sketch, drawn onto an already-L-shaped face (the result of a prior split) and creating a new
  T-junction against one of *its* (non-rectangular, concave) edges, still work? Ear-clipping's
  `is_convex` check is general-purpose and should handle concave polygons — but this specific
  interaction (concave outer boundary + T-junction + sibling propagation) hasn't been exercised.
- ❓ **Repeated/chained splits.** Two *separate* corner-stud sketches, drawn sequentially on the same
  original face (each its own `resplit_face_with_loops` call, the second one targeting whatever new
  face the first split left behind). Should work structurally (each call only ever sees the current,
  live face state), but not explicitly tested end-to-end.
- ❓ **`inset_face`'s interaction with fix (1).** `inset_face` also calls `resplit_face_with_loops`
  ([document.rs:256](../src-tauri/src/scene/document.rs#L256)) with an inward-offset loop. Normally its
  points move *away* from the boundary, so `splits` should end up empty and fixes (1)/(2) are inert for
  it — but this wasn't explicitly re-verified after the fixes landed, only inferred from the existing
  inset test suite staying green. If there's a near-zero offset edge case, check whether it could
  accidentally produce a spurious T-junction.
- ❓ **Undo.** `propagate_boundary_split_to_solid_siblings` mutates *other, pre-existing* faces' `outer`/
  `holes` vectors in place rather than creating new `FaceId`s. `History::record()` clones the whole
  `Document` before each command mutates, so undo should trivially restore the pre-split sibling ring —
  this follows from the existing architecture and wasn't separately tested, but is worth one manual
  Ctrl+Z check given it's a new *in-place mutation of an unrelated face* pattern that nothing else in
  this codebase does (every other operation either creates new faces or fully replaces a face's own
  loop, never reaches into a *different* face and edits part of its loop).

## Files touched by this specific fix (not the whole `measure-guides` branch)

- `src-tauri/src/geometry/face_detect.rs` — `split_edges_at_interior_points` + 2 new tests.
- `src-tauri/src/geometry/triangulate.rs` — `on_segment_interior` + the new `skips_a_collinear_point`
  check in `ear_clip`. No new dedicated test (see checklist).
- `src-tauri/src/scene/document.rs` — `propagate_boundary_split_to_solid_siblings`, its call site in
  `resplit_face_with_loops`, the rewritten `a_rectangle_sharing_a_corner_with_its_target_face_splits_it_correctly`
  test (previously asserted the no-op/rejection — now asserts the correct split), and the new
  `pushing_a_corner_stud_sketch_into_a_real_stud_stays_watertight` test.
- `CLAUDE.md` — the `resplit_loops`/`resplit_face_with_loops`/`resplit_plane` robustness bullet and the
  `geometry/triangulate.rs` bullet were both updated to describe all of the above.
- `docs/measure-guides-plan.md` — "Second follow-up" section has the same account in narrative form,
  written during the fix rather than after, if you want a second independent description to cross-check
  this one against.

## What "thoroughly check" should mean here

Don't just re-read this report and nod. Concretely:

1. Pick at least 2 of the ❓ items above and either write a failing test or convince yourself (in
   writing, in your own report back) why it's unreachable or harmless.
2. Independently re-derive at least one of the three fixes' correctness from the code, without relying
   on this report's explanation of *why* it works — the goal is a second, independent proof, not a
   restatement of the first one.
3. Actually run `cargo test` and `npx tsc --noEmit` yourself rather than trusting the "99 passing"
   claim above.
4. If you find a fourth bug, apply the same standard this report tried to hold itself to: don't just
   patch the symptom you found — ask what the *next* layer down might be doing with the same input, the
   way fix (2) led to fix (3).

---

## Audit results (second pass, independent reviewer)

Baseline re-verified independently before touching anything: `cargo test` 99 passing, `npx tsc
--noEmit` exit 0. Both claims above were accurate.

**Two further bugs found, both reachable from the workflow this feature exists to enable, both now
fixed with red/green tests.** Final state: **105 tests passing**, `tsc` clean, `cargo build` warning-free.

### Bug 4 — a sketch touching an existing stud's rim refilled that rim

`resplit_loops` protects a source face's existing holes from being re-materialized as faces, matching
the re-detected region against the recorded hole via `matches_any_loop` — which compared **exact
length**. Fix (1) breaks that assumption directly: when the new sketch's corner lands partway along a
*hole* edge (a stud's rim), `split_edges_at_interior_points` inserts a T-junction vertex into it, so
the region re-traces one vertex longer than the hole recorded moments earlier. The protection silently
stops matching and the rim is refilled with a face duplicating every edge the stud's wall already
paired with.

Reproduced at **5 duplicate + 8 open edges**, with the single-stud precondition asserted clean in the
same test, so the failure is attributable to the second sketch alone.

This is the report's own methodological point turned on its own fix: fix (1) was validated against a
face's *outer* boundary, and nothing asked what it did to a face's *holes*.

Fixed by splitting the comparison into two explicitly-named functions rather than loosening the shared
one — `loop_covers_any` (subset, for hole protection) and `matches_any_loop` (exact, retained for
`resplit_plane`'s "did an erased face fill this hole"). Loosening the shared helper, which was the
first thing I tried, silently changed the second call site's meaning too.

### Bug 5 — a face with two edge-adjacent holes triangulates into a partial mesh

Fixing bug 4 dropped the duplicates 5 → 1 but left 8 open edges. The cap's stored boundary loops were
by then **completely correct**; the failure was one layer further down, in triangulation — the same
shape of discovery that led from fix (2) to fix (3).

Cause: the cap legitimately ended up with two holes (the stud's rim, and the new sketch's own
footprint) that **share an edge**. `triangulate_face` bridges each hole to the nearest polygon vertex,
so the second bridge lands on a vertex the first hole already contributed — a zero-length bridge and a
self-touching polygon that ear-clipping cannot resolve. It bails out via its `if !clipped_an_ear`
escape hatch, emitting a partial triangulation (covering **200 of 276** units of area) whose edges no
longer reconstruct the boundary.

**This one is pre-existing, not caused by any of the three fixes** — confirmed by disabling fix (3)
and observing the identical 200/276. The fixes only made the configuration reachable, because
producing it requires exactly the collinear-edge alignment that `face_detect` used to reject.

Fixed at the source in `face_detect::merge_holes_sharing_an_edge`: two hole loops running along a
shared edge are fused into the single loop tracing their union. Two adjacent *regions* are correctly
two cycles, but as *holes of the face around them* they describe one connected uncovered area — two
loops there is wrong data, not an unusual encoding. Fixing the data rather than teaching
`triangulate_face` to cope also covers `pushpull` (which would otherwise raise one wall per loop and
duplicate the shared edge) and rendering/export, for free.

### Corrections to this report's own claims

- **Fix (3)'s coverage gap was worse than "no dedicated test".** My first attempt at the missing
  `triangulate.rs` test *passed with fix (3) disabled* — false confidence rather than coverage. Two
  reasons, both worth knowing before writing another one: (a) **asserting on area cannot catch this
  bug at all**, because the triangle the clipper skips over is degenerate and is dropped, leaving the
  total area correct — only the **edges** reveal it; (b) **vertex order decides whether the bug
  fires** — with the collinear triple adjacent to the ring's starting vertex, ear-clipping consumes
  ears in an order that never proposes the bad diagonal. The committed test
  (`a_collinear_t_junction_vertex_is_never_skipped_by_a_triangle_edge`) is verified red/green.
  Fix (3) is genuinely necessary; the report was right, but for reasons it hadn't demonstrated.
- **The ❓ about `propagate`'s non-solid early-return is resolved, and the doc comment was wrong** —
  though the conclusion is benign. Flat sketches left adjacent by `resplit_plane` *do* share edges, so
  the comment's "a flat sketch's own edges aren't shared with anything else" is false. But nothing
  pairs edges *across* two flat sketches: each extrudes into its own independently closed solid built
  from its own loop, so the stale edge never becomes a manifold error. Verified by test, and the doc
  comment now states the accurate reason.
- **The ❓ about T-junctions against a *different* face's edge is unreachable**, as the report
  suspected but left open. `FACE_FIT_TOLERANCE` confines every new point to `face_id`'s own 2D
  boundary, so another face's edge can only be hit where that face is coplanar with and inside
  `face_id`'s footprint — which is precisely a hole rim, and `propagate` already walks `face_id`'s
  holes alongside its outer. A wall meeting `face_id` does so *at* one of `face_id`'s own boundary
  edges, likewise covered.
- **The ❓ tolerance mismatch is harmless.** A point 1e-3..1e-2 from an edge fails the T-junction and
  weld tests but passes the fit check, and simply becomes an ordinary interior vertex — a genuinely
  thin sliver face, which is what was drawn. Not reachable from snapping in practice: f32 round-trip
  error at this app's ~100 mm coordinates is ~1e-5, three orders below the 1e-3 threshold.
- **The ❓ `inset_face` interaction is inert**, as suspected. Offsets below the weld tolerance collapse
  onto the source boundary and are absorbed before `propagate` sees them; larger ones sit clear of it,
  so `splits` comes back empty either way.

### Added coverage

| Test | Guards |
| --- | --- |
| `a_sketch_touching_an_existing_studs_rim_does_not_refill_that_rim` (document.rs) | Bug 4 + bug 5 end-to-end; also re-splits the *merged* hole a third time, confirming the merge didn't break hole protection |
| `two_adjacent_inner_regions_become_one_merged_hole_not_two` (face_detect.rs) | Bug 5 at the unit level |
| `a_collinear_t_junction_vertex_is_never_skipped_by_a_triangle_edge` (triangulate.rs) | Fix (3), verified red/green — the report's missing test |
| `a_second_corner_stud_on_the_resulting_concave_face_stays_watertight` (document.rs) | The ❓ concave-face and chained-split items together |
| `a_t_junction_on_an_edge_shared_between_two_flat_sketches_survives_extrusion` (document.rs) | The ❓ non-solid sibling item |
| `undoing_a_corner_stud_sketch_restores_the_neighboring_walls_split_edge` (commands.rs) | The ❓ undo item — `propagate` is the only in-place edit of an otherwise untouched face |

### Still not verified

- **No live GUI pass after these two fixes.** The user's manual pass predates them. Bug 5's symptom is
  visual (a chunk of a face missing on screen and in exported STL), so it is worth one run of the
  corner-stud → stud-rim sequence before merging.
- **Older `project.json` files** saved by a build between fix (1) and this audit could contain a face
  with two edge-adjacent holes already baked in. Loading one reproduces bug 5's symptom, since the
  merge now happens in `face_detect` rather than on load. Judged not worth migration machinery — the
  window is one uncommitted branch that never shipped — but it is the reason to merge this before
  cutting any release from `measure-guides`.
- **Holes meeting at a single vertex** (rather than along an edge) are deliberately not merged; they
  pinch to a figure-eight no simple loop represents. No current draw tool produces that without also
  sharing an edge, so it is unreachable today rather than fixed.
