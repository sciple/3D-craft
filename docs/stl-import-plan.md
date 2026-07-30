# Import STL

## Context

The app can currently only *write* STL (`src-tauri/src/io/stl_export.rs`); there is no reader
anywhere. This plan adds the ability to open an existing STL and actually model on it, not just
display it.

The load-bearing piece already exists: `src-tauri/src/geometry/face_detect.rs`'s `detect_faces`
reconstructs polygonal faces-with-holes from a coplanar edge graph. So an STL triangle soup can be
welded and merged *back* into real editable `Face`s rather than staying a soup. A 3D-printed
bracket imports as ~10 clean polygons that Push/Pull, Inset, Move, group, and Export STL all
operate on normally.

Honest limit, stated up front: this works for **flat-faced parts**, which is what this app targets.
**Curved or organic meshes** (spheres, scans, sculpts) have no coplanar triangles to merge, so they
import as one face per triangle — they render, move, and re-export correctly, but per-face editing
on them is not meaningful. Hence the >50k-triangle warning.

Decisions locked with the user:
- **Full editable import** (not raw triangles, not a visualize-only reference object).
- **Replace the current document** on import, like Open Project.
- **Warn above ~50k triangles and let the user proceed** — never hard-refuse.

Follow the repo's guidance: *favor the simplest correct implementation over generality*.

---

## Two verified facts that shape the design

**(a) `detect_faces` returns a hole-filling face per hole.** `trace_ccw_loops` keeps *every*
positive-area cycle, so a ring-shaped cluster comes back as two `DetectedFace`s: the ring *and* the
disc filling its hole. This is asserted by `face_detect.rs`'s
`nested_square_becomes_ring_hole_and_own_face` test. Emitting both would cover every hole in the
imported model with a bogus face and triple up the rim edges, breaking manifoldness. **The merge
step must drop any detected face whose `outer` matches another's hole loop** — same shape as
`Document::matches_any_loop` (`document.rs:881`). Safe within a cluster: an island inside a hole is
never edge-connected to the ring around it, so it is always a *different* cluster.

**(b) Winding comes out correct, no negation.** `Plane::from_normal` builds `u = v.cross(normal)`
so `u × v = normal` (`plane.rs:26`, asserted `plane.rs:58`). A positive-area 2D loop therefore
lifts to a 3D polygon whose Newell normal is `+normal`, and `detect_faces` only keeps positive-area
loops — so feeding its `outer` straight to `add_face` yields the cluster's own normal. **Still add
an explicit guard**, because a globally-inverted face set passes `check_manifold` (edge pairing is
symmetric under a global flip), so a sign error here would silently export inside-out models.

---

## Part 1 — STL parsing

### New file: `src-tauri/src/io/stl_import.rs`

Hand-rolled to match the hand-rolled exporter — a reader is ~120 lines, less surface than adding a
crate.

```rust
pub fn parse_stl(bytes: &[u8]) -> Result<Vec<[DVec3; 3]>, String>;
pub fn triangle_count(bytes: &[u8]) -> Result<u32, String>;
pub fn is_binary_length(declared_count: u32, len: u64) -> bool;
```

Return a bare triangle soup — per-facet normals are deliberately discarded (winding is
authoritative; `write_binary_stl`'s doc comment already establishes stored normals aren't trusted).

**Format detection — the "solid" trap.** Check the binary length identity *first*:

```
binary_exact = len >= 84 && len == 84 + 50 * u32_le(bytes[80..84])
if binary_exact                      -> parse_binary   // beats a header starting with "solid"
else if starts_with_solid_token && is_probably_text(&bytes[..512])  -> parse_ascii
else if len >= 84 + 50*declared      -> parse_binary   // lenient: trailing junk
else                                 -> Err(truncated/corrupt, with declared vs actual counts)
```

`is_probably_text` = first 512 bytes all `\t\r\n` or `0x20..=0x7e` (a NUL-filled binary header
fails). The residual ambiguity (a text file whose bytes 80–84 coincidentally satisfy the length
identity) is ~0 probability — document in a comment, don't add a heuristic that could misfire the
other way.

**Totality is mandatory.** `document.rs:776-786` documents the rule: a panic while holding the
`AppState` mutex poisons it and bricks the app. So: `declared` is only trusted *after* the length
check (no allocation bomb from `with_capacity`), slice access via `get(..)` even where bounds are
pre-proven, non-finite floats dropped, `String::from_utf8_lossy` for ASCII (never `Err`).

