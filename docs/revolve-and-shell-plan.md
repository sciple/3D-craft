# Revolve + Shell tools

## Context

3D-craft today has a complete SketchUp-style loop (sketch → push/pull → inset, plus
move/rotate/scale/duplicate/mirror, groups, undo/redo, project save-load, manifold-checked STL
export). For its mission — *modeling small parts to 3D print* — two capability gaps stand out:

- **No revolve/lathe**: round/turned parts (knobs, nozzles, vases, spindles) can't be made from a
  profile, even though the plane+profile infrastructure already exists.
- **No shell/hollow**: solids can't be hollowed to a wall thickness to save material/weight and
  make printable enclosures.

Both fit the existing architecture cleanly (new geometry ops in the Rust core, mirroring
`push_pull`; no new selection model, no CSG engine). This plan adds a **Revolve** tool (pick a
profile face + one of its boundary edges as the axis, sweep to an angle) and a **Shell** tool
(hollow a solid to a wall thickness, opening the selected face(s)).

Decisions locked with the user:
- **Revolve axis** = one of the profile's own boundary edges (classic lathe; guarantees the axis
  is coplanar with the profile, and the axis edge collapses to the lathe pole line — no external
  axis / torus case to handle).
- **Shell** opens the **selected/clicked face(s)** (cup/box/enclosure), not a fully-sealed cavity.

Follow the repo's guidance: *favor the simplest correct implementation over generality*. Both v1
tools target the common prismatic/turned parts this app produces, with documented limitations.

---

## Part 1 — Revolve (backend)

### New file: `src-tauri/src/geometry/revolve.rs`

Mirror `pushpull.rs`'s structure and doc-comment density. Core function:

```rust
pub fn revolve(
    mesh: &mut Mesh,
    face_id: FaceId,
    axis_a: DVec3,        // one endpoint of the picked axis edge
    axis_b: DVec3,        // the other endpoint
    angle_radians: f64,   // sweep angle; TAU for a full lathe
    segments: usize,      // angular subdivisions of the full sweep
) -> Vec<FaceId>          // empty on rejection (mirrors push_pull's no-op-on-invalid)
```

Algorithm:
1. Clone the source `Face`. **v1 rejects a profile with holes** (return empty) — a holed lathe
   profile is out of scope. `dir = (axis_b - axis_a).normalize()`; reject if degenerate.
2. In-plane side test using `side = face.normal.cross(dir)`: for every outer-loop vertex `P`
   compute `dot(P - axis_a, side)`. All must share one sign (endpoints of the axis edge sit at
   ~0, allowed). If the profile **straddles** the axis, return empty (would self-intersect).
3. Build a vertex grid `ring[k][i]` for `k in 0..=nsteps` (`nsteps` = `segments` scaled to the
   sweep angle; for a full `TAU`, ring `nsteps` ≡ ring `0` so reuse it instead of duplicating).
   Rotate each outer vertex about the axis line: `axis_a + DQuat::from_axis_angle(dir, theta_k) *
   (P - axis_a)`. **Dedupe axis vertices** (radial distance < eps): they map to one shared
   `VertexId` reused across all `k` (this is what makes the axis edge collapse to a pole and turns
   its neighboring quads into pole-fan triangles).
4. For each outer edge `(P_i, P_{i+1})` and each step, emit a face from the four grid corners;
   **skip edges whose both endpoints are on the axis** (fully degenerate), and drop coincident
   corners so a quad with one axis endpoint becomes a triangle.
5. **Caps** (only when `angle_radians < TAU - eps`): add the profile loop at `theta=0` and at
   `theta=angle` as two cap faces.
6. **Winding** — use a robust "flip if inward" rule instead of hand-derived sign math (the
   codebase already reasons this way; `is_manifold` is the oracle). For each generated face
   compute geometric normal `N` and centroid `C`, and a reference outward `O`:
   - side/pole faces: `O` = radial component of `(C - axis_a)` perpendicular to `dir`;
   - start cap: `O = -(dir.cross(C - axis_a))`; end cap: `O = +(dir.cross(C - axis_a))`.
   Reverse the loop before `add_face` if `N.dot(O) < 0`. Preserves the CCW-outer invariant.
7. `mesh.remove_face(face_id)` (as `push_pull` does); return the new face ids.

### `Document::revolve_face` in `scene/document.rs`

Model it on `push_pull` (lines 262-304): read `was_grouped`, call `revolve`, add all new faces to
`solid_face_ids` (a revolve always produces a closed solid), re-attach the source face's group
membership onto the new faces, and drop the source from selection/group/solid bookkeeping. Return
the new face ids. Do the side/degeneracy validation here too (belt-and-suspenders) so a bad call
is a clean no-op.

### Command + registration

- `commands.rs`: `revolve_face(state, face_id, axis_a: DVec3, axis_b: DVec3, angle_radians: f64,
  segments: usize) -> DocumentSnapshot`. Call `history.record()` then mutate — same shape as
  `push_pull_face` (commands.rs:131-137).
- `lib.rs`: add `commands::revolve_face` to the `generate_handler!` list (after `push_pull_faces`).
- `geometry/mod.rs`: add `pub mod revolve;`.

### Rust tests (inline `#[cfg(test)]` in `revolve.rs`)

- Rectangle with one edge on the axis, full `TAU` → `is_manifold`; expected face count
  (`segments` side bands + pole fans, no caps).
- Same profile, partial 90° sweep → `is_manifold`, and exactly two extra cap faces vs. the full case.
- Profile straddling the axis → returns empty.
- (In `document.rs` tests) `revolve_face` puts every new face in `solid_face_ids` and carries the
  source group.

---

## Part 2 — Shell / hollow (backend)

### New file: `src-tauri/src/geometry/shell.rs`

Reuses the **shared old→new vertex map + reversed clone** pattern already proven in
`clone_faces_mapped` / `mirror_faces` (document.rs:353-419). Core function:

```rust
pub fn shell(
    mesh: &mut Mesh,
    solid_faces: &[FaceId],   // the full closed solid (caller supplies the whole component)
    open_faces: &[FaceId],    // subset to leave open
    thickness: f64,
) -> Result<Vec<FaceId>, String>   // Err(msg) e.g. "wall too thick" — surfaced to the user
```

Algorithm:
1. **Inner-vertex map** (plane-offset inset of the polyhedron): for each unique `VertexId` in the
   solid, gather the faces meeting at it, form each face's inward-offset plane (`normal`,
   point `p - normal*thickness`), and solve for the inner vertex as the intersection of those
   planes (3-plane solve; least-squares if >3). Exact for right-angled/prismatic corners (the
   push/pull output this mostly targets); acceptable approximation for faceted cylinders.
   Store `old_vertex -> inner VertexId`.
2. **Validity check**: if any inner vertex lands on the wrong side of its faces (offset crossed
   through the part — thickness too large) → `Err("wall thickness too large for this solid")`.
3. **Inner shell**: for every solid face *except* the opened ones, clone the loop through the
   inner-vertex map with the loop **reversed** (inner surface faces the cavity), preserving the
   winding invariant — exactly like `mirror_faces` reverses reflected loops.
4. **Openings**: for each opened face, remove the outer face and skip its inner face; add a **rim
   band** — a wall quad per outer-loop edge connecting the outer edge to its inner counterpart —
   so the opening shows wall thickness and the shell stays watertight.
5. Keep all non-opened outer faces as-is. Result = outer shell (minus openings) + reversed inner
   shell (minus openings) + rim bands = a closed, watertight hollow. Return the new/added face ids
   (inner faces + rim bands).

**v1 limitations to document in the doc-comment**: single closed solid only; concave/reflex
corners may produce imperfect inner geometry (plane-offset, not a true 3D straight skeleton);
opening non-adjacent faces on a concave solid is out of scope.

### `Document::shell_solid` in `scene/document.rs`

- Expand the user's `open_faces` to the **full solid**: flood-fill `solid_face_ids` by shared
  `VertexId` starting from the opened faces (add a small `solid_component_of` helper). Reject if
  the opened faces aren't part of a solid (e.g. a bare sketch face) → `Err`.
- Call `shell`; on `Ok`, register the new faces in `solid_face_ids`, propagate the solid's group
  membership onto them, remove the opened faces from selection/group/solid bookkeeping. Return
  `Result<(), String>`.

### Command + registration

- `commands.rs`: `shell_solid(state, open_face_ids: Vec<FaceId>, thickness: f64) ->
  Result<DocumentSnapshot, String>` — `history.record()` then mutate; return the snapshot on
  success, propagate the `Err` string on rejection (same `Result` shape as `export_stl`,
  commands.rs:276).
- `lib.rs`: add `commands::shell_solid` to `generate_handler!`.
- `geometry/mod.rs`: add `pub mod shell;`.

### Rust tests (inline in `shell.rs`)

- Shell a `push_pull`'d unit box with the top face open → `is_manifold`; inner vertices offset
  inward by `thickness`; expected face count (5 outer + 5 inner + 4 rim).