ASCII parsing is a tolerant whitespace-token scan: on `vertex`, take 3 tokens and `parse::<f64>()`;
ignore every other keyword. A `vertex` with unparseable numbers or a trailing partial triangle →
`Err` (silently importing half a model is worse than refusing). Zero triangles → `Err`.

Add `pub mod stl_import;` to `src-tauri/src/io/mod.rs`.

---

## Part 2 — Triangle soup → editable faces

### New file: `src-tauri/src/geometry/reconstruct.rs`

Keeps the algorithm out of the already-1938-line `document.rs`, and out of `io/` (it's geometry,
reusable if OBJ import ever lands). Add `pub mod reconstruct;` to `geometry/mod.rs`.

```rust
pub struct ReconstructedFace { pub outer: Vec<u32>, pub holes: Vec<Vec<u32>>, pub normal: DVec3 }
pub struct ReconstructedMesh { pub positions: Vec<DVec3>, pub faces: Vec<ReconstructedFace> }
pub fn reconstruct(triangles: &[[DVec3; 3]]) -> ReconstructedMesh;  // total, never panics
```

Index-based rather than returning a `Mesh`, so `document.rs` reuses `from_project_file`'s
index-stable interning idiom and stays the only place touching private `solid_face_ids`.

### 2a. Weld (required — nothing downstream works without it)

`Mesh::connected_components` (`mesh.rs:80`) and `pushpull::check_manifold` define adjacency purely
by **shared `VertexId`**, so unwelded soup reports as N separate open parts and fails the export
gate.

**Epsilon, relative not fixed** — STL stores `f32` (~1e-7 relative noise), and a fixed `1e-4 mm`
would destroy a model authored in metres:

```rust
let eps = (bbox_diagonal * 1e-6).clamp(1e-9, 1e-3);
```

**Key: spatial hash `HashMap<(i64,i64,i64), Vec<u32>>` on `floor(p/eps)`, probing all 27
neighbouring cells.** Naive quantize-only is not sufficient here: exporters that recompute a shared
corner per-facet produce last-bit-differing values that straddle cell boundaries, and a *single*
missed weld breaks connectivity, manifoldness, and the coplanar merge for that whole region. The
27-cell probe is ~15 lines and milliseconds at 50k triangles.

After welding, **drop triangles whose 3 indices aren't distinct**. Not cosmetic: a repeated vertex
makes `face_detect::angle_at` compute `atan2(0,0)` and corrupts the half-edge rotation order.

### 2b. Coplanar clustering — union-find over shared edges

Key simplification: **two triangles sharing an edge with parallel normals are necessarily
coplanar** (they share a whole line). So there is **no plane-offset test and no quantized plane
key** — only a normal test on edge-adjacent pairs, which removes the fragile-at-the-boundary
tolerance class entirely.

```
edge_map: HashMap<(min,max) welded pair, Vec<tri>>
for each edge, for each pair (i,j): if normals[i].dot(normals[j]) > COPLANAR_DOT { uf.union(i,j) }
```

- **Direction must agree — `dot > +cos θ`, never `.abs()`.** Two back-to-back faces (a thin wall, a
  zero-thickness fin) share rim edges with exactly opposite normals; merging them fuses front and
  back into nonsense.
- `COPLANAR_ANGLE_DEG = 0.1` → `COPLANAR_DOT = 0.999_998_476`. Tight, because the failure modes are
  asymmetric: *under*-merging just leaves a flat region as a few extra correct faces;
  *over*-merging makes a non-planar "face" whose 2D projection is distorted and whose triangulation
  can self-intersect. 0.1° also refuses any realistic curve (a 32-segment cylinder is 11.25°/facet).

### 2c. Per cluster: boundary → `detect_faces`