- Thickness ≥ half the box extent → `Err`.
- (document.rs) `shell_solid` given a sketch (non-solid) face → `Err`; success path registers new
  faces in `solid_face_ids`.

---

## Part 3 — Frontend

### `src/tools/revolve-tool.ts` (new; `Tool` interface, shortcut **V**)

Two-click + angle, following the face-picking pattern in `pushpull-tool.ts`/`plane.ts`
(`ctx.meshRenderer.faceIdForTriangle`):
1. **Click 1** → profile face. Store its id; read its `outer` loop world positions + `normal`
   from `documentStore.getSnapshot()`. Highlight it.
2. **Click 2** → axis edge: of the profile's `outer` edges, pick the segment nearest the mouse ray
   (ray-vs-segment distance). Lock `axisA/axisB`; highlight the chosen edge. Client-side run the
   same one-side-of-axis check and, if it fails, show a HUD message and don't arm.
3. **Angle** via `NumericBuffer` in **degrees** (reuse `tools/numeric-input.ts` +
   `ui/measurement-hud.ts`, as every other tool does). **Default 360°** so the common path is
   click-face → click-edge → Enter. Optional drag sets the angle by projecting the pointer onto
   the plane ⊥ the axis. Lightweight **wireframe preview**: the profile loop rotated to a few
   intermediate angles (rings), plus the angle in the HUD — avoids porting the full triangulation
   to TS while still reading as a revolve.
4. Commit (release / Enter) → `documentStore.revolveFace(faceId, axisA, axisB, angleRadians,
   segments)`. Use the same segment count the circle draw tool uses for visual consistency.

### `src/tools/shell-tool.ts` (new; `Tool` interface, shortcut **H**)

Selection-aware like the other face tools (CLAUDE.md's shared pattern): the selected face(s) — or
the clicked face if none selected — are the faces to **open**.
1. Click/confirm the open face(s).
2. **Thickness** via drag inward along the clicked face normal (reuse
   `axis-drag.ts::closestDistanceAlongAxis`, as inset/pushpull do) and/or typed `NumericBuffer`
   value; show it in the HUD; highlight the opened faces. (v1 preview = highlight + HUD number, no
   full ghost.)
3. Commit → `documentStore.shellSolid(openFaceIds, thickness)`. On the rejected-promise `Err`,
   surface the message the same way the file menu surfaces export/save errors.

### Wiring

- `src/state/document-store.ts`: add `revolveFace(...)` and `shellSolid(...)` wrappers following
  the existing `enqueue` + `apply(await invoke<DocumentSnapshot>(...))` pattern (lines 148-160).
  `shellSolid` handles the `Result` error (let the promise reject so the tool can catch it).
- `src/ui/icons.ts`: add `revolve` and `shell` SVG icons (match the existing icon style).
- `src/main.ts` + toolbar: instantiate both tools and add two `createToolbar` entries —
  `{ tool: revolveTool, label: "Revolve", shortcut: "v", icon: icons.revolve }` and
  `{ tool: shellTool, label: "Shell", shortcut: "h", icon: icons.shell }`. (V and H are unused
  today; existing shortcuts are s/r/c/l/p/i/g/m/t.) The draw tools' `returnToSelect` callback
  pattern doesn't apply — these behave like push/pull (stay active after commit).

---

## Verification

1. **Rust**: `cd src-tauri && cargo test` — new `revolve`/`shell` tests + all existing pass
   (crucially the `is_manifold` assertions).
2. **Types**: `npx tsc --noEmit` clean.
3. **End-to-end** (`npm run tauri dev`):
   - **Revolve**: draw a rectangle on the ground → Revolve (V) → click the face, click its bottom
     edge, press Enter → a cylinder; try an L-shaped polygon profile → a stepped spindle; try
     typing `90` → a quarter wedge with caps. Export STL — must succeed (manifold check passes).
   - **Shell**: draw a rectangle → Push/Pull into a box → Shell (H) → select the top face, type
     `2`, Enter → an open-topped box with 2 mm walls. Orbit inside to confirm the cavity + rim;
     export STL (manifold passes). Then try an absurd thickness → the error message shows and the
     model is unchanged.
   - **Undo/redo**: Ctrl+Z reverts a revolve/shell in one step (both call `history.record()`).

## Out of scope (v1)

Revolve of holed profiles; external-axis revolve (torus); shell of concave solids with reflex
corners or non-adjacent openings; a full solid-of-revolution live preview (wireframe only).