1. `normal` = area-weighted mean of cluster triangle normals (slivers don't drag it); `origin` =
   cluster vertex centroid (keeps 2D coords well-conditioned).
2. **Planarity check**: `max |(p-origin)·normal| <= max(1e-6, diag*1e-5)`, else → fallback (2e).
3. **Boundary edges** = cluster edges used by exactly one cluster triangle. Undirected;
   `detect_faces` re-derives winding.
4. **Compact indexing**: collect *boundary* vertices only into `Vec<u32>` + `HashMap<u32,usize>`;
   project via `plane.to_2d`. Boundary-only keeps indices dense and in-range, which is what keeps
   the `.expect` at `face_detect.rs:77` and the `outgoing[a]` indexing safe.
5. `detect_faces(&points, &edges)`.
6. **Drop hole-fill artifacts** (fact (a)): build a `HashSet<BTreeSet<usize>>` of all returned hole
   loops; discard any face whose `outer` is in it.
7. Map local indices back through `boundary_verts[local]`.
8. **Orientation guard** (fact (b)): `newell_normal` the lifted outer loop; if
   `dot(cluster_normal) < 0`, reverse `outer` and every hole.
9. Drop faces with `outer.len() < 3` and hole loops with `< 3`.

### 2d. Collinear simplification — global, not per-loop (sequence last)

T-junctions leave long collinear runs that bloat loops and stress the O(n²) ear clipper.

**The naive per-loop version is a manifoldness bug**: dropping a collinear vertex from face A while
neighbour B keeps it un-pairs that edge and silently blocks re-export. Instead: mark `keep[v]`
globally across *all* loops where `sin θ > 1e-3` at any occurrence, then apply the removal set to
every loop — so a genuine T-junction (flat on A, a corner on B) survives in both. Never shrink a
loop below 3.

**Do this in a separate commit after the round-trip tests are green** — it's the step most likely to
introduce a subtle regression, and it's optional.

### 2e. Fallback — never lose geometry

If the planarity check fails, `detect_faces` returns nothing usable, or the candidate polygon fails
a triangulation smoke test (`triangulate_face` returns empty — `ear_clip` *breaks out* on
self-intersecting loops at `triangulate.rs:99` and silently under-produces), emit **each cluster
triangle as its own 3-vertex face**. That's exactly the input geometry: always valid, preserves
whatever manifoldness the source had.

---

## Part 3 — Document construction

### `Document::from_stl_triangles` in `src-tauri/src/scene/document.rs`

Lives here because `solid_face_ids`/`face_to_group` are private. Thin — mirrors
`from_project_file` (`document.rs:787`) line for line.

```rust
pub fn from_stl_triangles(triangles: &[[DVec3; 3]], group_name: Option<String>) -> Self
```

`reconstruct()` → `Vec<Option<VertexId>>` gated on `pos.is_finite()` (index-stable, as
`document.rs:799`) → `resolve()` with `len() >= 3` filters → `add_face` → **`solid_face_ids.insert(face_id)`
unconditionally** → `group_faces(&ids, name)`.

**Marking every face solid is mandatory.** `resplit_plane` (`document.rs:172`) excludes solid faces
from its document-wide coplanar sweep; a non-solid imported face would be swept into and destroyed
by the next unrelated coplanar sketch. It's also what `solid_boundary_face_ids()`, export,
`check_model`, and `arrange_for_print` are scoped to.

**Group: yes** — one group named after the file stem (fallback `"Imported STL"`). Free, and it's
what makes the import immediately usable: the outliner lists it, one click selects the whole thing
for Move/Rotate/Mirror.

---

## Part 4 — Commands and registration

In `src-tauri/src/commands.rs`:

```rust
#[tauri::command] pub fn stl_triangle_count(path: String) -> Result<u32, String>
#[tauri::command] pub fn import_stl(state: State<AppState>, path: String) -> Result<DocumentSnapshot, String>
```

`import_stl` goes **further than `load_project`'s lock discipline** — build the whole document
*before* taking the lock, since reconstruction is seconds on a large mesh and touches no shared
state:

```rust
let bytes = std::fs::read(&path).map_err(|e| e.to_string())?;
let triangles = stl_import::parse_stl(&bytes)?;
let name = Path::new(&path).file_stem().map(|s| s.to_string_lossy().into_owned());
let document = Document::from_stl_triangles(&triangles, name);   // outside the lock
let mut history = state.0.lock().unwrap();
history.record();                                                 // Ctrl+Z undoes the import
history.document = document;
Ok(history.document.snapshot())
```

Note that difference in the doc comment so a later refactor doesn't "normalize" it back to
`load_project`'s shape.

`stl_triangle_count`: `fs::metadata` for length + read 84 bytes; if `is_binary_length(declared, len)`
return `declared` (O(1), no body read); else read fully and count `facet` tokens.

Register both in `generate_handler!` in `src-tauri/src/lib.rs`. **No capability change** —
`dialog:default` is already granted in `src-tauri/capabilities/default.json` and all file I/O is
Rust-side.

---

## Part 5 — Frontend

**`src/state/document-store.ts`** — `importStl(path)` and `stlTriangleCount(path)`, both through
`enqueue`. Use **`applyEdit`, not `apply`**: `loadProject` sets `dirty = false` because the document
matches a file on disk, but an imported mesh corresponds to no saved project, so it is genuinely
dirty and the close-guard must prompt. `applyEdit` also clears the stale problem overlay for free.

**`src/ui/icons.ts`** — `importStl`: the `exportStl` glyph with the arrow reversed, so the pair
reads as a set.

**`src/ui/file-menu.ts`** — button after Open Project, before Export STL.
`open({ title: "Import STL", multiple: false, filters: [{ name: "STL", extensions: ["stl"] }] })`
with the same `Array.isArray` guard as `handleOpen`.

**Threshold via the separate count command**, not a `confirmed: bool` param and not
import-then-warn. Import-then-warn is disqualified — the document is already replaced and the work
already done, and gating is the warning's whole purpose. A `confirmed` flag would bake the constant
into the backend and need a richer return envelope than every other command uses. The count command
keeps `import_stl` a plain `Result<DocumentSnapshot, String>` and leaves threshold/wording in the
frontend with the other dialogs. TOCTOU on a just-picked local file is not a real concern.

```ts
const IMPORT_WARN_TRIANGLES = 50_000;
// > threshold: ask(..., { kind: "warning", okLabel: "Import Anyway" }), explaining that
// curved surfaces can't merge and stay high-face-count. Never refuse.
```

**No unsaved-changes prompt before replacing** — matches `handleOpen`, and the import is
`record()`ed so Ctrl+Z restores the previous document.

**Post-import watertight check: yes, but silent when clean.** Run `checkModel()` after import; if
`reportHasProblems`, reuse `handleExportStl`'s exact `ask(..., okLabel: "Show Me")` pattern. Wild
STLs are frequently non-manifold, and without this the first hint is a refused export much later
with no connection to the import. Does *not* auto-paint the model red.

Errors: `alert()` / dialog `message`, matching existing style (there is no toast system).

---

## Part 6 — Camera framing (required, not optional)

`src/viewport/controls.ts:131` clamps zoom radius to `[0.2, 2000]` and there is no fit-to-view
anywhere; `target` only moves by panning. **An STL larger than ~2000 units, or centred far from the
origin, imports invisibly with no way to reach it** — the most likely "I imported and nothing
happened" report, which would undercut the whole feature.

Add to `CadCameraControls`:

```ts
frameBounds(min: THREE.Vector3, max: THREE.Vector3): void
```

Sets `target` to the bbox centre and `radius` to fit the bbox for the current fov, and raises a new
`private maxRadius` field (default 2000, used by `onWheel`'s clamp) to
`Math.max(2000, fitRadius * 4)` so the user can still zoom back out afterwards.

Call it from `src/main.ts` after a successful import, computing the bbox in TS from
`snapshot.vertices` — no backend change needed.

---

## Part 7 — Tests

Inline `#[cfg(test)] mod tests` per repo convention (no `tests/` dir).

**`stl_import.rs`**: binary round-trip of an exported cube → 12 triangles; **binary file whose
header starts with `"solid"` still parses as binary**; ASCII literal parses with exact coords;
CRLF/leading-whitespace tolerance; truncated binary → `Err` not panic; trailing junk still parses;
`&[]` / garbage → `Err` not panic; ASCII `vertex` missing a number → `Err`; non-finite f32s dropped
not panicked on; `triangle_count` agrees with `parse_stl` for both formats.

**`reconstruct.rs`**: weld merges per-facet-repeated corners; **weld merges two vertices `eps/10`
apart that straddle a lattice boundary** (this is the test that fails with a quantize-only key);
cube soup → 6 quads / 8 positions; **merged face normals dot-positive with their source triangles**
(guards fact (b)); perpendicular edge-sharing triangles not merged; **back-to-back opposite
triangles not merged**; **square annulus → exactly 1 face with 1 hole, not 2 faces** (guards fact
(a)); NaN/degenerate input → no panic; collinear points dropped consistently across loops.

**`document.rs`**: `from_stl_triangles` marks every face solid and creates the group; **cube
round-trip** (`draw_rectangle` + `push_pull` → `write_binary_stl` → `parse_stl` →
`from_stl_triangles`) → 6 faces, 8 vertices, `is_manifold`; **imported cube normals point outward**
(the silent-inversion guard); tube round-trip preserves its `holes` loop and stays manifold;
**16-segment cylinder round-trip stays 18 faces** (fails loudly if `COPLANAR_DOT` is ever loosened
past 11.25°); **imported faces survive a coplanar sketch drawn elsewhere** (the
`solid_face_ids`/`resplit_plane` interaction).

---

## Part 8 — Order of work

1. `io/stl_import.rs` + `io/mod.rs` + tests → `cargo test` green first.
2. `geometry/reconstruct.rs`: weld → clustering → boundary → `detect_faces` → hole-fill filter →
   orientation guard → fallback. **Defer 2d (collinear) to its own commit.**
3. `Document::from_stl_triangles` + round-trip/manifold/normal tests.
4. `commands::import_stl` + `stl_triangle_count` + `lib.rs` → `cargo build`.
5. `document-store.ts`, then `icons.ts` + `file-menu.ts`, then `controls.ts` `frameBounds` +
   `main.ts` wiring → `npx tsc --noEmit`.
6. `README.md` (File menu section): Import STL, binary+ASCII, replaces the document, Ctrl+Z undoes
   it, lands as a group, the >50k prompt and why, that imports may not be watertight and export
   stays blocked until fixed, and that geometry keeps its original coordinates (use Arrange for
   Print / Drop to Plate). `CLAUDE.md`: one line each for `io/stl_import.rs` and
   `geometry/reconstruct.rs`, plus a cross-cutting note that `detect_faces` returns a hole-filling
   face per hole and callers must filter it.

---

## Part 9 — Verification

- `cd src-tauri && cargo test` — all new tests plus the existing suite.
- `npx tsc --noEmit`.
- `npm run tauri dev`, then end-to-end:
  1. Model a cube, Export STL, Import it back — expect 6 faces, black outlines only on real edges
     (not triangle diagonals), Push/Pull a whole side.
  2. Same with a 16-segment cylinder — expect 18 faces, still manifold, Export STL succeeds.
  3. A real downloaded STL — expect it framed in view, listed as a group in the outliner, and
     either a clean import or the "Show Me" watertight prompt.
  4. A >50k-triangle mesh — expect the confirm dialog, and that cancelling leaves the current
     document untouched.

---

## Part 10 — Risks and limitations

1. **Curved/organic meshes don't merge** — by design (0.1°). A 50k-triangle scan becomes a
   50k-face document: renders, moves, exports, but isn't meaningfully face-editable.
2. **`History::record` clones the entire `Document` per edit** (`commands.rs:33`), 100-step cap,
   whose doc comment assumes "a spaceship part, not a large assembly". A big import breaks that
   assumption. Not fixed here; a follow-up could scale the cap by face count.
3. **The frontend rebuilds every buffer per snapshot** and pushes one `FaceId` per triangle into
   `triangleFaceIds` — at 50k+ faces every click re-allocates and re-uploads everything. This is
   the main reason the warning exists.
4. **Ear clipping is O(n²) per face and runs on every `snapshot()`** — i.e. after every command. A
   merged face with a 2,000-point boundary is ~4M tests per snapshot; 2d is the mitigation.
5. **Imported wild STLs are often non-manifold** and stay blocked from re-export. Intended; the
   post-import prompt makes it discoverable rather than mysterious. No repair/hole-filling in this
   feature.
6. **Zero-area facets are dropped at weld time**, which can open an edge in an otherwise-closed
   mesh. Accepted — the input was already degenerate there, and keeping them corrupts `face_detect`.
7. **No unit handling.** STL is unitless; the app assumes 1 unit = 1 mm, so a model authored in
   inches imports at the wrong scale with the Scale tool as the only remedy. A "scale on import"
   field would fix it; out of scope.
