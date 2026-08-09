use std::collections::{HashMap, HashSet};

use glam::{DQuat, DVec2, DVec3};
use serde::Serialize;
use slotmap::{new_key_type, SlotMap};

use crate::geometry::face_detect;
use crate::geometry::inset;
use crate::geometry::mesh::{Face, FaceId, Mesh, VertexId};
use crate::geometry::plane::Plane;
use crate::geometry::triangulate::triangulate_face;
use crate::geometry::{primitives, pushpull};
use crate::io::project_file::{ProjectFace, ProjectFile, ProjectGroup};

use super::selection::Selection;

new_key_type! {
    pub struct GroupId;
}

/// Gap (mm) left between each part's bounding box when `arrange_for_print`
/// lays them out on a grid - matches the ground grid's 10mm cell convention
/// (see CLAUDE.md's "1 world unit = 1mm" note).
const PRINT_ARRANGE_SPACING: f64 = 10.0;

/// A simple named collection of faces, moved/renamed together. No shared
/// component-definition/instance system - each group just owns its faces
/// directly, which is enough for one-off spaceship parts (hull, wing, ...).
#[derive(Debug, Clone)]
pub struct Group {
    pub name: String,
    pub face_ids: Vec<FaceId>,
}

/// A construction guide left behind by the Measure tool: the measured
/// segment itself. Its two endpoints, its midpoint, and any point along it
/// are snap targets, so a primitive can be built exactly on a distance you
/// just measured. Guides are reference-only annotations - they never enter
/// `Mesh`, so they're automatically invisible to triangulation, STL export,
/// `check_model`, bounding boxes and `connected_components`.
///
/// Deliberately world-fixed: no transform command (translate/rotate/scale,
/// drop-to-plate, arrange-for-print) moves them. There's no non-arbitrary
/// rule for "which guides belong to this face set", and the primary workflow
/// - measure on part A, move A aside, build part B on the marks - depends on
/// guides staying put.
#[derive(Debug, Clone, Copy, Serialize, serde::Deserialize)]
pub struct Guide {
    pub a: DVec3,
    pub b: DVec3,
}

/// The whole editable document: one flat geometry pool plus the groups and
/// selection layered on top of it. Faces created by unrelated draw/push-pull
/// operations never share vertices, so translating/rotating/scaling a set of
/// faces can never accidentally drag unrelated geometry - it only affects
/// vertices actually referenced by the given faces.
#[derive(Clone)]
pub struct Document {
    pub mesh: Mesh,
    pub groups: SlotMap<GroupId, Group>,
    pub selection: Selection,
    pub guides: Vec<Guide>,
    face_to_group: HashMap<FaceId, GroupId>,
    /// Faces created by push_pull (caps and side walls of an already-built
    /// solid). Excluded from resplit_plane's coplanar search: a solid's cap
    /// can easily end up coplanar with the ground plane (e.g. a downward
    /// push/pull leaves its top cap sitting at z=0), and without this it
    /// would get silently swept into and corrupted by the next unrelated
    /// sketch drawn on that plane.
    solid_face_ids: HashSet<FaceId>,
}

impl Document {
    pub fn new() -> Self {
        Document {
            mesh: Mesh::new(),
            groups: SlotMap::with_key(),
            selection: Selection::default(),
            guides: Vec::new(),
            face_to_group: HashMap::new(),
            solid_face_ids: HashSet::new(),
        }
    }

    /// `target_face_id`: when the sketch was drawn on top of an existing
    /// solid face (see `resolve_sketch_target` on the frontend), the new
    /// loop is merged with just that face's own boundary/holes instead of
    /// every coplanar face in the document - see `resplit` below.
    pub fn draw_rectangle(
        &mut self,
        plane: &Plane,
        corner_a: DVec2,
        corner_b: DVec2,
        target_face_id: Option<FaceId>,
    ) -> Vec<FaceId> {
        let temp_face_id = primitives::add_rectangle(&mut self.mesh, plane, corner_a, corner_b);
        let new_loop = self.mesh.faces[temp_face_id].outer.clone();
        self.mesh.remove_face(temp_face_id);
        self.resplit(plane, new_loop, target_face_id)
    }

    pub fn draw_circle(
        &mut self,
        plane: &Plane,
        center: DVec2,
        radius: f64,
        segments: usize,
        target_face_id: Option<FaceId>,
    ) -> Vec<FaceId> {
        let temp_face_id = primitives::add_circle(&mut self.mesh, plane, center, radius, segments);
        let new_loop = self.mesh.faces[temp_face_id].outer.clone();
        self.mesh.remove_face(temp_face_id);
        self.resplit(plane, new_loop, target_face_id)
    }

    /// Draws a chord-closed circular segment (the Arc tool) - see
    /// `primitives::add_arc` for the shape this produces and why it's
    /// chord-closed rather than routed through a center vertex.
    pub fn draw_arc(
        &mut self,
        plane: &Plane,
        center: DVec2,
        radius: f64,
        start_angle_deg: f64,
        sweep_deg: f64,
        segments: usize,
        target_face_id: Option<FaceId>,
    ) -> Vec<FaceId> {
        let temp_face_id = primitives::add_arc(&mut self.mesh, plane, center, radius, start_angle_deg, sweep_deg, segments);
        let new_loop = self.mesh.faces[temp_face_id].outer.clone();
        self.mesh.remove_face(temp_face_id);
        self.resplit(plane, new_loop, target_face_id)
    }

    /// Draws a regular polygon (5-8 sides, the N-Gon tool) - see
    /// `primitives::add_ngon`. `start_angle_deg` sets the rotation (the
    /// tool derives it from the click that sets the radius, so one vertex
    /// lands under the cursor).
    pub fn draw_ngon(
        &mut self,
        plane: &Plane,
        center: DVec2,
        radius: f64,
        sides: usize,
        start_angle_deg: f64,
        target_face_id: Option<FaceId>,
    ) -> Vec<FaceId> {
        let temp_face_id = primitives::add_ngon(&mut self.mesh, plane, center, radius, sides, start_angle_deg);
        let new_loop = self.mesh.faces[temp_face_id].outer.clone();
        self.mesh.remove_face(temp_face_id);
        self.resplit(plane, new_loop, target_face_id)
    }

    /// Draws a closed polygon from explicit click points (the Polygon/Line
    /// tool). Point winding doesn't matter: resplit_plane's face_detect pass
    /// re-derives correct orientation from the undirected edge graph either
    /// way. Returns an empty Vec if fewer than 3 points were given.
    pub fn draw_polygon(&mut self, plane: &Plane, points: Vec<DVec2>, target_face_id: Option<FaceId>) -> Vec<FaceId> {
        let Some(temp_face_id) = primitives::add_polyline_loop(&mut self.mesh, plane, &points) else {
            return Vec::new();
        };
        let new_loop = self.mesh.faces[temp_face_id].outer.clone();
        self.mesh.remove_face(temp_face_id);
        self.resplit(plane, new_loop, target_face_id)
    }

    /// Routes a freshly drawn loop to either a single target face's own
    /// resplit (sketching on a solid's side wall to cut a porthole/hatch
    /// without disturbing unrelated coplanar geometry) or the general
    /// coplanar-search resplit (`resplit_plane`) when there's no target, or
    /// the target no longer exists (stale id - e.g. an unrelated edit
    /// removed it since the frontend's last snapshot). Falling back instead
    /// of no-op-ing means a stale target never silently drops the user's
    /// drawn shape.
    fn resplit(&mut self, plane: &Plane, new_loop: Vec<VertexId>, target_face_id: Option<FaceId>) -> Vec<FaceId> {
        if let Some(face_id) = target_face_id {
            if self.mesh.faces.contains_key(face_id) {
                return self.resplit_face_with_loops(face_id, vec![new_loop]);
            }
        }
        self.resplit_plane(plane, new_loop)
    }

    /// Combines `new_loop` with every existing face coplanar with `plane`,
    /// re-runs face detection over the merged edge graph, and replaces the
    /// old coplanar faces with the freshly detected ones. This is what makes
    /// drawing a loop inside an existing coplanar face auto-split it into an
    /// inner face and an outer face-with-a-hole ("sticky geometry") - e.g.
    /// drawing a smaller circle inside a larger one, then erasing the inner
    /// face, leaves a ring rather than two independent overlapping disks.
    fn resplit_plane(&mut self, plane: &Plane, new_loop: Vec<VertexId>) -> Vec<FaceId> {
        let coplanar_face_ids: Vec<FaceId> = self
            .mesh
            .faces
            .iter()
            .filter(|(id, face)| !self.solid_face_ids.contains(id) && is_coplanar(&self.mesh, face, plane))
            .map(|(id, _)| id)
            .collect();

        let outers: Vec<Vec<VertexId>> =
            coplanar_face_ids.iter().map(|&fid| self.mesh.faces[fid].outer.clone()).collect();
        let holes: Vec<Vec<VertexId>> =
            coplanar_face_ids.iter().flat_map(|&fid| self.mesh.faces[fid].holes.clone()).collect();

        let mut loops: Vec<Vec<VertexId>> = outers.iter().cloned().chain(holes.iter().cloned()).collect();
        loops.push(new_loop);

        // A hole here may legitimately be filled by another face in this same
        // coplanar set (a disc sitting in a ring's middle) - that face is
        // being erased too, so its disc has to come back from the re-detect.
        // A hole that matches no erased face's outer, though, is genuinely
        // empty (the user erased what filled it, or a solid's wall closes it
        // in 3D) and must stay empty.
        let protected_holes: Vec<Vec<VertexId>> =
            holes.into_iter().filter(|h| !matches_any_loop(h, &outers)).collect();

        // Detect the replacement geometry before erasing anything - see
        // `resplit_face_with_loops` for why: erasing first and finding
        // `resplit_loops` came back empty would permanently delete every
        // coplanar face in this set with nothing to replace them.
        let new_faces = self.resplit_loops(plane, loops, &protected_holes);
        if new_faces.is_empty() {
            return Vec::new();
        }

        for fid in &coplanar_face_ids {
            self.erase_face(*fid);
        }

        new_faces
    }

    /// Inline-offsets `face_id`'s outer boundary inward by `offset` (in
    /// model units), splitting it into an inner face (the shrunk copy) and
    /// an outer frame face with the inset loop as a hole - the "Offset"
    /// workflow from SketchUp, generalized to any face shape instead of
    /// requiring the user to hand-draw a second concentric loop and erase
    /// the middle. Any existing holes in `face_id` are carried through
    /// unchanged. Returns an empty Vec (no-op) if `face_id` doesn't exist or
    /// the offset is too large for the face's shape (see
    /// `geometry::inset::offset_polygon`).
    pub fn inset_face(&mut self, face_id: FaceId, offset: f64) -> Vec<FaceId> {
        let Some(face) = self.mesh.faces.get(face_id) else {
            return Vec::new();
        };
        let origin = self.mesh.position(face.outer[0]);
        let plane = Plane::from_normal(origin, face.normal);

        let outer_2d: Vec<DVec2> = face.outer.iter().map(|&vid| plane.to_2d(self.mesh.position(vid))).collect();
        let Some(inset_2d) = inset::offset_polygon(&outer_2d, offset) else {
            return Vec::new();
        };
        let inset_loop: Vec<VertexId> = inset_2d.into_iter().map(|p| self.mesh.add_vertex(plane.to_3d(p))).collect();

        self.resplit_face_with_loops(face_id, vec![inset_loop])
    }

    /// Erases `face_id` and rebuilds it by resplitting its own boundary +
    /// holes together with `extra_loops` (new loop(s) added on top of it,
    /// e.g. an inset ring or a sketch drawn directly on this face) - a
    /// *local* resplit restricted to this one face's own loops, unlike
    /// `resplit_plane`'s document-wide coplanar search. New faces inherit
    /// `face_id`'s group and solid-boundary membership, so insetting or
    /// sketching on a face never silently ejects the result from its group
    /// or strips its printable status. Returns an empty Vec if `face_id`
    /// doesn't exist.
    fn resplit_face_with_loops(&mut self, face_id: FaceId, extra_loops: Vec<Vec<VertexId>>) -> Vec<FaceId> {
        let Some(face) = self.mesh.faces.get(face_id) else {
            return Vec::new();
        };
        let origin = self.mesh.position(face.outer[0]);
        let plane = Plane::from_normal(origin, face.normal);

        // A loop sketched "on" this face must actually lie within its
        // boundary. The frontend resolves which face a click landed on with
        // a single raycast, which is ambiguous exactly on a shared
        // vertex/edge between two faces - the same spot a snapped corner
        // (an existing vertex, edge midpoint, or measure-tool guide) is
        // most likely to land on. When that raycast resolves to the "wrong"
        // neighboring face, the rest of the shape - sized from screen
        // positions the user intended for a completely different plane -
        // ends up mostly or entirely outside `face_id`'s own outline.
        // `face_detect`'s planar graph can only represent loops that touch
        // an existing boundary at shared vertices, not ones whose edges
        // cross it, and silently corrupts the resulting geometry instead of
        // erroring cleanly when that happens. Reject rather than corrupt -
        // matching `inset_face`'s no-op-on-invalid-input pattern - and let
        // the small tolerance still allow the common, legitimate case of a
        // corner sketched exactly on this face's own edge or corner.
        const FACE_FIT_TOLERANCE: f64 = 1e-2;
        let outer_2d: Vec<DVec2> = face.outer.iter().map(|&v| plane.to_2d(self.mesh.position(v))).collect();
        for loop_vertices in &extra_loops {
            for &vid in loop_vertices {
                let p = plane.to_2d(self.mesh.position(vid));
                if !face_detect::point_in_or_near_polygon(p, &outer_2d, FACE_FIT_TOLERANCE) {
                    return Vec::new();
                }
            }
        }

        // Weld `extra_loops` onto `face_id`'s whole connected solid, not
        // just this one face's own boundary. A corner sketched on this face
        // and snapped onto a vertex/edge/guide on a *different, adjacent*
        // face of the same solid (e.g. building a stepped greeble, where
        // each new level's corner is snapped onto the previous level's rim)
        // is a brand new `VertexId` at a position a hair off that other
        // face's vertex - see `Mesh::WELD_TOLERANCE`. `face_id`'s own
        // boundary/holes are already covered below (their vertices are
        // included in the connected-solid set too), so this is what makes
        // that cross-face case connect instead of leaving a sliver of open
        // edge right at the seam a guide was used to align. Scoped to the
        // connected solid (not the whole mesh) so this can't reach into an
        // unrelated, independently-drawn object that merely happens to
        // share a coordinate (e.g. two separate parts both starting at the
        // origin) - `resplit_plane`'s own coplanar-and-non-solid scoping
        // handles that case correctly already and must stay that way.
        let solid_vertices = self.connected_component_vertices(face_id);
        let extra_loops: Vec<Vec<VertexId>> =
            extra_loops.into_iter().map(|loop_verts| self.weld_loop_onto(loop_verts, &solid_vertices)).collect();

        // A hole in this face is there because something else already closes
        // it - most often a push/pulled stud or recess whose wall meets this
        // face at exactly that rim. Re-detecting over this face's loops would
        // otherwise hand back a fresh disc filling that rim, duplicating every
        // edge the wall already pairs with and breaking watertightness. Since
        // only this one face is being rebuilt, anything that did fill the hole
        // is a different face and is left untouched, so dropping the refill is
        // always right here (unlike in `resplit_plane`, which erases and
        // rebuilds a whole set of coplanar faces at once).
        let protected_holes = face.holes.clone();
        let mut loops = vec![face.outer.clone()];
        loops.extend(face.holes.iter().cloned());
        loops.extend(extra_loops.iter().cloned());

        // `resplit_loops` only reads vertex positions and creates new faces
        // via `add_face` - it never touches `face_id` itself - so it's safe
        // to run before erasing the source face. That ordering matters: if
        // face_detect's half-edge tracing hits a degenerate/ambiguous
        // configuration it has no representation for and comes back with
        // nothing, erasing first would delete `face_id` with nothing to
        // replace it - a real "face disappeared" data-loss bug, not just a
        // rejected draw.
        let new_faces = self.resplit_loops(&plane, loops, &protected_holes);
        if new_faces.is_empty() {
            return Vec::new();
        }

        // `face_detect::split_edges_at_interior_points` may have just split
        // one of `face_id`'s own boundary/hole edges at a T-junction where
        // `extra_loops` touches it partway along (e.g. a rectangle sketched
        // flush into the corner of this face - the everyday "stud in the
        // corner of a box top" move). If `face_id` is on a solid's boundary,
        // any OTHER face of that same solid sharing that exact edge (a wall
        // meeting this face at its rim) still has the old, unsplit edge -
        // propagate the same split to it, or its untouched edge loses its
        // manifold pairing with this face's newly-split one and the solid
        // gets an open edge. Must run before `erase_face` below, while
        // `face_id`'s own (pre-split) boundary is still there to read.
        self.propagate_boundary_split_to_solid_siblings(face_id, &extra_loops, &new_faces);

        let was_grouped = self.face_to_group.get(&face_id).copied();
        let was_solid = self.solid_face_ids.contains(&face_id);
        self.erase_face(face_id);

        if let Some(group_id) = was_grouped {
            if let Some(group) = self.groups.get_mut(group_id) {
                group.face_ids.extend(&new_faces);
                for &f in &new_faces {
                    self.face_to_group.insert(f, group_id);
                }
            }
        }
        if was_solid {
            self.solid_face_ids.extend(&new_faces);
        }
        new_faces
    }

    /// Every vertex referenced by any face in the same connected component
    /// (see `Mesh::connected_components`) as `face_id` - i.e. the whole
    /// solid (or flat coplanar sketch group) `face_id` is currently part of,
    /// not just `face_id`'s own boundary. Used to scope which existing
    /// vertices a freshly-drawn loop is allowed to weld onto (see
    /// `weld_loop_onto`); deliberately not "every vertex in the mesh", so an
    /// unrelated, independently-drawn object can never be pulled in just
    /// because it happens to share a coordinate.
    fn connected_component_vertices(&self, face_id: FaceId) -> Vec<VertexId> {
        let all_face_ids: Vec<FaceId> = self.mesh.faces.keys().collect();
        let component = self
            .mesh
            .connected_components(&all_face_ids)
            .into_iter()
            .find(|c| c.contains(&face_id))
            .unwrap_or_default();
        component
            .into_iter()
            .flat_map(|fid| {
                let face = &self.mesh.faces[fid];
                face.outer.iter().copied().chain(face.holes.iter().flatten().copied())
            })
            .collect()
    }

    /// Replaces any vertex in `loop_verts` that lies within
    /// `Mesh::WELD_TOLERANCE` of a vertex in `candidates` with that existing
    /// vertex id; every other vertex passes through unchanged. `candidates`
    /// is deliberately scoped by the caller (see `connected_component_vertices`)
    /// rather than searching the whole mesh, so this only ever connects a new
    /// loop to geometry the current operation is actually meant to touch.
    fn weld_loop_onto(&self, loop_verts: Vec<VertexId>, candidates: &[VertexId]) -> Vec<VertexId> {
        loop_verts
            .into_iter()
            .map(|vid| {
                let p = self.mesh.position(vid);
                candidates
                    .iter()
                    .copied()
                    .find(|&c| (self.mesh.position(c) - p).length_squared() < Mesh::WELD_TOLERANCE * Mesh::WELD_TOLERANCE)
                    .unwrap_or(vid)
            })
            .collect()
    }

    /// Finds every point in `extra_loops` that lands on the interior of one
    /// of `face_id`'s own boundary/hole edges (a T-junction -
    /// `face_detect::split_edges_at_interior_points` is about to split that
    /// edge on `face_id`'s side), and inserts the same point(s), in the same
    /// order, into the matching edge of every OTHER pre-existing face in the
    /// same connected solid (`just_created` - this same call's own
    /// replacement faces, already built from the already-split edge graph -
    /// is excluded, or its already-correct boundary would get a redundant,
    /// corrupting duplicate vertex inserted into it). Faces don't share
    /// topology in this mesh (see `Mesh`'s struct doc comment) - each keeps
    /// its own independent loop of vertex ids - so splitting `face_id`'s
    /// side of a shared rim without also updating the neighbor's side leaves
    /// that neighbor's untouched edge with no matching reverse-direction
    /// pairing anywhere any more: exactly the open-edge/"not watertight"
    /// corruption this exists to prevent.
    ///
    /// Deliberately a no-op for a `face_id` that isn't on a solid's
    /// boundary. Flat sketches left adjacent by an earlier `resplit_plane`
    /// *do* share edges with each other, so the neighbor really is left
    /// holding a stale, unsplit copy - but nothing pairs edges across two
    /// flat sketches. Each extrudes into its own independently closed solid
    /// built from its own loop, so the mismatch never becomes a manifold
    /// error (pinned by
    /// `a_t_junction_on_an_edge_shared_between_two_flat_sketches_survives_extrusion`).
    /// Propagating there anyway would reach across two *unrelated* sketches
    /// that merely touch, which is the over-broad behavior
    /// `connected_component_vertices` exists to avoid.
    fn propagate_boundary_split_to_solid_siblings(
        &mut self,
        face_id: FaceId,
        extra_loops: &[Vec<VertexId>],
        just_created: &[FaceId],
    ) {
        if !self.solid_face_ids.contains(&face_id) {
            return;
        }
        let Some(face) = self.mesh.faces.get(face_id) else {
            return;
        };
        let plane = Plane::from_normal(self.mesh.position(face.outer[0]), face.normal);
        let rings: Vec<Vec<VertexId>> = std::iter::once(face.outer.clone()).chain(face.holes.iter().cloned()).collect();

        // For each of face_id's own directed boundary edges (a -> b), every
        // new point from `extra_loops` landing on its interior, in order.
        let mut splits: Vec<(VertexId, VertexId, Vec<VertexId>)> = Vec::new();
        for ring in &rings {
            let n = ring.len();
            for i in 0..n {
                let a = ring[i];
                let b = ring[(i + 1) % n];
                let pa = plane.to_2d(self.mesh.position(a));
                let pb = plane.to_2d(self.mesh.position(b));
                let ab = pb - pa;
                let len_sq = ab.length_squared();
                if len_sq < 1e-18 {
                    continue;
                }
                let mut on_edge: Vec<(VertexId, f64)> = Vec::new();
                for loop_verts in extra_loops {
                    for &vid in loop_verts {
                        if vid == a || vid == b {
                            continue;
                        }
                        let p = plane.to_2d(self.mesh.position(vid));
                        let t = (p - pa).dot(ab) / len_sq;
                        if !(1e-9..=1.0 - 1e-9).contains(&t) {
                            continue;
                        }
                        let closest = pa + ab * t;
                        if (p - closest).length_squared() < Mesh::WELD_TOLERANCE * Mesh::WELD_TOLERANCE {
                            on_edge.push((vid, t));
                        }
                    }
                }
                if on_edge.is_empty() {
                    continue;
                }
                on_edge.sort_by(|x, y| x.1.partial_cmp(&y.1).unwrap());
                on_edge.dedup_by_key(|&mut (v, _)| v);
                splits.push((a, b, on_edge.into_iter().map(|(v, _)| v).collect()));
            }
        }
        if splits.is_empty() {
            return;
        }

        let component_face_ids: Vec<FaceId> = self
            .mesh
            .connected_components(&self.mesh.faces.keys().collect::<Vec<_>>())
            .into_iter()
            .find(|c| c.contains(&face_id))
            .unwrap_or_default();

        for sibling_id in component_face_ids {
            // `face_id` itself is about to be erased by the caller, and
            // `just_created` (this same call's own replacement faces) were
            // just built directly from the already-split edge graph - both
            // already have the split point exactly where they need it, so
            // touching either here would insert a redundant, corrupting
            // duplicate vertex into an already-correct boundary.
            if sibling_id == face_id || just_created.contains(&sibling_id) {
                continue;
            }
            let Some(sibling) = self.mesh.faces.get_mut(sibling_id) else {
                continue;
            };
            for ring in std::iter::once(&mut sibling.outer).chain(sibling.holes.iter_mut()) {
                let n = ring.len();
                if n < 2 {
                    continue;
                }
                let mut new_ring = Vec::with_capacity(n);
                let mut changed = false;
                for i in 0..n {
                    let x = ring[i];
                    let y = ring[(i + 1) % n];
                    new_ring.push(x);
                    // A sibling sharing this rim traverses it in the
                    // opposite direction (y -> x) under this app's
                    // CCW-outward winding convention, so its matching split
                    // is keyed on (b, a) = (y, x).
                    if let Some((_, _, new_verts)) = splits.iter().find(|(a, b, _)| *a == y && *b == x) {
                        new_ring.extend(new_verts.iter().rev().copied());
                        changed = true;
                    }
                }
                if changed {
                    *ring = new_ring;
                }
            }
        }
    }

    /// Builds the combined 2D edge graph for `loops` (each a closed ring of
    /// mesh vertices, already known to be coplanar with `plane`), re-detects
    /// faces/holes over it, and creates the resulting faces. Shared by
    /// `resplit_plane` (sticky-geometry auto-split while drawing) and
    /// `inset_face` (splitting one face into an inner face + offset frame).
    ///
    /// `protected_holes` are loops that must stay empty: `face_detect`
    /// reports every enclosed region it finds, including the inside of a
    /// hole the source face already had, so without this it would
    /// re-materialize a face there. See `resplit_face_with_loops` for why
    /// that's wrong.
    fn resplit_loops(
        &mut self,
        plane: &Plane,
        loops: Vec<Vec<VertexId>>,
        protected_holes: &[Vec<VertexId>],
    ) -> Vec<FaceId> {
        // Two loops can reference *different* VertexIds at what's meant to
        // be the same position - most commonly a freshly-drawn shape whose
        // corner was snapped onto an existing vertex/edge/guide: the
        // frontend only ever sends raw coordinates (never vertex ids), and
        // those coordinates round-tripped through the f32 DocumentSnapshot
        // on the way there and back, picking up a tiny rounding error
        // relative to the vertex's true f64 position. Without a weld here,
        // `face_detect`'s neighbor-angle sort sees two almost-but-not-quite
        // coincident points and produces an essentially arbitrary ordering
        // between them, splicing unrelated edges into a self-intersecting
        // face. See `Mesh::WELD_TOLERANCE` for why 1e-3mm.
        let weld_eps_sq = Mesh::WELD_TOLERANCE * Mesh::WELD_TOLERANCE;

        let mut index_of: HashMap<VertexId, usize> = HashMap::new();
        let mut points: Vec<DVec2> = Vec::new();
        let mut vertex_by_index: Vec<VertexId> = Vec::new();
        let mut edges: Vec<(usize, usize)> = Vec::new();

        for loop_vertices in &loops {
            let local_indices: Vec<usize> = loop_vertices
                .iter()
                .map(|&vid| {
                    if let Some(&i) = index_of.get(&vid) {
                        return i;
                    }
                    let p = plane.to_2d(self.mesh.position(vid));
                    // Loops are processed in the order the caller built
                    // `loops` in - the source face's own pre-existing
                    // boundary/holes always come before any newly-drawn loop
                    // (see `resplit_plane`/`resplit_face_with_loops`), so a
                    // match here always keeps the pre-existing vertex - the
                    // one an adjacent, untouched face may still reference at
                    // that same corner, which is what keeps the mesh
                    // watertight there.
                    let i = points
                        .iter()
                        .position(|&q| (q - p).length_squared() < weld_eps_sq)
                        .unwrap_or_else(|| {
                            points.push(p);
                            vertex_by_index.push(vid);
                            points.len() - 1
                        });
                    index_of.insert(vid, i);
                    i
                })
                .collect();
            let n = local_indices.len();
            for i in 0..n {
                edges.push((local_indices[i], local_indices[(i + 1) % n]));
            }
        }

        let detected = face_detect::detect_faces(&points, &edges);

        detected
            .into_iter()
            .filter_map(|df| {
                let outer: Vec<VertexId> = df.outer.iter().map(|&i| vertex_by_index[i]).collect();
                if loop_covers_any(&outer, protected_holes) {
                    return None;
                }
                let holes: Vec<Vec<VertexId>> = df
                    .holes
                    .iter()
                    .map(|h| h.iter().map(|&i| vertex_by_index[i]).collect())
                    .collect();
                Some(self.mesh.add_face(outer, holes))
            })
            .collect()
    }

    pub fn push_pull(&mut self, face_id: FaceId, distance: f64) -> Vec<FaceId> {
        if !self.mesh.faces.contains_key(face_id) {
            // Stale id (e.g. the selection wasn't refreshed after an
            // unrelated resplit erased and recreated this face) - a no-op
            // is safer than panicking on an invalid SlotMap key.
            return Vec::new();
        }
        if distance.abs() < 1e-9 {
            // pushpull::push_pull would no-op anyway; returning before the
            // bookkeeping below keeps the face's group/solid membership
            // intact instead of stripping it for nothing.
            return Vec::new();
        }
        let was_grouped = self.face_to_group.get(&face_id).copied();
        // A face already on a solid's boundary (a cap/wall from an earlier
        // push_pull, or an inset split of one) extrudes in "attached" mode:
        // no cap at the source position, so the result merges with the
        // existing solid as one watertight shell instead of leaving a
        // coincident interior cap that breaks manifoldness.
        let was_solid = self.solid_face_ids.contains(&face_id);
        self.face_to_group.remove(&face_id);
        self.solid_face_ids.remove(&face_id);
        let new_faces = if was_solid {
            pushpull::push_pull_attached(&mut self.mesh, face_id, distance)
        } else {
            pushpull::push_pull(&mut self.mesh, face_id, distance)
        };
        self.solid_face_ids.extend(&new_faces);
        // Keep the extruded solid's faces in whatever group the source
        // sketch face was in, so pushing/pulling inside a group doesn't
        // silently eject the result from it.
        if let Some(group_id) = was_grouped {
            if let Some(group) = self.groups.get_mut(group_id) {
                group.face_ids.retain(|&f| f != face_id);
                group.face_ids.extend(&new_faces);
                for &f in &new_faces {
                    self.face_to_group.insert(f, group_id);
                }
            }
        }
        self.selection.faces.remove(&face_id);
        new_faces
    }

    /// Push/pulls every face in `face_ids` by the same signed `distance`,
    /// each along its own normal - so selecting several faces (e.g. two
    /// opposite walls) and pushing/pulling them together grows/shrinks each
    /// of them outward by the same amount, not just whichever one was
    /// clicked.
    pub fn push_pull_faces(&mut self, face_ids: &[FaceId], distance: f64) -> Vec<FaceId> {
        face_ids.iter().flat_map(|&face_id| self.push_pull(face_id, distance)).collect()
    }

    pub fn erase_face(&mut self, face_id: FaceId) {
        self.remove_face_from_its_group(face_id);
        self.selection.faces.remove(&face_id);
        self.solid_face_ids.remove(&face_id);
        self.mesh.remove_face(face_id);
    }

    pub fn translate_faces(&mut self, face_ids: &[FaceId], delta: DVec3) {
        let moved = self.unique_vertices(face_ids);
        for &vid in &moved {
            let p = self.mesh.position(vid);
            self.mesh.vertices[vid].position = p + delta;
        }
        self.recompute_normals_touching(&moved);
    }

    pub fn rotate_faces(&mut self, face_ids: &[FaceId], pivot: DVec3, axis: DVec3, angle_radians: f64) {
        let rotation = DQuat::from_axis_angle(axis.normalize(), angle_radians);
        let moved = self.unique_vertices(face_ids);
        for &vid in &moved {
            let p = self.mesh.position(vid);
            self.mesh.vertices[vid].position = pivot + rotation * (p - pivot);
        }
        self.recompute_normals_touching(&moved);
    }

    /// Scales relative to `pivot` along world X/Y/Z independently. Matches
    /// SketchUp's basic scale tool closely enough for v1; it does not
    /// support scaling along an arbitrary (rotated) local axis.
    pub fn scale_faces(&mut self, face_ids: &[FaceId], pivot: DVec3, scale: DVec3) {
        let moved = self.unique_vertices(face_ids);
        for &vid in &moved {
            let p = self.mesh.position(vid);
            self.mesh.vertices[vid].position = pivot + (p - pivot) * scale;
        }
        self.recompute_normals_touching(&moved);
    }

    pub fn duplicate_faces(&mut self, face_ids: &[FaceId], delta: DVec3) -> Vec<FaceId> {
        let new_faces = self.clone_faces_mapped(face_ids, |p| p + delta, false);
        self.selection.faces = new_faces.iter().copied().collect();
        new_faces
    }

    /// Copies `face_ids` into a `columns` x `rows` grid, stepping `pitch_x`
    /// along world X per column and `pitch_y` along world Y per row. The
    /// pitch is center-to-center, so a pitch smaller than the part
    /// deliberately overlaps rather than being clamped - the user asked for
    /// that spacing. The counts include the source, which stays put in cell
    /// (0, 0), so only the other `columns * rows - 1` cells get copies;
    /// that's how CAD array counts read (3 x 2 means six objects total).
    /// Passes `reverse_winding: false` to `clone_faces_mapped`: a pure
    /// translation preserves handedness, so reversing would flip every
    /// copy's normals inward.
    ///
    /// Unlike `duplicate_faces`/`mirror_faces`, which leave just the *copy*
    /// selected, this leaves the *whole grid* (source included) selected -
    /// the grid, not any one copy, is the thing the user just made, and it's
    /// ready for a follow-up Move or Group Selected.
    pub fn array_faces(
        &mut self,
        face_ids: &[FaceId],
        columns: usize,
        rows: usize,
        pitch_x: f64,
        pitch_y: f64,
    ) -> Vec<FaceId> {
        let mut new_faces = Vec::new();
        for row in 0..rows {
            for col in 0..columns {
                if row == 0 && col == 0 {
                    continue; // the source already occupies this cell
                }
                let delta = DVec3::new(col as f64 * pitch_x, row as f64 * pitch_y, 0.0);
                new_faces.extend(self.clone_faces_mapped(face_ids, |p| p + delta, false));
            }
        }
        if !new_faces.is_empty() {
            // Stale source ids are silently dropped rather than selected, the
            // same way `clone_faces_mapped` skips them.
            self.selection.faces = face_ids
                .iter()
                .copied()
                .filter(|&fid| self.mesh.faces.contains_key(fid))
                .chain(new_faces.iter().copied())
                .collect();
        }
        new_faces
    }

    /// Mirrors a *copy* of `face_ids` across the world plane perpendicular
    /// to `axis` through `pivot` (e.g. `MirrorAxis::X` mirrors across the
    /// plane x = pivot.x) - the source geometry is left untouched, matching
    /// SketchUp's Mirror. Building one half of a symmetric hull/wing and
    /// mirroring a copy is the common case this exists for.
    pub fn mirror_faces(&mut self, face_ids: &[FaceId], axis: MirrorAxis, pivot: DVec3) -> Vec<FaceId> {
        let reflect = move |p: DVec3| -> DVec3 {
            match axis {
                MirrorAxis::X => DVec3::new(2.0 * pivot.x - p.x, p.y, p.z),
                MirrorAxis::Y => DVec3::new(p.x, 2.0 * pivot.y - p.y, p.z),
                MirrorAxis::Z => DVec3::new(p.x, p.y, 2.0 * pivot.z - p.z),
            }
        };
        let new_faces = self.clone_faces_mapped(face_ids, reflect, true);
        self.selection.faces = new_faces.iter().copied().collect();
        new_faces
    }

    /// Clones `face_ids` into new faces through a single shared vertex map,
    /// so faces that share vertices in the source (e.g. every face of a
    /// solid) keep sharing them in the copy - required for the copy to be
    /// manifold on its own, not just a pile of individually-closed faces.
    /// `map_pos` repositions each newly cloned vertex; `reverse_winding`
    /// reverses every loop (outer and holes), which a mirror needs to
    /// restore the CCW-outer/CW-hole invariant after a reflection flips
    /// handedness (a plain translation preserves handedness and must NOT
    /// reverse). Copies inherit their source face's solid-boundary
    /// membership but join no group - detaching from a group on copy
    /// matches SketchUp's Copy/Mirror.
    fn clone_faces_mapped(
        &mut self,
        face_ids: &[FaceId],
        map_pos: impl Fn(DVec3) -> DVec3,
        reverse_winding: bool,
    ) -> Vec<FaceId> {
        let mut vertex_map: HashMap<VertexId, VertexId> = HashMap::new();
        let mut new_face_ids = Vec::new();

        for &face_id in face_ids {
            let Some(face) = self.mesh.faces.get(face_id).cloned() else {
                continue;
            };
            let was_solid = self.solid_face_ids.contains(&face_id);

            let outer = clone_loop_through_map(&mut self.mesh, &mut vertex_map, &face.outer, &map_pos, reverse_winding);
            let holes: Vec<Vec<VertexId>> = face
                .holes
                .iter()
                .map(|h| clone_loop_through_map(&mut self.mesh, &mut vertex_map, h, &map_pos, reverse_winding))
                .collect();

            let new_id = self.mesh.add_face(outer, holes);
            if was_solid {
                self.solid_face_ids.insert(new_id);
            }
            new_face_ids.push(new_id);
        }

        new_face_ids
    }

    pub fn group_faces(&mut self, face_ids: &[FaceId], name: String) -> GroupId {
        for &fid in face_ids {
            self.remove_face_from_its_group(fid);
        }
        let group_id = self.groups.insert(Group { name, face_ids: face_ids.to_vec() });
        for &fid in face_ids {
            self.face_to_group.insert(fid, group_id);
        }
        group_id
    }

    pub fn ungroup(&mut self, group_id: GroupId) {
        if let Some(group) = self.groups.remove(group_id) {
            for fid in group.face_ids {
                self.face_to_group.remove(&fid);
            }
        }
    }

    pub fn add_guide(&mut self, a: DVec3, b: DVec3) {
        self.guides.push(Guide { a, b });
    }

    pub fn clear_guides(&mut self) {
        self.guides.clear();
    }

    pub fn select(&mut self, face_ids: &[FaceId]) {
        self.selection.faces = face_ids.iter().copied().collect();
    }

    pub fn select_group(&mut self, group_id: GroupId) {
        if let Some(group) = self.groups.get(group_id) {
            self.selection.faces = group.face_ids.iter().copied().collect();
        }
    }

    fn remove_face_from_its_group(&mut self, face_id: FaceId) {
        if let Some(group_id) = self.face_to_group.remove(&face_id) {
            if let Some(group) = self.groups.get_mut(group_id) {
                group.face_ids.retain(|&f| f != face_id);
            }
        }
    }

    fn unique_vertices(&self, face_ids: &[FaceId]) -> HashSet<VertexId> {
        let mut vertices = HashSet::new();
        for &fid in face_ids {
            if let Some(face) = self.mesh.faces.get(fid) {
                vertices.extend(face.outer.iter().copied());
                for hole in &face.holes {
                    vertices.extend(hole.iter().copied());
                }
            }
        }
        vertices
    }

    /// Recomputes the normal of every face referencing any vertex in
    /// `moved` - not just the faces the user transformed. Vertices are
    /// shared within a solid, so transforming a subset of its faces also
    /// deforms the adjacent faces; leaving those with stale normals would
    /// skew later push/pulls (which extrude along the stored normal) and
    /// triangulation (which projects onto its plane).
    fn recompute_normals_touching(&mut self, moved: &HashSet<VertexId>) {
        let affected: Vec<FaceId> = self
            .mesh
            .faces
            .iter()
            .filter(|(_, face)| {
                face.outer.iter().chain(face.holes.iter().flatten()).any(|v| moved.contains(v))
            })
            .map(|(id, _)| id)
            .collect();
        for fid in affected {
            self.mesh.recompute_normal(fid);
        }
    }

    /// The faces forming solid boundaries (created by push_pull or inset
    /// splits of them) - the printable subset STL export writes. Flat
    /// sketch faces are excluded: they have zero thickness, so they can
    /// never print, and leaving one lying around must not block exporting
    /// the actual solids.
    pub fn solid_boundary_face_ids(&self) -> Vec<FaceId> {
        self.mesh.faces.keys().filter(|id| self.solid_face_ids.contains(id)).collect()
    }

    /// Runs the watertightness check STL export gates on, but keeps the
    /// detail: which edges are open and which faces border them, so the
    /// frontend can highlight the broken spot instead of leaving the user to
    /// hunt for it. Scoped exactly like `export_stl` and `arrange_for_print`
    /// (printable solids only - a flat sketch has no thickness and can't be
    /// "open"), and run per connected component so a document with several
    /// parts can report how many of them are actually broken.
    pub fn check_model(&self) -> ModelReport {
        let solid_ids = self.solid_boundary_face_ids();
        let components = self.mesh.connected_components(&solid_ids);

        let mut report = ModelReport {
            part_count: components.len(),
            broken_part_count: 0,
            open_edges: Vec::new(),
            duplicate_edges: Vec::new(),
            problem_face_ids: Vec::new(),
        };
        let to_edge = |(a, b): (VertexId, VertexId)| {
            let (pa, pb) = (self.mesh.position(a), self.mesh.position(b));
            ProblemEdge {
                a: [pa.x as f32, pa.y as f32, pa.z as f32],
                b: [pb.x as f32, pb.y as f32, pb.z as f32],
            }
        };

        for component in components {
            let issues = pushpull::check_manifold(&self.mesh, &component);
            if issues.is_empty() {
                continue;
            }
            report.broken_part_count += 1;
            report.open_edges.extend(issues.open_edges.into_iter().map(to_edge));
            report.duplicate_edges.extend(issues.duplicate_edges.into_iter().map(to_edge));
            report.problem_face_ids.extend(issues.problem_faces);
        }
        report
    }

    /// Moves every disconnected solid onto a non-overlapping grid in the XY
    /// plane, floor-aligned (each solid's lowest point lands at z=0) - a
    /// one-click "prepare for print" step. Each connected component of
    /// `solid_boundary_face_ids()` (see `Mesh::connected_components`) is
    /// treated as one printable part; flat, un-extruded sketch faces are
    /// excluded, same scoping as `export_stl`. A no-op if there's nothing
    /// printable.
    ///
    /// Every grid cell is sized to the *largest* part's footprint plus
    /// `PRINT_ARRANGE_SPACING`, so every part's own bounding box - which is
    /// no larger than that cell - fits entirely within the cell it's
    /// centered in. Since cells themselves sit on a regular, non-overlapping
    /// lattice, parts can never overlap regardless of their individual
    /// sizes or shapes. This is a simple correctness-first uniform grid, not
    /// a space-efficient bin-packing.
    pub fn arrange_for_print(&mut self) {
        let solid_ids = self.solid_boundary_face_ids();
        let components = self.mesh.connected_components(&solid_ids);
        if components.is_empty() {
            return;
        }

        struct Part {
            face_ids: Vec<FaceId>,
            min: DVec3,
            max: DVec3,
        }
        let mut parts: Vec<Part> = components
            .into_iter()
            .map(|face_ids| {
                let (min, max) = self.mesh.bounding_box(&face_ids);
                Part { face_ids, min, max }
            })
            .collect();

        // Deterministic reading-order placement - SlotMap iteration order
        // isn't guaranteed stable across edits.
        parts.sort_by(|a, b| {
            a.min.y.partial_cmp(&b.min.y).unwrap().then(a.min.x.partial_cmp(&b.min.x).unwrap())
        });

        let cols = (parts.len() as f64).sqrt().ceil() as usize;
        let cell_x = parts.iter().map(|p| p.max.x - p.min.x).fold(0.0, f64::max) + PRINT_ARRANGE_SPACING;
        let cell_y = parts.iter().map(|p| p.max.y - p.min.y).fold(0.0, f64::max) + PRINT_ARRANGE_SPACING;

        for (i, part) in parts.iter().enumerate() {
            let col = i % cols;
            let row = i / cols;
            let target_center_x = col as f64 * cell_x;
            let target_center_y = row as f64 * cell_y;
            let current_center_x = (part.min.x + part.max.x) / 2.0;
            let current_center_y = (part.min.y + part.max.y) / 2.0;
            let delta = DVec3::new(
                target_center_x - current_center_x,
                target_center_y - current_center_y,
                -part.min.z,
            );
            self.translate_faces(&part.face_ids, delta);
        }
    }

    /// Moves every disconnected object within `face_ids` independently down
    /// (or up) along Z so each one's own lowest point rests on the build
    /// plate (Z = 0). Mirrors `arrange_for_print`'s per-component floor
    /// alignment, scoped to the given selection instead of the whole
    /// document, and only touches Z (no X/Y repositioning).
    pub fn drop_to_plate(&mut self, face_ids: &[FaceId]) {
        for component in self.mesh.connected_components(face_ids) {
            let (min, _) = self.mesh.bounding_box(&component);
            self.translate_faces(&component, DVec3::new(0.0, 0.0, -min.z));
        }
    }

    /// Builds the full render/selection payload sent to the frontend after
    /// every mutating command. Documents at this app's scale (a spaceship
    /// part, not a large assembly) are small enough that resending
    /// everything is simpler and plenty fast - no incremental diffing.
    pub fn snapshot(&self) -> DocumentSnapshot {
        let mut vertex_index: HashMap<VertexId, u32> = HashMap::new();
        let mut vertices: Vec<[f32; 3]> = Vec::new();
        let mut faces = Vec::new();

        for (face_id, face) in self.mesh.faces.iter() {
            let mut intern = |vid: VertexId| -> u32 {
                *vertex_index.entry(vid).or_insert_with(|| {
                    let p = self.mesh.position(vid);
                    vertices.push([p.x as f32, p.y as f32, p.z as f32]);
                    (vertices.len() - 1) as u32
                })
            };

            let triangle_indices: Vec<[u32; 3]> = triangulate_face(&self.mesh, face)
                .into_iter()
                .map(|tri| [intern(tri[0]), intern(tri[1]), intern(tri[2])])
                .collect();
            // Boundary loops (outer + holes), separate from the triangles:
            // these are what the frontend draws as edge lines - the ear-clip
            // diagonals inside `triangle_indices` are a render/export detail,
            // not a visible border a user should see or snap to.
            let outer: Vec<u32> = face.outer.iter().map(|&v| intern(v)).collect();
            let holes: Vec<Vec<u32>> =
                face.holes.iter().map(|h| h.iter().map(|&v| intern(v)).collect()).collect();

            faces.push(FaceSnapshot {
                id: face_id,
                group_id: self.face_to_group.get(&face_id).copied(),
                triangles: triangle_indices,
                outer,
                holes,
                normal: [face.normal.x as f32, face.normal.y as f32, face.normal.z as f32],
            });
        }

        let groups = self
            .groups
            .iter()
            .map(|(id, g)| GroupSnapshot { id, name: g.name.clone() })
            .collect();

        DocumentSnapshot {
            vertices,
            faces,
            groups,
            selected_face_ids: self.selection.faces.iter().copied().collect(),
            guides: self.guides.clone(),
        }
    }

    /// Serializes this document to the flat, index-based project format
    /// (see `ProjectFile`'s doc comment for why it isn't just a direct
    /// serialization of `Document` itself).
    pub fn to_project_file(&self) -> ProjectFile {
        let mut vertex_index: HashMap<VertexId, u32> = HashMap::new();
        let mut vertices: Vec<[f64; 3]> = Vec::new();
        let mut face_index: HashMap<FaceId, u32> = HashMap::new();
        let mut faces = Vec::new();

        for (face_id, face) in self.mesh.faces.iter() {
            let mut intern = |vid: VertexId| -> u32 {
                *vertex_index.entry(vid).or_insert_with(|| {
                    let p = self.mesh.position(vid);
                    vertices.push([p.x, p.y, p.z]);
                    (vertices.len() - 1) as u32
                })
            };
            let outer: Vec<u32> = face.outer.iter().map(|&v| intern(v)).collect();
            let holes: Vec<Vec<u32>> = face.holes.iter().map(|h| h.iter().map(|&v| intern(v)).collect()).collect();
            face_index.insert(face_id, faces.len() as u32);
            faces.push(ProjectFace { outer, holes, solid: self.solid_face_ids.contains(&face_id) });
        }

        let groups = self
            .groups
            .values()
            .map(|g| ProjectGroup {
                name: g.name.clone(),
                face_indices: g.face_ids.iter().filter_map(|fid| face_index.get(fid).copied()).collect(),
            })
            .collect();

        ProjectFile { vertices, faces, groups, guides: self.guides.clone() }
    }

    /// Rebuilds a document from a loaded project file, re-interning every
    /// vertex/face/group into fresh mesh/slotmap ids.
    ///
    /// Every index in the file is untrusted input - the file may have been
    /// hand-edited, truncated, or written by a different version - so a
    /// reference that doesn't resolve drops the thing referencing it rather
    /// than indexing blindly. This has to be total: a panic here would
    /// happen inside `commands::load_project` while the `AppState` mutex is
    /// held, poisoning it and making every subsequent command panic on its
    /// own `lock().unwrap()` - i.e. one bad file would brick the running app,
    /// not just fail the load.
    pub fn from_project_file(project: &ProjectFile) -> Self {
        let mut doc = Document::new();

        // Index-stable: `None` marks a vertex that can't be used, so later
        // indices still line up with the file's own numbering.
        let vertex_ids: Vec<Option<VertexId>> = project
            .vertices
            .iter()
            .map(|p| {
                let pos = DVec3::new(p[0], p[1], p[2]);
                // NaN/infinity would silently poison every normal, bounding
                // box, and exported triangle downstream.
                pos.is_finite().then(|| doc.mesh.add_vertex(pos))
            })
            .collect();
        let resolve = |loop_indices: &[u32]| -> Option<Vec<VertexId>> {
            loop_indices.iter().map(|&i| vertex_ids.get(i as usize).copied().flatten()).collect()
        };

        let face_ids: Vec<Option<FaceId>> = project
            .faces
            .iter()
            .map(|pf| {
                // A loop of under 3 vertices encloses no area and has no
                // meaningful normal, so it can't be a face.
                let outer = resolve(&pf.outer).filter(|o| o.len() >= 3)?;
                let holes: Vec<Vec<VertexId>> =
                    pf.holes.iter().filter_map(|h| resolve(h).filter(|r| r.len() >= 3)).collect();
                let face_id = doc.mesh.add_face(outer, holes);
                if pf.solid {
                    doc.solid_face_ids.insert(face_id);
                }
                Some(face_id)
            })
            .collect();

        for group in &project.groups {
            let member_ids: Vec<FaceId> = group
                .face_indices
                .iter()
                .filter_map(|&i| face_ids.get(i as usize).copied().flatten())
                .collect();
            if member_ids.is_empty() {
                continue;
            }
            doc.group_faces(&member_ids, group.name.clone());
        }

        doc.guides = project.guides.iter().copied().filter(|g| g.a.is_finite() && g.b.is_finite()).collect();

        doc
    }
}

/// Which world plane a mirror reflects across: `X` is the plane x = pivot.x
/// (flips left/right), and so on. Deserialized from the lowercase strings
/// "x"/"y"/"z" the frontend sends.
#[derive(Debug, Clone, Copy, serde::Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum MirrorAxis {
    X,
    Y,
    Z,
}

/// Maps each vertex of `loop_verts` through `vertex_map` (inserting a freshly
/// cloned, `map_pos`-repositioned vertex on first sight of a given source
/// id, reusing it on every later sight - the sharing that keeps a cloned
/// solid manifold), then reverses the result if `reverse_winding`.
fn clone_loop_through_map(
    mesh: &mut Mesh,
    vertex_map: &mut HashMap<VertexId, VertexId>,
    loop_verts: &[VertexId],
    map_pos: &impl Fn(DVec3) -> DVec3,
    reverse_winding: bool,
) -> Vec<VertexId> {
    let mut cloned: Vec<VertexId> = loop_verts
        .iter()
        .map(|&v| {
            *vertex_map.entry(v).or_insert_with(|| {
                let p = mesh.position(v);
                mesh.add_vertex(map_pos(p))
            })
        })
        .collect();
    if reverse_winding {
        cloned.reverse();
    }
    cloned
}

/// Whether `subject` is the same ring of mesh vertices as any of
/// `candidates`. Compared as vertex sets: `face_detect` traces its cycles
/// from the same vertices the original loops were built from, but is free to
/// start anywhere in the ring and (for the opposite winding) to run the other
/// way round, so neither order nor starting point can be relied on.
fn matches_any_loop(subject: &[VertexId], candidates: &[Vec<VertexId>]) -> bool {
    if candidates.is_empty() {
        return false;
    }
    let subject_set: HashSet<VertexId> = subject.iter().copied().collect();
    candidates
        .iter()
        .any(|c| c.len() == subject.len() && c.iter().all(|v| subject_set.contains(v)))
}

/// Whether `subject` traces a ring that covers all of some candidate's
/// vertices - the same test as `matches_any_loop`, but tolerating `subject`
/// carrying *extra* vertices the candidate doesn't have.
///
/// Used only to decide whether a freshly detected region is one of the
/// `protected_holes` that must stay empty. It cannot be an exact comparison,
/// because `face_detect::split_edges_at_interior_points` inserts a vertex
/// into any edge a newly-drawn loop touches partway along (a T-junction), so
/// the region re-traced for a protected hole legitimately comes back one or
/// more vertices longer than the hole recorded before the split. A sketch
/// whose corner lands on an existing stud's rim does exactly that; comparing
/// by exact length there silently un-protected the rim and refilled it with a
/// face duplicating every edge the stud's wall already pairs with (see
/// `a_sketch_touching_an_existing_studs_rim_does_not_refill_that_rim`).
///
/// A false positive here *drops* a face, so the looser test has to stay
/// tight in that direction - and it does: the detected regions partition the
/// source face's area with the hole excluded from it, so the only region that
/// can contain *every* vertex of a hole's loop is that hole's own region. A
/// neighboring region touches only part of the loop, and a region enclosing
/// the hole carries it among its `holes` rather than in its outer.
///
/// Deliberately not folded into `matches_any_loop`: that one also answers
/// "did an erased face fill this hole" in `resplit_plane`, where a smaller
/// candidate loop genuinely is a different loop and must not match.
fn loop_covers_any(subject: &[VertexId], candidates: &[Vec<VertexId>]) -> bool {
    if candidates.is_empty() {
        return false;
    }
    let subject_set: HashSet<VertexId> = subject.iter().copied().collect();
    candidates
        .iter()
        .any(|c| c.len() <= subject.len() && c.iter().all(|v| subject_set.contains(v)))
}

/// A face is coplanar with `plane` when it faces the same way (not the
/// opposite side of the same plane) and every one of its outer-loop vertices
/// lies on it within tolerance.
fn is_coplanar(mesh: &Mesh, face: &Face, plane: &Plane) -> bool {
    const EPS: f64 = 1e-6;
    if face.normal.dot(plane.normal) < 1.0 - EPS {
        return false;
    }
    face.outer
        .iter()
        .all(|&vid| (mesh.position(vid) - plane.origin).dot(plane.normal).abs() < EPS)
}

impl Default for Document {
    fn default() -> Self {
        Self::new()
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct FaceSnapshot {
    pub id: FaceId,
    pub group_id: Option<GroupId>,
    pub triangles: Vec<[u32; 3]>,
    /// Outer boundary loop, as indices into `DocumentSnapshot::vertices`.
    pub outer: Vec<u32>,
    /// Hole boundary loops, same indexing.
    pub holes: Vec<Vec<u32>>,
    pub normal: [f32; 3],
}

#[derive(Debug, Clone, Serialize)]
pub struct GroupSnapshot {
    pub id: GroupId,
    pub name: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct DocumentSnapshot {
    pub vertices: Vec<[f32; 3]>,
    pub faces: Vec<FaceSnapshot>,
    pub groups: Vec<GroupSnapshot>,
    pub selected_face_ids: Vec<FaceId>,
    /// Kept as f64 (unlike `vertices`, which is f32 because it feeds a GPU
    /// buffer) - there are only ever a handful of guides, so no conversion
    /// code here is simpler than some.
    pub guides: Vec<Guide>,
}

/// One offending edge, in world coordinates rather than as indices into
/// `DocumentSnapshot::vertices`: that interning is per-call and depends on
/// face iteration order (see `Document::snapshot`), so index-based edges
/// could silently desync from whatever snapshot the frontend is currently
/// holding. Positions can't.
#[derive(Debug, Clone, Serialize)]
pub struct ProblemEdge {
    pub a: [f32; 3],
    pub b: [f32; 3],
}

/// The result of `Document::check_model` - what STL export needs to be
/// watertight, phrased so the frontend can both explain the problem and draw
/// it in the viewport.
#[derive(Debug, Clone, Serialize)]
pub struct ModelReport {
    /// Connected printable solids found (see `Mesh::connected_components`).
    pub part_count: usize,
    /// How many of those have at least one issue.
    pub broken_part_count: usize,
    pub open_edges: Vec<ProblemEdge>,
    pub duplicate_edges: Vec<ProblemEdge>,
    pub problem_face_ids: Vec<FaceId>,
}

impl ModelReport {
    pub fn is_watertight(&self) -> bool {
        self.open_edges.is_empty() && self.duplicate_edges.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn draw_push_pull_and_snapshot_round_trip() {
        let mut doc = Document::new();
        let plane = Plane::from_normal(DVec3::ZERO, DVec3::Z);
        let face_id = doc.draw_rectangle(&plane, DVec2::ZERO, DVec2::new(1.0, 1.0), None)[0];
        doc.push_pull(face_id, 1.0);
        let snapshot = doc.snapshot();
        assert_eq!(snapshot.faces.len(), 6);
        assert_eq!(snapshot.vertices.len(), 8); // a unit cube has 8 distinct corners
    }

    #[test]
    fn push_pull_with_a_stale_face_id_is_a_no_op_not_a_panic() {
        let mut doc = Document::new();
        let plane = Plane::from_normal(DVec3::ZERO, DVec3::Z);
        let face_id = doc.draw_rectangle(&plane, DVec2::ZERO, DVec2::new(1.0, 1.0), None)[0];
        doc.erase_face(face_id);
        assert_eq!(doc.push_pull(face_id, 1.0), Vec::new());
    }

    #[test]
    fn push_pull_faces_extrudes_every_selected_face_along_its_own_normal() {
        let mut doc = Document::new();
        let plane = Plane::from_normal(DVec3::ZERO, DVec3::Z);
        doc.draw_rectangle(&plane, DVec2::new(0.0, 0.0), DVec2::new(1.0, 1.0), None);
        doc.draw_rectangle(&plane, DVec2::new(5.0, 5.0), DVec2::new(6.0, 6.0), None);
        // Both rectangles are coplanar, so drawing the second one resplits
        // (and reassigns the FaceId of) the first - look both up fresh
        // afterward rather than trusting either draw call's return value.
        let current_ids: Vec<FaceId> = doc.mesh.faces.iter().map(|(id, _)| id).collect();
        assert_eq!(current_ids.len(), 2);

        let new_faces = doc.push_pull_faces(&current_ids, 1.0);

        // Each flat rectangle became its own 6-face box.
        assert_eq!(new_faces.len(), 12);
        assert!(pushpull::is_manifold(&doc.mesh, &new_faces));
        assert_eq!(doc.mesh.faces.len(), 12);
    }

    #[test]
    fn draw_arc_at_180_degrees_is_a_pushable_manifold_semicircle() {
        // The exact sweep a half-pipe cross-section needs, and the one angle
        // where a center-vertex ("pie") closure would have been numerically
        // fragile - the chord closure this tool uses has no center vertex,
        // so this must stay manifold with no special-casing.
        let mut doc = Document::new();
        let plane = Plane::from_normal(DVec3::ZERO, DVec3::Z);
        let face_ids = doc.draw_arc(&plane, DVec2::ZERO, 5.0, 0.0, 180.0, 16, None);
        assert_eq!(face_ids.len(), 1);

        let new_faces = doc.push_pull(face_ids[0], 3.0);
        assert!(pushpull::is_manifold(&doc.mesh, &new_faces), "extruded semicircle should be a watertight solid");
    }

    #[test]
    fn arc_inset_then_pushpull_makes_a_hollow_manifold_half_pipe() {
        let mut doc = Document::new();
        let plane = Plane::from_normal(DVec3::ZERO, DVec3::Z);
        let face_id = doc.draw_arc(&plane, DVec2::ZERO, 10.0, 0.0, 180.0, 16, None)[0];

        let inset_faces = doc.inset_face(face_id, 1.0);
        assert_eq!(inset_faces.len(), 2);
        let frame_id = inset_faces
            .iter()
            .copied()
            .find(|&id| !doc.mesh.faces[id].holes.is_empty())
            .expect("inset should produce an outer frame face with a hole");

        let new_faces = doc.push_pull(frame_id, 4.0);
        assert!(pushpull::is_manifold(&doc.mesh, &new_faces), "extruded ring-shaped arc frame should be a hollow, watertight half-pipe");
    }

    #[test]
    fn draw_polygon_creates_a_pushable_face() {
        let mut doc = Document::new();
        let plane = Plane::from_normal(DVec3::ZERO, DVec3::Z);
        // An L-shaped pentagon - not a primitive shape rect/circle can make.
        let points = vec![
            DVec2::new(0.0, 0.0),
            DVec2::new(2.0, 0.0),
            DVec2::new(2.0, 1.0),
            DVec2::new(1.0, 1.0),
            DVec2::new(1.0, 2.0),
            DVec2::new(0.0, 2.0),
        ];
        let face_ids = doc.draw_polygon(&plane, points, None);
        assert_eq!(face_ids.len(), 1);
        assert_eq!(doc.mesh.faces[face_ids[0]].outer.len(), 6);

        let new_faces = doc.push_pull(face_ids[0], 1.0);
        assert!(pushpull::is_manifold(&doc.mesh, &new_faces));
    }

    #[test]
    fn draw_polygon_with_fewer_than_three_points_does_nothing() {
        let mut doc = Document::new();
        let plane = Plane::from_normal(DVec3::ZERO, DVec3::Z);
        let face_ids = doc.draw_polygon(&plane, vec![DVec2::ZERO, DVec2::new(1.0, 0.0)], None);
        assert!(face_ids.is_empty());
        assert!(doc.mesh.faces.is_empty());
    }

    #[test]
    fn erase_face_removes_it_from_its_group() {
        let mut doc = Document::new();
        let plane = Plane::from_normal(DVec3::ZERO, DVec3::Z);
        let face_id = doc.draw_rectangle(&plane, DVec2::ZERO, DVec2::new(1.0, 1.0), None)[0];
        let group_id = doc.group_faces(&[face_id], "hull".to_string());
        doc.erase_face(face_id);
        assert!(doc.groups[group_id].face_ids.is_empty());
    }

    #[test]
    fn translate_moves_only_the_given_faces() {
        let mut doc = Document::new();
        let plane = Plane::from_normal(DVec3::ZERO, DVec3::Z);
        doc.draw_rectangle(&plane, DVec2::ZERO, DVec2::new(1.0, 1.0), None);
        doc.draw_rectangle(&plane, DVec2::new(5.0, 5.0), DVec2::new(6.0, 6.0), None);

        // Both rectangles are coplanar, so drawing the second one resplits
        // (and thus reassigns the FaceId of) both - look them up by position
        // afterward rather than trusting either draw call's return value.
        let mut ids = doc.mesh.faces.iter().map(|(id, _)| id);
        let first = ids.next().unwrap();
        let second = ids.next().unwrap();
        let (moved, stationary) = if doc.mesh.position(doc.mesh.faces[first].outer[0]).x < 3.0 {
            (first, second)
        } else {
            (second, first)
        };
        let stationary_before = doc.mesh.faces[stationary].outer.clone();

        doc.translate_faces(&[moved], DVec3::new(10.0, 0.0, 0.0));

        let stationary_after = &doc.mesh.faces[stationary].outer;
        for (&before, &after) in stationary_before.iter().zip(stationary_after.iter()) {
            assert_eq!(doc.mesh.position(before), doc.mesh.position(after));
        }
        let moved_vertex = doc.mesh.position(doc.mesh.faces[moved].outer[0]);
        assert!(moved_vertex.x >= 10.0);
    }

    #[test]
    fn an_earlier_solid_left_coplanar_with_the_ground_stays_out_of_later_resplits() {
        // Pushing/pulling DOWNWARD leaves the resulting solid's top cap
        // sitting exactly on the ground plane (z=0), facing +Z - exactly
        // coplanar with the ground plane by every measure resplit_plane
        // checks. A later, unrelated sketch on that same plane must not
        // sweep that cap up into its own face-splitting.
        let mut doc = Document::new();
        let plane = Plane::from_normal(DVec3::ZERO, DVec3::Z);

        let first_id = doc.draw_rectangle(&plane, DVec2::new(-10.0, -10.0), DVec2::new(-8.0, -8.0), None)[0];
        doc.push_pull(first_id, -2.0);
        let solid_face_count = doc.mesh.faces.len();
        assert_eq!(solid_face_count, 6); // a box

        doc.draw_circle(&plane, DVec2::ZERO, 5.0, 16, None);
        doc.draw_circle(&plane, DVec2::ZERO, 2.0, 16, None);

        assert_eq!(doc.mesh.faces.len(), solid_face_count + 2, "old solid's faces must be untouched");
        let faces_with_holes = doc.mesh.faces.values().filter(|f| !f.holes.is_empty()).count();
        assert_eq!(faces_with_holes, 1, "only the new outer ring should have a hole");
    }

    #[test]
    fn drawing_a_circle_inside_another_auto_splits_into_ring_and_inner_face() {
        let mut doc = Document::new();
        let plane = Plane::from_normal(DVec3::ZERO, DVec3::Z);
        doc.draw_circle(&plane, DVec2::ZERO, 5.0, 16, None);
        doc.draw_circle(&plane, DVec2::ZERO, 2.0, 16, None);

        assert_eq!(doc.mesh.faces.len(), 2);
        let faces_with_holes = doc.mesh.faces.values().filter(|f| !f.holes.is_empty()).count();
        assert_eq!(faces_with_holes, 1, "exactly one face (the outer ring) should have a hole");
    }

    #[test]
    fn erasing_inner_circle_leaves_the_outer_as_a_printable_ring() {
        let mut doc = Document::new();
        let plane = Plane::from_normal(DVec3::ZERO, DVec3::Z);
        doc.draw_circle(&plane, DVec2::ZERO, 5.0, 16, None);
        doc.draw_circle(&plane, DVec2::ZERO, 2.0, 16, None);

        let inner_face_id = doc.mesh.faces.iter().find(|(_, f)| f.holes.is_empty()).unwrap().0;
        doc.erase_face(inner_face_id);

        assert_eq!(doc.mesh.faces.len(), 1);
        let ring = doc.mesh.faces.values().next().unwrap();
        assert_eq!(ring.holes.len(), 1, "outer face should still have its hole after erasing the inner disk");

        let ring_id = doc.mesh.faces.iter().next().unwrap().0;
        let new_faces = doc.push_pull(ring_id, 3.0);
        assert!(pushpull::is_manifold(&doc.mesh, &new_faces), "extruded ring should be a hollow, watertight tube");
    }

    #[test]
    fn draw_ngon_creates_a_pushable_hexagon() {
        let mut doc = Document::new();
        let plane = Plane::from_normal(DVec3::ZERO, DVec3::Z);
        let face_id = doc.draw_ngon(&plane, DVec2::ZERO, 5.0, 6, 0.0, None)[0];
        assert_eq!(doc.mesh.faces[face_id].outer.len(), 6);

        let new_faces = doc.push_pull(face_id, 3.0);
        assert!(pushpull::is_manifold(&doc.mesh, &new_faces), "extruded hexagon should be a watertight solid");
    }

    #[test]
    fn drawing_an_ngon_inside_a_circle_auto_splits_into_ring_and_inner_face() {
        let mut doc = Document::new();
        let plane = Plane::from_normal(DVec3::ZERO, DVec3::Z);
        doc.draw_circle(&plane, DVec2::ZERO, 5.0, 16, None);
        doc.draw_ngon(&plane, DVec2::ZERO, 2.0, 6, 0.0, None);

        assert_eq!(doc.mesh.faces.len(), 2);
        let faces_with_holes = doc.mesh.faces.values().filter(|f| !f.holes.is_empty()).count();
        assert_eq!(faces_with_holes, 1, "exactly one face (the outer ring) should have a hole");
    }

    #[test]
    fn inset_face_splits_a_rectangle_into_an_inner_face_and_a_framed_hole() {
        let mut doc = Document::new();
        let plane = Plane::from_normal(DVec3::ZERO, DVec3::Z);
        let face_id = doc.draw_rectangle(&plane, DVec2::ZERO, DVec2::new(10.0, 10.0), None)[0];

        let new_faces = doc.inset_face(face_id, 2.0);
        assert_eq!(new_faces.len(), 2);
        assert_eq!(doc.mesh.faces.len(), 2);

        let faces_with_holes = doc.mesh.faces.values().filter(|f| !f.holes.is_empty()).count();
        assert_eq!(faces_with_holes, 1, "the outer frame should have the inset loop as a hole");
        let inner = doc.mesh.faces.values().find(|f| f.holes.is_empty()).unwrap();
        assert_eq!(inner.outer.len(), 4);
        for &vid in &inner.outer {
            let p = doc.mesh.position(vid);
            assert!(p.x > 1.9 && p.x < 8.1 && p.y > 1.9 && p.y < 8.1);
        }
    }

    #[test]
    fn inset_face_preserves_group_and_solid_membership() {
        let mut doc = Document::new();
        let plane = Plane::from_normal(DVec3::ZERO, DVec3::Z);
        let face_id = doc.draw_rectangle(&plane, DVec2::ZERO, DVec2::new(10.0, 10.0), None)[0];
        let group_id = doc.group_faces(&[face_id], "panel".to_string());

        let new_faces = doc.inset_face(face_id, 2.0);
        assert_eq!(doc.groups[group_id].face_ids.len(), 2);
        for &fid in &new_faces {
            assert!(doc.groups[group_id].face_ids.contains(&fid));
        }
    }

    #[test]
    fn inset_face_on_a_triangle_produces_a_non_overlapping_frame_triangulation() {
        // A rectangle's inset hole happens to stay symmetric even if
        // face_detect ever regresses hole winding, which is what let that
        // bug slip through undetected for a while - a triangle's inset hole
        // doesn't have that accidental symmetry, so this is a much more
        // sensitive check: total triangulated area of the frame face must
        // not exceed the original face's area (it would if the hole loop's
        // winding were wrong, since bridge_hole would then produce a
        // self-overlapping "simple" polygon instead of a true ring).
        let mut doc = Document::new();
        let plane = Plane::from_normal(DVec3::ZERO, DVec3::Z);
        let points = vec![DVec2::new(0.0, 0.0), DVec2::new(4.0, 0.0), DVec2::new(2.0, 3.0)];
        let tri_id = doc.draw_polygon(&plane, points, None)[0];
        let outer_area = 6.0; // base 4 * height 3 / 2

        let new_faces = doc.inset_face(tri_id, 0.3);
        let frame_id = *new_faces.iter().find(|&&fid| !doc.mesh.faces[fid].holes.is_empty()).unwrap();
        let frame_face = doc.mesh.faces[frame_id].clone();
        let triangles = triangulate_face(&doc.mesh, &frame_face);
        let total_area: f64 = triangles
            .iter()
            .map(|tri| {
                let pts: Vec<DVec3> = tri.iter().map(|&v| doc.mesh.position(v)).collect();
                (pts[1] - pts[0]).cross(pts[2] - pts[0]).length() * 0.5
            })
            .sum();
        assert!(total_area < outer_area, "frame triangulation area ({total_area}) exceeds the original face's area ({outer_area}) - overlapping triangles");
    }

    #[test]
    fn inset_face_on_an_angled_push_pull_side_wall_produces_a_non_overlapping_frame() {
        // The user-reported case: insetting a face whose normal isn't axis
        // aligned (a side wall from pushing/pulling a non-rectangular
        // polygon). The plane-basis math is orientation-agnostic, so this
        // is really the same bug as the ground-plane triangle case above,
        // just via the path an actual user hit it through.
        let mut doc = Document::new();
        let plane = Plane::from_normal(DVec3::ZERO, DVec3::Z);
        let points = vec![DVec2::new(0.0, 0.0), DVec2::new(4.0, 0.0), DVec2::new(2.0, 3.0)];
        let tri_id = doc.draw_polygon(&plane, points, None)[0];
        let solid_faces = doc.push_pull(tri_id, 2.0);
        let angled_id = solid_faces
            .iter()
            .copied()
            .find(|&fid| {
                let n = doc.mesh.faces[fid].normal;
                n.z.abs() < 0.01 && n.x.abs() > 0.05 && n.y.abs() > 0.05
            })
            .expect("should find a side wall with a non-axis-aligned normal");
        let outer_area = {
            // The side wall is a planar quad (p0,p1,p2,p3) - split along one
            // diagonal into two triangles to get its area.
            let f = &doc.mesh.faces[angled_id];
            let pts: Vec<DVec3> = f.outer.iter().map(|&v| doc.mesh.position(v)).collect();
            (pts[1] - pts[0]).cross(pts[2] - pts[0]).length() * 0.5
                + (pts[2] - pts[0]).cross(pts[3] - pts[0]).length() * 0.5
        };

        let new_faces = doc.inset_face(angled_id, 0.3);
        assert!(!new_faces.is_empty());
        let frame_id = *new_faces.iter().find(|&&fid| !doc.mesh.faces[fid].holes.is_empty()).unwrap();
        let frame_face = doc.mesh.faces[frame_id].clone();
        let triangles = triangulate_face(&doc.mesh, &frame_face);
        let total_area: f64 = triangles
            .iter()
            .map(|tri| {
                let pts: Vec<DVec3> = tri.iter().map(|&v| doc.mesh.position(v)).collect();
                (pts[1] - pts[0]).cross(pts[2] - pts[0]).length() * 0.5
            })
            .sum();
        assert!(total_area < outer_area, "frame triangulation area ({total_area}) exceeds the angled wall's own area ({outer_area}) - overlapping triangles");
    }

    #[test]
    fn inset_face_with_an_offset_too_large_for_the_shape_is_a_no_op() {
        let mut doc = Document::new();
        let plane = Plane::from_normal(DVec3::ZERO, DVec3::Z);
        let face_id = doc.draw_rectangle(&plane, DVec2::ZERO, DVec2::new(2.0, 2.0), None)[0];

        assert_eq!(doc.inset_face(face_id, 5.0), Vec::new());
        assert_eq!(doc.mesh.faces.len(), 1, "the original face must be untouched");
    }

    #[test]
    fn inset_face_with_a_stale_face_id_is_a_no_op_not_a_panic() {
        let mut doc = Document::new();
        let plane = Plane::from_normal(DVec3::ZERO, DVec3::Z);
        let face_id = doc.draw_rectangle(&plane, DVec2::ZERO, DVec2::new(2.0, 2.0), None)[0];
        doc.erase_face(face_id);
        assert_eq!(doc.inset_face(face_id, 0.5), Vec::new());
    }

    #[test]
    fn pulling_a_solids_cap_extends_it_into_one_manifold_solid() {
        // The most common action after making a box: pull its top cap to
        // make it taller. This must merge into a single watertight shell,
        // not stack a second closed box (with a buried interior cap) on top
        // of a now-open one.
        let mut doc = Document::new();
        let plane = Plane::from_normal(DVec3::ZERO, DVec3::Z);
        let sketch_id = doc.draw_rectangle(&plane, DVec2::ZERO, DVec2::new(1.0, 1.0), None)[0];
        let box_faces = doc.push_pull(sketch_id, 1.0);

        let top_cap = box_faces
            .iter()
            .copied()
            .find(|&fid| doc.mesh.faces[fid].normal.z > 0.9)
            .expect("box should have an upward-facing cap");
        doc.push_pull(top_cap, 1.0);

        let all_faces: Vec<FaceId> = doc.mesh.faces.keys().collect();
        // 5 remaining original faces + 4 new wall quads + 1 new top cap.
        assert_eq!(all_faces.len(), 10);
        assert!(pushpull::is_manifold(&doc.mesh, &all_faces), "extended box must stay watertight");
        let top_z = doc.mesh.vertices.values().map(|v| v.position.z).fold(f64::MIN, f64::max);
        assert!((top_z - 2.0).abs() < 1e-9);
    }

    /// The slab's top cap: the z=2 upward face still carrying the original
    /// rectangle's 4-vertex outer loop. Looked up fresh on every use because
    /// each resplit erases and recreates it.
    fn slab_top_cap(doc: &Document) -> FaceId {
        doc.mesh
            .faces
            .iter()
            .find(|(_, f)| f.normal.z > 0.9 && f.outer.len() == 4 && (doc.mesh.position(f.outer[0]).z - 2.0).abs() < 1e-9)
            .map(|(id, _)| id)
            .expect("slab top cap")
    }

    #[test]
    fn a_second_stud_drawn_on_a_solid_keeps_the_first_stud_watertight() {
        // Two circles sketched on the same face of a solid and extruded into
        // studs. The second sketch resplits a face that already carries the
        // FIRST stud's rim as a hole - that hole must stay a hole, because
        // the first stud's wall already closes it in 3D. Refilling it with a
        // disc duplicates every edge around that rim.
        let mut doc = Document::new();
        let ground = Plane::from_normal(DVec3::ZERO, DVec3::Z);
        let sketch_id = doc.draw_rectangle(&ground, DVec2::ZERO, DVec2::new(30.0, 10.0), None)[0];
        doc.push_pull(sketch_id, 2.0);
        // Same 2D frame as `ground`, just lifted to the slab's top face, so
        // circle centers below read in the rectangle's own coordinates.
        let top_plane = Plane::from_normal(DVec3::new(0.0, 0.0, 2.0), DVec3::Z);

        let split = doc.draw_circle(&top_plane, DVec2::new(7.0, 5.0), 3.0, 16, Some(slab_top_cap(&doc)));
        let disc = split.iter().copied().find(|&fid| doc.mesh.faces[fid].holes.is_empty()).unwrap();
        doc.push_pull(disc, 3.0);
        assert!(
            pushpull::is_manifold(&doc.mesh, &doc.solid_boundary_face_ids()),
            "one stud on a slab must be watertight"
        );

        // The second circle, drawn on that same now-holed top cap.
        doc.draw_circle(&top_plane, DVec2::new(23.0, 5.0), 3.0, 16, Some(slab_top_cap(&doc)));
        assert!(
            pushpull::is_manifold(&doc.mesh, &doc.solid_boundary_face_ids()),
            "sketching a second circle must not refill the first stud's rim"
        );

        let second_disc = doc
            .mesh
            .faces
            .iter()
            .find(|(_, f)| {
                f.normal.z > 0.9 && f.holes.is_empty() && (doc.mesh.position(f.outer[0]).z - 2.0).abs() < 1e-9
            })
            .map(|(id, _)| id)
            .expect("the new circle's disc");
        doc.push_pull(second_disc, 3.0);
        assert!(
            pushpull::is_manifold(&doc.mesh, &doc.solid_boundary_face_ids()),
            "two studs on one slab must be watertight"
        );
    }

    #[test]
    fn drawing_elsewhere_on_the_ground_does_not_refill_an_erased_hole() {
        // Same root cause as the two-stud test, on the flat-sketch path:
        // ring on the ground (circle drawn inside a rectangle, inner disc
        // erased), then an unrelated rectangle drawn coplanar. The
        // document-wide resplit re-detects over every coplanar loop - the
        // ring's hole among them - and must not hand the erased disc back.
        let mut doc = Document::new();
        let plane = Plane::from_normal(DVec3::ZERO, DVec3::Z);
        doc.draw_rectangle(&plane, DVec2::ZERO, DVec2::new(10.0, 10.0), None);
        let split = doc.draw_circle(&plane, DVec2::new(5.0, 5.0), 2.0, 16, None);
        let disc = split.iter().copied().find(|&fid| doc.mesh.faces[fid].holes.is_empty()).unwrap();
        doc.erase_face(disc);
        assert_eq!(doc.mesh.faces.len(), 1, "a ring: one face with one hole");
        assert_eq!(doc.mesh.faces.values().next().unwrap().holes.len(), 1);

        doc.draw_rectangle(&plane, DVec2::new(20.0, 20.0), DVec2::new(25.0, 25.0), None);

        let ring = doc
            .mesh
            .faces
            .values()
            .find(|f| f.outer.iter().any(|&v| doc.mesh.position(v).x < 15.0))
            .expect("the ring");
        assert_eq!(ring.holes.len(), 1, "the ring must still have its hole");
        assert_eq!(doc.mesh.faces.len(), 2, "just the ring and the new rectangle - no refilled disc");
    }

    #[test]
    fn raised_panel_greeble_stays_manifold() {
        // Box -> inset its top cap -> pull the inner panel up: the raised-
        // panel greeble workflow this app exists for.
        let mut doc = Document::new();
        let plane = Plane::from_normal(DVec3::ZERO, DVec3::Z);
        let sketch_id = doc.draw_rectangle(&plane, DVec2::ZERO, DVec2::new(10.0, 10.0), None)[0];
        let box_faces = doc.push_pull(sketch_id, 2.0);
        let top_cap = box_faces.iter().copied().find(|&fid| doc.mesh.faces[fid].normal.z > 0.9).unwrap();

        let split = doc.inset_face(top_cap, 2.0);
        let inner = split.iter().copied().find(|&fid| doc.mesh.faces[fid].holes.is_empty()).unwrap();
        doc.push_pull(inner, 1.0);

        let all_faces: Vec<FaceId> = doc.mesh.faces.keys().collect();
        assert!(pushpull::is_manifold(&doc.mesh, &all_faces), "raised panel must stay watertight");
    }

    #[test]
    fn recessed_panel_greeble_stays_manifold() {
        // Same as above but pushing the inner panel INTO the solid - a
        // recessed panel line.
        let mut doc = Document::new();
        let plane = Plane::from_normal(DVec3::ZERO, DVec3::Z);
        let sketch_id = doc.draw_rectangle(&plane, DVec2::ZERO, DVec2::new(10.0, 10.0), None)[0];
        let box_faces = doc.push_pull(sketch_id, 2.0);
        let top_cap = box_faces.iter().copied().find(|&fid| doc.mesh.faces[fid].normal.z > 0.9).unwrap();

        let split = doc.inset_face(top_cap, 2.0);
        let inner = split.iter().copied().find(|&fid| doc.mesh.faces[fid].holes.is_empty()).unwrap();
        doc.push_pull(inner, -0.5);

        let all_faces: Vec<FaceId> = doc.mesh.faces.keys().collect();
        assert!(pushpull::is_manifold(&doc.mesh, &all_faces), "recessed panel must stay watertight");
        // The recess floor must face upward (out of the pocket), not down.
        let floor = doc
            .mesh
            .faces
            .values()
            .find(|f| f.outer.iter().all(|&v| (doc.mesh.position(v).z - 1.5).abs() < 1e-9))
            .expect("recess floor at z=1.5");
        assert!(floor.normal.z > 0.9, "recess floor normal should point up, got {:?}", floor.normal);
    }

    #[test]
    fn moving_part_of_a_solid_keeps_every_affected_normal_accurate() {
        // Moving one cap of a box drags the shared vertices of all four
        // walls with it; the walls' stored normals must be recomputed too,
        // not just the moved cap's.
        let mut doc = Document::new();
        let plane = Plane::from_normal(DVec3::ZERO, DVec3::Z);
        let sketch_id = doc.draw_rectangle(&plane, DVec2::ZERO, DVec2::new(1.0, 1.0), None)[0];
        let box_faces = doc.push_pull(sketch_id, 1.0);
        let top_cap = box_faces.iter().copied().find(|&fid| doc.mesh.faces[fid].normal.z > 0.9).unwrap();

        doc.translate_faces(&[top_cap], DVec3::new(0.5, 0.0, 0.0));

        for face in doc.mesh.faces.values() {
            let points: Vec<DVec3> = face.outer.iter().map(|&v| doc.mesh.position(v)).collect();
            let actual = crate::geometry::mesh::newell_normal(&points);
            assert!(
                (face.normal - actual).length() < 1e-9,
                "stored normal {:?} doesn't match geometry normal {:?}",
                face.normal,
                actual
            );
        }
    }

    #[test]
    fn solid_boundary_face_ids_excludes_flat_sketches() {
        let mut doc = Document::new();
        let plane = Plane::from_normal(DVec3::ZERO, DVec3::Z);
        let sketch_id = doc.draw_rectangle(&plane, DVec2::ZERO, DVec2::new(1.0, 1.0), None)[0];
        doc.push_pull(sketch_id, 1.0);
        // A leftover construction sketch elsewhere on the ground plane.
        doc.draw_circle(&plane, DVec2::new(10.0, 10.0), 1.0, 8, None);

        let solids = doc.solid_boundary_face_ids();
        assert_eq!(solids.len(), 6, "only the box's faces are printable solids");
        assert!(pushpull::is_manifold(&doc.mesh, &solids), "the solid subset must be watertight on its own");
    }

    #[test]
    fn push_pull_with_zero_distance_keeps_group_and_solid_membership() {
        let mut doc = Document::new();
        let plane = Plane::from_normal(DVec3::ZERO, DVec3::Z);
        let sketch_id = doc.draw_rectangle(&plane, DVec2::ZERO, DVec2::new(1.0, 1.0), None)[0];
        let box_faces = doc.push_pull(sketch_id, 1.0);
        let group_id = doc.group_faces(&box_faces, "hull".to_string());
        let cap = box_faces[0];

        assert!(doc.push_pull(cap, 0.0).is_empty());
        assert!(doc.groups[group_id].face_ids.contains(&cap), "zero-distance push/pull must not eject the face from its group");
        assert_eq!(doc.solid_boundary_face_ids().len(), 6, "zero-distance push/pull must not strip solid status");
    }

    #[test]
    fn duplicate_of_a_box_is_an_independent_manifold_copy() {
        let mut doc = Document::new();
        let plane = Plane::from_normal(DVec3::ZERO, DVec3::Z);
        let sketch_id = doc.draw_rectangle(&plane, DVec2::ZERO, DVec2::new(1.0, 1.0), None)[0];
        let box_faces = doc.push_pull(sketch_id, 1.0);
        assert_eq!(doc.mesh.vertices.len(), 8);

        let copy_faces = doc.duplicate_faces(&box_faces, DVec3::new(5.0, 0.0, 0.0));

        assert_eq!(copy_faces.len(), 6);
        assert!(pushpull::is_manifold(&doc.mesh, &copy_faces), "duplicated box must be manifold on its own");
        assert_eq!(
            doc.mesh.vertices.len(),
            16,
            "copy must clone its own 8 distinct vertices, reusing them across its 6 faces just like the source"
        );
        assert_eq!(doc.solid_boundary_face_ids().len(), 12, "the copy must inherit solid-boundary status from its source faces");
        let selected: HashSet<FaceId> = copy_faces.iter().copied().collect();
        assert_eq!(doc.selection.faces, selected, "the copy should become the new selection");

        for &vid in &doc.mesh.faces[box_faces[0]].outer {
            let x = doc.mesh.position(vid).x;
            assert!((0.0..=1.0).contains(&x), "duplicating must not move the source geometry, x={x}");
        }
    }

    #[test]
    fn array_of_a_box_produces_a_grid_of_independent_manifold_copies() {
        let mut doc = Document::new();
        let plane = Plane::from_normal(DVec3::ZERO, DVec3::Z);
        let sketch_id = doc.draw_rectangle(&plane, DVec2::ZERO, DVec2::new(1.0, 1.0), None)[0];
        let box_faces = doc.push_pull(sketch_id, 1.0);

        let copies = doc.array_faces(&box_faces, 3, 2, 30.0, 30.0);

        assert_eq!(copies.len(), 5 * 6, "3 x 2 counts the source, so 5 cells get a 6-faced copy");
        assert_eq!(doc.mesh.vertices.len(), 8 * 6, "each copy clones its own 8 distinct vertices, shared across its 6 faces");
        assert_eq!(doc.solid_boundary_face_ids().len(), 6 * 6, "every copy must inherit solid-boundary status");

        let solids = doc.solid_boundary_face_ids();
        let components = doc.mesh.connected_components(&solids);
        assert_eq!(components.len(), 6, "the grid must be 6 separate objects, not one fused blob");
        for component in &components {
            assert!(pushpull::is_manifold(&doc.mesh, component), "each box in the array must be manifold on its own");
        }

        for &vid in &doc.mesh.faces[box_faces[0]].outer {
            let x = doc.mesh.position(vid).x;
            assert!((0.0..=1.0).contains(&x), "arraying must not move the source geometry, x={x}");
        }

        let expected: HashSet<FaceId> = box_faces.iter().chain(copies.iter()).copied().collect();
        assert_eq!(doc.selection.faces, expected, "the whole grid, source included, should become the new selection");
    }

    #[test]
    fn array_pitch_is_center_to_center() {
        let mut doc = Document::new();
        let plane = Plane::from_normal(DVec3::ZERO, DVec3::Z);
        let sketch_id = doc.draw_rectangle(&plane, DVec2::ZERO, DVec2::new(20.0, 20.0), None)[0];
        let box_faces = doc.push_pull(sketch_id, 5.0);

        doc.array_faces(&box_faces, 3, 2, 30.0, 40.0);

        let all: Vec<FaceId> = doc.mesh.faces.keys().collect();
        let (min, max) = doc.mesh.bounding_box(&all);
        assert!((min.x - 0.0).abs() < 1e-9 && (min.y - 0.0).abs() < 1e-9, "the grid should start at the source, min={min}");
        // Last column's near edge sits at 2 * pitch, plus the part's own 20mm.
        assert!((max.x - (2.0 * 30.0 + 20.0)).abs() < 1e-9, "3 columns at pitch 30 should span 80mm, got {}", max.x);
        assert!((max.y - (1.0 * 40.0 + 20.0)).abs() < 1e-9, "2 rows at pitch 40 should span 60mm, got {}", max.y);
    }

    #[test]
    fn array_of_one_by_one_creates_nothing() {
        let mut doc = Document::new();
        let plane = Plane::from_normal(DVec3::ZERO, DVec3::Z);
        let sketch_id = doc.draw_rectangle(&plane, DVec2::ZERO, DVec2::new(1.0, 1.0), None)[0];
        let box_faces = doc.push_pull(sketch_id, 1.0);
        let before = doc.mesh.faces.len();
        doc.select(&box_faces);

        let copies = doc.array_faces(&box_faces, 1, 1, 30.0, 30.0);

        assert!(copies.is_empty(), "a 1 x 1 array is just the source - nothing to copy");
        assert_eq!(doc.mesh.faces.len(), before, "a no-op array must not touch the mesh");
        assert_eq!(doc.selection.faces, box_faces.iter().copied().collect::<HashSet<FaceId>>(), "a no-op array must leave the selection alone");
    }

    #[test]
    fn mirroring_a_box_leaves_the_original_untouched_and_produces_an_outward_facing_copy() {
        let mut doc = Document::new();
        let plane = Plane::from_normal(DVec3::ZERO, DVec3::Z);
        let sketch_id = doc.draw_rectangle(&plane, DVec2::new(5.0, 5.0), DVec2::new(6.0, 6.0), None)[0];
        let box_faces = doc.push_pull(sketch_id, 1.0);

        let mirrored = doc.mirror_faces(&box_faces, MirrorAxis::X, DVec3::ZERO);

        assert_eq!(mirrored.len(), 6);
        assert!(pushpull::is_manifold(&doc.mesh, &mirrored), "mirrored copy must be manifold - winding must survive the reflection");

        for &vid in &doc.mesh.faces[box_faces[0]].outer {
            let x = doc.mesh.position(vid).x;
            assert!((4.999..=6.001).contains(&x), "mirroring must not move the source geometry, x={x}");
        }

        let copy_points: Vec<DVec3> =
            mirrored.iter().flat_map(|&f| doc.mesh.faces[f].outer.iter().map(|&v| doc.mesh.position(v))).collect();
        assert!(copy_points.iter().all(|p| p.x <= -4.999), "mirrored copy should sit entirely on the opposite side of x=0");

        let centroid = copy_points.iter().fold(DVec3::ZERO, |acc, &p| acc + p) / copy_points.len() as f64;
        for &fid in &mirrored {
            let face = &doc.mesh.faces[fid];
            let face_centroid =
                face.outer.iter().map(|&v| doc.mesh.position(v)).fold(DVec3::ZERO, |acc, p| acc + p) / face.outer.len() as f64;
            assert!(face.normal.dot(face_centroid - centroid) > 0.0, "mirrored face normal should point outward from the copy");
        }
    }

    #[test]
    fn mirroring_a_hollow_ring_stays_manifold() {
        let mut doc = Document::new();
        let plane = Plane::from_normal(DVec3::ZERO, DVec3::Z);
        doc.draw_circle(&plane, DVec2::new(10.0, 0.0), 5.0, 16, None);
        doc.draw_circle(&plane, DVec2::new(10.0, 0.0), 2.0, 16, None);
        let inner_id = doc.mesh.faces.iter().find(|(_, f)| f.holes.is_empty()).unwrap().0;
        doc.erase_face(inner_id);
        let ring_id = doc.mesh.faces.iter().next().unwrap().0;
        let tube_faces = doc.push_pull(ring_id, 3.0);

        let mirrored = doc.mirror_faces(&tube_faces, MirrorAxis::X, DVec3::ZERO);

        assert!(pushpull::is_manifold(&doc.mesh, &mirrored), "mirrored hollow tube (with reversed hole loops) must stay manifold");
    }

    /// Finds the wall's own 2D bounding-box center, in the wall's own plane
    /// coordinates - used by the draw-on-face tests below to place a small
    /// rectangle safely inside an arbitrary axis-aligned box wall.
    fn wall_plane_and_center(doc: &Document, wall_id: FaceId) -> (Plane, DVec2) {
        let wall = doc.mesh.faces[wall_id].clone();
        let wall_plane = Plane::from_normal(doc.mesh.position(wall.outer[0]), wall.normal);
        let wall_2d: Vec<DVec2> = wall.outer.iter().map(|&v| wall_plane.to_2d(doc.mesh.position(v))).collect();
        let min = wall_2d.iter().cloned().reduce(DVec2::min).unwrap();
        let max = wall_2d.iter().cloned().reduce(DVec2::max).unwrap();
        (wall_plane, (min + max) * 0.5)
    }

    #[test]
    fn drawing_a_rectangle_on_a_solids_side_wall_splits_just_that_wall() {
        let mut doc = Document::new();
        let plane = Plane::from_normal(DVec3::ZERO, DVec3::Z);
        let sketch_id = doc.draw_rectangle(&plane, DVec2::ZERO, DVec2::new(10.0, 10.0), None)[0];
        let box_faces = doc.push_pull(sketch_id, 4.0);
        let group_id = doc.group_faces(&box_faces, "hull".to_string());
        let wall_id = box_faces.iter().copied().find(|&fid| doc.mesh.faces[fid].normal.z.abs() < 0.01).unwrap();

        let (wall_plane, center) = wall_plane_and_center(&doc, wall_id);
        let new_faces =
            doc.draw_rectangle(&wall_plane, center - DVec2::new(1.0, 1.0), center + DVec2::new(1.0, 1.0), Some(wall_id));

        assert_eq!(new_faces.len(), 2, "the wall should split into a framed hole + an inner panel");
        assert_eq!(doc.mesh.faces.len(), 7, "5 untouched box faces + the wall's 2 split pieces");
        let faces_with_holes = new_faces.iter().filter(|&&fid| !doc.mesh.faces[fid].holes.is_empty()).count();
        assert_eq!(faces_with_holes, 1);
        for &fid in &new_faces {
            assert!(doc.solid_boundary_face_ids().contains(&fid), "split wall pieces must stay solid-flagged");
            assert!(doc.groups[group_id].face_ids.contains(&fid), "split wall pieces must stay in the source wall's group");
        }
    }

    #[test]
    fn drawing_on_a_face_with_a_corner_nearly_matching_an_existing_vertex_does_not_corrupt_the_face() {
        // Regression test: snapping a new shape's corner onto an existing
        // vertex (endpoint/edge/guide snap) sends only raw coordinates, and
        // those round-trip through the f32 DocumentSnapshot on the way to
        // the frontend and back - so the value that arrives back at
        // `draw_polygon` is very close to, but not bit-identical to, the
        // existing vertex's true f64 position. Before `resplit_loops` welded
        // near-duplicate points, this produced two almost-coincident-but-
        // distinct points feeding face_detect's neighbor-angle sort, which
        // spliced unrelated edges together into a self-intersecting face
        // instead of a clean split.
        //
        // Uses a triangle (via `draw_polygon`), not a rectangle: two
        // axis-aligned rectangles sharing a corner in the same basis always
        // have collinear edges there, which is a related but separate
        // T-junction case `face_detect::split_edges_at_interior_points`
        // handles - see
        // `a_rectangle_sharing_a_corner_with_its_target_face_splits_it_correctly`.
        // This test isolates the welding behavior specifically, so its
        // edges must not be collinear with the target face's own
        // axis-aligned boundary.
        let mut doc = Document::new();
        let plane = Plane::from_normal(DVec3::ZERO, DVec3::Z);
        let sketch_id = doc.draw_rectangle(&plane, DVec2::ZERO, DVec2::new(10.0, 10.0), None)[0];
        let box_faces = doc.push_pull(sketch_id, 4.0);
        let top_id = box_faces.iter().copied().find(|&fid| doc.mesh.faces[fid].normal.z > 0.9).unwrap();

        let top = doc.mesh.faces[top_id].clone();
        let top_plane = Plane::from_normal(doc.mesh.position(top.outer[0]), top.normal);
        let outer_2d: Vec<DVec2> = top.outer.iter().map(|&v| top_plane.to_2d(doc.mesh.position(v))).collect();
        // A brand new VertexId at the very same position as an existing
        // corner - the near-duplicate scenario itself, not a proxy for it -
        // plus two more points chosen so neither triangle edge from it is
        // axis-aligned.
        let corner_2d = outer_2d[0];
        let triangle = vec![corner_2d, corner_2d + DVec2::new(3.0, 1.0), corner_2d + DVec2::new(1.0, 3.0)];

        let new_faces = doc.draw_polygon(&top_plane, triangle, Some(top_id));

        assert_eq!(new_faces.len(), 2, "the top face should split into a framed hole + an inner panel");
        for &fid in &new_faces {
            let face = &doc.mesh.faces[fid];
            assert!(face.outer.len() >= 3);
            for &vid in &face.outer {
                assert!(
                    (doc.mesh.position(vid).z - 4.0).abs() < 1e-6,
                    "a corrupted split would pull vertices off the top face's own plane"
                );
            }
        }
    }

    #[test]
    fn studs_on_two_adjacent_faces_snapped_to_the_same_corner_stay_watertight() {
        // Sanity check for a "staircase" build: sketching on one face,
        // snapping a corner onto an existing box corner, pushing/pulling
        // into a stud, then separately sketching on the *adjacent* face and
        // snapping another corner onto that same box corner. Each sketch is
        // its own `resplit_face_with_loops` call - `connected_component_vertices`
        // is what lets the second sketch's near-duplicate corner weld onto
        // the first face's own (unchanged) copy of that vertex rather than
        // creating an unrelated one. Note this doesn't by itself prove the
        // weld is *necessary*: each stud's own wall+cap is a self-contained
        // closed shape regardless of which vertex its base welds to, so
        // `is_manifold` alone can pass even without the connected-solid
        // weld in cases like this one. Kept as coverage for the scenario the
        // fix targets, not as a red/green proof of it.
        let mut doc = Document::new();
        let plane = Plane::from_normal(DVec3::ZERO, DVec3::Z);
        let sketch_id = doc.draw_rectangle(&plane, DVec2::ZERO, DVec2::new(10.0, 10.0), None)[0];
        let box_faces = doc.push_pull(sketch_id, 4.0);
        let top_id = box_faces.iter().copied().find(|&fid| doc.mesh.faces[fid].normal.z > 0.9).unwrap();
        let wall_id = box_faces
            .iter()
            .copied()
            .find(|&fid| doc.mesh.faces[fid].normal.z.abs() < 0.01)
            .unwrap();

        let top = doc.mesh.faces[top_id].clone();
        let wall = doc.mesh.faces[wall_id].clone();
        let wall_vertices: HashSet<VertexId> = wall.outer.iter().copied().collect();
        let shared = *top.outer.iter().find(|v| wall_vertices.contains(v)).expect("top and wall share an edge");
        let corner = doc.mesh.position(shared);

        // Two different tiny nudges, both well under Mesh::WELD_TOLERANCE -
        // standing in for two *separate* f32-DocumentSnapshot round trips
        // (each sketch's corner snap is its own frontend round trip, so
        // there's no reason they'd land on the exact same rounding error).
        let near_1 = corner + DVec3::new(2e-5, -3e-5, 1e-5);
        let near_2 = corner + DVec3::new(-1e-5, 2e-5, -2e-5);

        // A small, non-axis-aligned triangle anchored at `corner_2d`, built
        // generically from the target face's own outline so it works for
        // any convex face regardless of size or orientation - the corner-to-
        // centroid direction is used (not a rectangle's own axes) to isolate
        // the cross-face weld from the separate T-junction case
        // `a_rectangle_sharing_a_corner_with_its_target_face_splits_it_correctly`
        // covers.
        fn stud_triangle(mesh: &Mesh, plane: &Plane, outer: &[VertexId], corner_3d: DVec3) -> Vec<DVec2> {
            let outer_2d: Vec<DVec2> = outer.iter().map(|&v| plane.to_2d(mesh.position(v))).collect();
            let centroid = outer_2d.iter().fold(DVec2::ZERO, |acc, &p| acc + p) / outer_2d.len() as f64;
            let corner_2d = plane.to_2d(corner_3d);
            let nearest = (0..outer_2d.len())
                .min_by(|&a, &b| {
                    (outer_2d[a] - corner_2d).length_squared().partial_cmp(&(outer_2d[b] - corner_2d).length_squared()).unwrap()
                })
                .unwrap();
            let adjacent = outer_2d[(nearest + 1) % outer_2d.len()];
            // Convex combinations of corner/centroid/adjacent-vertex stay
            // inside any convex polygon regardless of aspect ratio - unlike
            // a perpendicular offset sized off the corner-to-centroid
            // vector, which overshoots a short dimension on an elongated
            // (e.g. wide, short) face.
            let p1 = corner_2d * 0.7 + centroid * 0.3;
            let p2 = corner_2d * 0.7 + adjacent * 0.15 + centroid * 0.15;
            vec![corner_2d, p1, p2]
        }

        let top_plane = Plane::from_normal(doc.mesh.position(top.outer[0]), top.normal);
        let stud_a = doc.draw_polygon(&top_plane, stud_triangle(&doc.mesh, &top_plane, &top.outer, near_1), Some(top_id));
        assert_eq!(stud_a.len(), 2, "top face should split into a framed hole + inner panel");
        let stud_a_cap = stud_a
            .iter()
            .copied()
            .find(|&fid| doc.mesh.faces[fid].holes.is_empty())
            .expect("the inner panel (no holes) is the new stud's cap");
        doc.push_pull_faces(&[stud_a_cap], 1.0);

        let wall_plane = Plane::from_normal(doc.mesh.position(wall.outer[0]), wall.normal);
        let stud_b =
            doc.draw_polygon(&wall_plane, stud_triangle(&doc.mesh, &wall_plane, &wall.outer, near_2), Some(wall_id));
        assert_eq!(stud_b.len(), 2, "wall should split into a framed hole + inner panel");
        let stud_b_cap = stud_b
            .iter()
            .copied()
            .find(|&fid| doc.mesh.faces[fid].holes.is_empty())
            .expect("the inner panel (no holes) is the new stud's cap");
        doc.push_pull_faces(&[stud_b_cap], 1.0);

        let solids = doc.solid_boundary_face_ids();
        assert!(pushpull::is_manifold(&doc.mesh, &solids), "two studs snapped to the same corner must stay watertight");
    }

    #[test]
    fn a_rectangle_sharing_a_corner_with_its_target_face_splits_it_correctly() {
        // Two axis-aligned rectangles sharing a corner - the "stud built
        // into the corner of a box's top face" workflow, and the most basic
        // way to snap a new sketch onto existing geometry (an endpoint,
        // edge, or measure-tool guide snap all reach this shape
        // identically). Always produces two pairs of exactly-collinear
        // edges at the shared corner, which used to tie `face_detect`'s
        // neighbor-angle sort with no tiebreaker and force a safe-but-wrong
        // no-op rejection of an extremely common, valid draw. Fixed by
        // `face_detect::split_edges_at_interior_points`, which splits the
        // target face's long edges at the new rectangle's T-junction points
        // before tracing, removing the tie instead of merely detecting and
        // rejecting it.
        let mut doc = Document::new();
        let plane = Plane::from_normal(DVec3::ZERO, DVec3::Z);
        let sketch_id = doc.draw_rectangle(&plane, DVec2::ZERO, DVec2::new(10.0, 10.0), None)[0];
        let box_faces = doc.push_pull(sketch_id, 4.0);
        let top_id = box_faces.iter().copied().find(|&fid| doc.mesh.faces[fid].normal.z > 0.9).unwrap();

        let top = doc.mesh.faces[top_id].clone();
        let top_plane = Plane::from_normal(doc.mesh.position(top.outer[0]), top.normal);
        let corner_2d = top_plane.to_2d(doc.mesh.position(top.outer[0]));

        let new_faces = doc.draw_rectangle(&top_plane, corner_2d, corner_2d + DVec2::new(5.0, 5.0), Some(top_id));

        assert_eq!(new_faces.len(), 2, "the corner-stud footprint and the remaining L-shaped area");
        assert!(!doc.mesh.faces.contains_key(top_id), "the original top face is replaced by the split");
        for &fid in &new_faces {
            for &vid in &doc.mesh.faces[fid].outer {
                assert!(
                    (doc.mesh.position(vid).z - 4.0).abs() < 1e-6,
                    "a corrupted split would pull vertices off the top face's own plane"
                );
            }
        }
        let report = doc.check_model();
        assert_eq!(report.broken_part_count, 0, "the split must stay watertight");
    }

    #[test]
    fn a_t_junction_on_an_edge_shared_between_two_flat_sketches_survives_extrusion() {
        // `propagate_boundary_split_to_solid_siblings` early-returns for a
        // `face_id` that isn't on a solid's boundary. Two *flat* sketches
        // left adjacent by an earlier `resplit_plane` do share an edge, so
        // splitting one of them at a T-junction leaves the other's copy of
        // that edge unsplit - and nothing propagates it. This pins what that
        // actually costs once both are extruded into real solids, which is
        // the point at which a stale shared edge would matter.
        let mut doc = Document::new();
        let ground = Plane::from_normal(DVec3::ZERO, DVec3::Z);
        doc.draw_rectangle(&ground, DVec2::ZERO, DVec2::new(20.0, 20.0), None);
        // Splits the sheet into a 10x10 square and a concave L-shape, both
        // flat sketches sharing the edges at x=10 and y=10.
        doc.draw_rectangle(&ground, DVec2::ZERO, DVec2::new(10.0, 10.0), None);

        let flat_face = |doc: &Document, want_len: usize| -> FaceId {
            doc.mesh
                .faces
                .iter()
                .find(|(id, f)| f.outer.len() == want_len && !doc.solid_face_ids.contains(id))
                .map(|(id, _)| id)
                .expect("a flat sketch face with that many corners")
        };
        let l_shape = flat_face(&doc, 6);

        // (10, 5) sits on the L's own edge - and on the square's copy of it.
        let split = doc.draw_rectangle(&ground, DVec2::new(10.0, 5.0), DVec2::new(15.0, 2.0), Some(l_shape));
        assert!(!split.is_empty(), "a T-junction sketch on a flat face must still be accepted");

        let square = flat_face(&doc, 4);
        doc.push_pull(square, 3.0);
        let report = doc.check_model();
        assert_eq!(
            report.broken_part_count, 0,
            "extruding the un-propagated neighbor must still produce a watertight solid: {} duplicate, {} open",
            report.duplicate_edges.len(),
            report.open_edges.len()
        );
    }

    #[test]
    fn a_second_corner_stud_on_the_resulting_concave_face_stays_watertight() {
        // Chained splits: after the first corner stud, the remaining top
        // face is a concave L-shape, and the second corner stud's
        // T-junctions land on *its* edges - so this exercises the split,
        // the sibling propagation and the collinear-aware triangulation
        // against a non-convex target face, and against a face that is
        // itself the product of an earlier split rather than a pristine
        // rectangle.
        let mut doc = Document::new();
        let ground = Plane::from_normal(DVec3::ZERO, DVec3::Z);
        let sketch_id = doc.draw_rectangle(&ground, DVec2::ZERO, DVec2::new(20.0, 20.0), None)[0];
        doc.push_pull(sketch_id, 4.0);
        let top_plane = Plane::from_normal(DVec3::new(0.0, 0.0, 4.0), DVec3::Z);
        let cap_at_z4 = |doc: &Document| -> FaceId {
            doc.mesh
                .faces
                .iter()
                .find(|(_, f)| f.normal.z > 0.9 && (doc.mesh.position(f.outer[0]).z - 4.0).abs() < 1e-9)
                .map(|(id, _)| id)
                .expect("a top face at z=4")
        };

        let split_1 = doc.draw_rectangle(&top_plane, DVec2::ZERO, DVec2::new(8.0, 8.0), Some(cap_at_z4(&doc)));
        assert_eq!(split_1.len(), 2, "corner stud footprint + concave remainder");
        let stud_1 = split_1.iter().copied().find(|&f| doc.mesh.faces[f].outer.len() == 4).unwrap();
        doc.push_pull(stud_1, 3.0);
        assert_eq!(doc.check_model().broken_part_count, 0, "precondition: the first corner stud is watertight");

        // The remaining face is now an L-shape. Put the second stud in the
        // diagonally opposite corner, so both its T-junctions land on the
        // L's own (post-split) edges.
        let l_face = cap_at_z4(&doc);
        assert_eq!(doc.mesh.faces[l_face].outer.len(), 6, "the remainder really is the concave L-shape");
        let split_2 =
            doc.draw_rectangle(&top_plane, DVec2::new(12.0, 12.0), DVec2::new(20.0, 20.0), Some(l_face));
        assert_eq!(split_2.len(), 2, "the concave face must split, not be rejected");
        let stud_2 = split_2.iter().copied().find(|&f| doc.mesh.faces[f].outer.len() == 4).unwrap();
        doc.push_pull(stud_2, 5.0);

        let report = doc.check_model();
        assert_eq!(
            report.broken_part_count, 0,
            "chained corner studs must stay watertight: {} duplicate, {} open",
            report.duplicate_edges.len(),
            report.open_edges.len()
        );
    }

    #[test]
    fn a_sketch_touching_an_existing_studs_rim_does_not_refill_that_rim() {
        // A T-junction landing on one of the target face's *hole* edges (not
        // its outer boundary): draw a stud on a box top, then draw a second
        // sketch on the remaining top face with a corner snapped onto the
        // middle of the first stud's rim - exactly the alignment a
        // measure-tool guide invites.
        //
        // `split_edges_at_interior_points` splits that rim edge, so the
        // re-detected loop for the hole region comes back with one MORE
        // vertex than the `protected_holes` entry recorded before the split.
        // `matches_any_loop` compares by exact length, so the protection
        // silently stops matching and the hole gets refilled with a face -
        // duplicating every edge the stud's wall already pairs with.
        let mut doc = Document::new();
        let ground = Plane::from_normal(DVec3::ZERO, DVec3::Z);
        let sketch_id = doc.draw_rectangle(&ground, DVec2::ZERO, DVec2::new(20.0, 20.0), None)[0];
        doc.push_pull(sketch_id, 4.0);

        let top_plane = Plane::from_normal(DVec3::new(0.0, 0.0, 4.0), DVec3::Z);
        let cap_id = |doc: &Document| -> FaceId {
            doc.mesh
                .faces
                .iter()
                .find(|(_, f)| f.normal.z > 0.9 && (doc.mesh.position(f.outer[0]).z - 4.0).abs() < 1e-9)
                .map(|(id, _)| id)
                .expect("the box's top cap")
        };

        // A rectangular stud in the middle of the top face, leaving the cap
        // with a 4-vertex hole at its rim.
        let split = doc.draw_rectangle(&top_plane, DVec2::new(5.0, 5.0), DVec2::new(15.0, 15.0), Some(cap_id(&doc)));
        let stud_cap = split.iter().copied().find(|&fid| doc.mesh.faces[fid].holes.is_empty()).unwrap();
        doc.push_pull(stud_cap, 3.0);
        assert_eq!(doc.check_model().broken_part_count, 0, "precondition: one stud on a box is watertight");

        // (10, 5) sits exactly halfway along the rim edge (5,5)->(15,5).
        // The rest of the rectangle stays clear of the hole (y <= 5).
        doc.draw_rectangle(&top_plane, DVec2::new(10.0, 5.0), DVec2::new(18.0, 2.0), Some(cap_id(&doc)));

        let report = doc.check_model();
        assert_eq!(
            report.broken_part_count, 0,
            "touching a stud's rim must not refill it: {} duplicate edge(s), {} open edge(s)",
            report.duplicate_edges.len(),
            report.open_edges.len()
        );

        // The cap now carries a *merged* hole (the rim fused with the new
        // sketch's footprint, see `face_detect::merge_holes_sharing_an_edge`).
        // Resplitting it again has to keep protecting that fused loop: it is
        // fed back in as one ring, so it re-traces as one region and
        // `matches_any_loop` still recognizes it. A third sketch elsewhere on
        // the cap is what proves the merge didn't break hole protection.
        doc.draw_rectangle(&top_plane, DVec2::new(2.0, 16.0), DVec2::new(8.0, 19.0), Some(cap_id(&doc)));
        let report = doc.check_model();
        assert_eq!(
            report.broken_part_count, 0,
            "a later sketch must not refill the merged hole: {} duplicate edge(s), {} open edge(s)",
            report.duplicate_edges.len(),
            report.open_edges.len()
        );
    }

    #[test]
    fn pushing_a_corner_stud_sketch_into_a_real_stud_stays_watertight() {
        // The actual end-to-end workflow the previous test's flat split is
        // a building block for: sketch a rectangle flush into a box top's
        // corner, then push/pull the resulting inner panel up into a real
        // 3D stud. Exercises the T-junction split
        // (`face_detect::split_edges_at_interior_points`) and the sibling
        // propagation (`Document::propagate_boundary_split_to_solid_siblings`)
        // together with a second level of extrusion on top, which is what
        // actually stresses the wall's now-pentagonal boundary
        // (`triangulate::on_segment_interior` is what keeps that pentagon's
        // own triangulation from silently skipping the T-junction vertex).
        let mut doc = Document::new();
        let plane = Plane::from_normal(DVec3::ZERO, DVec3::Z);
        let sketch_id = doc.draw_rectangle(&plane, DVec2::ZERO, DVec2::new(10.0, 10.0), None)[0];
        let box_faces = doc.push_pull(sketch_id, 4.0);
        let top_id = box_faces.iter().copied().find(|&fid| doc.mesh.faces[fid].normal.z > 0.9).unwrap();
        let top = doc.mesh.faces[top_id].clone();
        let top_plane = Plane::from_normal(doc.mesh.position(top.outer[0]), top.normal);
        let corner_2d = top_plane.to_2d(doc.mesh.position(top.outer[0]));

        let new_faces = doc.draw_rectangle(&top_plane, corner_2d, corner_2d + DVec2::new(5.0, 5.0), Some(top_id));
        let stud_cap = new_faces
            .iter()
            .copied()
            .find(|&fid| doc.mesh.faces[fid].outer.len() == 4)
            .expect("the corner-stud footprint (no holes, 4 corners)");
        doc.push_pull(stud_cap, 3.0);

        let report = doc.check_model();
        assert_eq!(report.broken_part_count, 0, "a stud pushed up from a T-junction split must stay watertight");
    }

    #[test]
    fn resplitting_a_face_with_a_loop_that_lands_on_the_wrong_neighboring_face_is_rejected() {
        // Reproduces a real bug: the frontend picks which face a click
        // "landed on" with a single raycast, which is ambiguous exactly on
        // a vertex/edge shared between two faces - precisely where a
        // snapped corner (an existing vertex, edge midpoint, or
        // measure-tool guide) is most likely to land. When that raycast
        // resolves to the wrong neighboring face, the rest of the shape -
        // sized from screen positions the user intended for a completely
        // different plane - ends up mostly or entirely outside the
        // (wrongly) resolved target face's own boundary. Before the
        // face-fit check, `resplit_face_with_loops` fed `face_detect` a
        // loop whose edges crossed the target face's boundary instead of
        // merely touching it, producing a self-intersecting, warped face
        // (a large sheared "wing") in place of the target face - the
        // "faces disappear" corruption a user reported after drawing on a
        // face using a measure-tool guide landing on a shared corner.
        let mut doc = Document::new();
        let plane = Plane::from_normal(DVec3::ZERO, DVec3::Z);
        let sketch_id = doc.draw_rectangle(&plane, DVec2::ZERO, DVec2::new(10.0, 10.0), None)[0];
        let box_faces = doc.push_pull(sketch_id, 4.0);
        let left_id = box_faces
            .iter()
            .copied()
            .find(|&fid| doc.mesh.faces[fid].normal.z.abs() < 0.01 && doc.mesh.faces[fid].normal.x.abs() > 0.9)
            .unwrap();
        let faces_before = doc.mesh.faces.len();

        // A loop shaped for the ground plane (spanning far beyond the LEFT
        // face's own footprint) mistakenly resplit against the LEFT face -
        // exactly what an ambiguous raycast at their shared corner produces.
        let wrong_loop = doc.draw_rectangle(&plane, DVec2::new(0.0, 0.0), DVec2::new(20.0, 20.0), None);
        let wrong_loop_vertices = doc.mesh.faces[wrong_loop[0]].outer.clone();
        doc.mesh.remove_face(wrong_loop[0]);

        let new_faces = doc.resplit_face_with_loops(left_id, vec![wrong_loop_vertices]);

        assert!(new_faces.is_empty(), "a loop mostly outside the target face must be rejected, not corrupt it");
        assert_eq!(doc.mesh.faces.len(), faces_before, "the document must be left exactly as it was");
    }

    #[test]
    fn pushing_a_face_drawn_on_a_solid_wall_stays_manifold() {
        // The porthole workflow end-to-end: sketch a rectangle on a wall,
        // then push it inward to carve a recess.
        let mut doc = Document::new();
        let plane = Plane::from_normal(DVec3::ZERO, DVec3::Z);
        let sketch_id = doc.draw_rectangle(&plane, DVec2::ZERO, DVec2::new(10.0, 10.0), None)[0];
        let box_faces = doc.push_pull(sketch_id, 4.0);
        let wall_id = box_faces.iter().copied().find(|&fid| doc.mesh.faces[fid].normal.z.abs() < 0.01).unwrap();

        let (wall_plane, center) = wall_plane_and_center(&doc, wall_id);
        let split = doc.draw_rectangle(&wall_plane, center - DVec2::new(1.0, 1.0), center + DVec2::new(1.0, 1.0), Some(wall_id));
        let inner = *split.iter().find(|&fid| doc.mesh.faces[*fid].holes.is_empty()).unwrap();
        doc.push_pull(inner, -0.5);

        let all_faces: Vec<FaceId> = doc.mesh.faces.keys().collect();
        assert!(pushpull::is_manifold(&doc.mesh, &all_faces), "porthole recess must keep the whole solid watertight");
    }

    #[test]
    fn drawing_on_a_target_face_does_not_disturb_other_coplanar_sketches() {
        // Two independent flat sketches on the ground plane (neither pushed
        // into a solid, so resplit_plane's solid_face_ids exclusion doesn't
        // apply to either) - confirms target_face_id routes through the
        // LOCAL resplit_face_with_loops path, not the document-wide
        // coplanar search resplit_plane does, which would otherwise merge
        // both rectangles the moment either one is touched.
        let mut doc = Document::new();
        let plane = Plane::from_normal(DVec3::ZERO, DVec3::Z);
        doc.draw_rectangle(&plane, DVec2::ZERO, DVec2::new(10.0, 10.0), None);
        // Drawing this second, far-away sketch resplits the WHOLE ground
        // plane (both are plain sketches, so neither is solid_face_ids-
        // excluded) - which erases and recreates the first rectangle's
        // face, invalidating any id captured before this point. Look the
        // target up fresh afterward instead - the same rule
        // `Document::push_pull`'s stale-id tests already rely on.
        doc.draw_rectangle(&plane, DVec2::new(20.0, 20.0), DVec2::new(21.0, 21.0), None);
        assert_eq!(doc.mesh.faces.len(), 2);
        let target_id = doc
            .mesh
            .faces
            .iter()
            .find(|(_, f)| doc.mesh.position(f.outer[0]).x < 15.0)
            .map(|(id, _)| id)
            .unwrap();

        let split = doc.draw_rectangle(&plane, DVec2::new(4.0, 4.0), DVec2::new(6.0, 6.0), Some(target_id));

        assert_eq!(split.len(), 2, "the targeted sketch should split into a framed hole + an inner panel");
        assert_eq!(doc.mesh.faces.len(), 3, "the untouched second sketch must survive alongside the 2 split pieces");
        let untouched = doc.mesh.faces.values().any(|f| {
            f.outer.len() == 4 && f.holes.is_empty() && doc.mesh.position(f.outer[0]).x > 15.0
        });
        assert!(untouched, "the unrelated second sketch rectangle must be unchanged");
    }

    #[test]
    fn drawing_with_a_stale_target_face_falls_back_to_the_plain_resplit() {
        let mut doc = Document::new();
        let plane = Plane::from_normal(DVec3::ZERO, DVec3::Z);
        let stale_id = doc.draw_rectangle(&plane, DVec2::ZERO, DVec2::new(1.0, 1.0), None)[0];
        doc.erase_face(stale_id);

        let new_faces = doc.draw_rectangle(&plane, DVec2::new(2.0, 2.0), DVec2::new(3.0, 3.0), Some(stale_id));

        assert_eq!(new_faces.len(), 1, "a stale target must not drop the drawn shape - falls back to the plain ground-plane resplit");
        assert_eq!(doc.mesh.faces[new_faces[0]].outer.len(), 4);
    }

    #[test]
    fn project_file_round_trip_preserves_geometry_and_groups() {
        let mut doc = Document::new();
        let plane = Plane::from_normal(DVec3::ZERO, DVec3::Z);
        let face_id = doc.draw_rectangle(&plane, DVec2::ZERO, DVec2::new(2.0, 3.0), None)[0];
        let solid_faces = doc.push_pull(face_id, 4.0);
        let group_id = doc.group_faces(&solid_faces, "hull".to_string());

        let project = doc.to_project_file();
        let mut reloaded = Document::from_project_file(&project);

        assert_eq!(reloaded.mesh.faces.len(), doc.mesh.faces.len());
        assert_eq!(reloaded.mesh.vertices.len(), doc.mesh.vertices.len());
        assert_eq!(reloaded.groups.len(), 1);
        let reloaded_group = reloaded.groups.values().next().unwrap();
        assert_eq!(reloaded_group.name, "hull");
        assert_eq!(reloaded_group.face_ids.len(), doc.groups[group_id].face_ids.len());

        // Every reloaded face was created via push_pull, so it must still be
        // excluded from resplit_plane's coplanar search - drawing a new,
        // unrelated shape on the ground plane must not sweep in the
        // reloaded solid's cap even though it's exactly coplanar with it.
        let face_count_before = reloaded.mesh.faces.len();
        reloaded.draw_rectangle(&plane, DVec2::new(10.0, 10.0), DVec2::new(11.0, 11.0), None);
        assert_eq!(reloaded.mesh.faces.len(), face_count_before + 1, "reloaded solid's faces must be untouched");
    }

    #[test]
    fn loading_a_project_file_with_out_of_range_indices_is_not_a_panic() {
        // A hand-edited, truncated, or hostile .json must not panic: a panic
        // inside a `#[tauri::command]` happens while the AppState mutex is
        // held, which poisons it and bricks every later command (they all
        // `lock().unwrap()`). Out-of-range references are dropped instead.
        let project = ProjectFile {
            vertices: vec![[0.0, 0.0, 0.0], [1.0, 0.0, 0.0], [1.0, 1.0, 0.0]],
            faces: vec![
                ProjectFace { outer: vec![0, 1, 2], holes: vec![], solid: false },
                ProjectFace { outer: vec![0, 1, 999], holes: vec![], solid: false },
                ProjectFace { outer: vec![0, 1, 2], holes: vec![vec![7]], solid: false },
            ],
            groups: vec![ProjectGroup { name: "g".to_string(), face_indices: vec![0, 42] }],
            guides: vec![],
        };

        let doc = Document::from_project_file(&project);

        // The face with the dangling *outer* index is dropped entirely; the
        // one whose only problem is a dangling hole keeps its valid outer
        // loop and loses just the hole.
        assert_eq!(doc.mesh.faces.len(), 2);
        assert!(doc.mesh.faces.values().all(|f| f.holes.is_empty()), "the unresolvable hole loop should be dropped");
        assert_eq!(doc.groups.len(), 1);
        assert_eq!(doc.groups.values().next().unwrap().face_ids.len(), 1, "the dangling face index should be dropped");
    }

    #[test]
    fn loading_a_project_file_with_degenerate_faces_drops_them() {
        // A loop with fewer than 3 vertices has no meaningful normal and
        // would flow into triangulation as a zero-area face.
        let project = ProjectFile {
            vertices: vec![[0.0, 0.0, 0.0], [1.0, 0.0, 0.0], [1.0, 1.0, 0.0]],
            faces: vec![
                ProjectFace { outer: vec![0, 1], holes: vec![], solid: false },
                ProjectFace { outer: vec![], holes: vec![], solid: false },
                ProjectFace { outer: vec![0, 1, 2], holes: vec![], solid: false },
            ],
            groups: vec![],
            guides: vec![],
        };

        let doc = Document::from_project_file(&project);

        assert_eq!(doc.mesh.faces.len(), 1, "the 2-vertex and empty loops must be dropped");
    }

    #[test]
    fn project_file_round_trip_preserves_vertex_positions() {
        let mut doc = Document::new();
        let plane = Plane::from_normal(DVec3::ZERO, DVec3::Z);
        doc.draw_rectangle(&plane, DVec2::new(1.5, -2.5), DVec2::new(4.0, 3.0), None);

        let project = doc.to_project_file();
        let reloaded = Document::from_project_file(&project);

        let original_face = doc.mesh.faces.values().next().unwrap();
        let reloaded_face = reloaded.mesh.faces.values().next().unwrap();
        let mut original_positions: Vec<DVec3> = original_face.outer.iter().map(|&v| doc.mesh.position(v)).collect();
        let mut reloaded_positions: Vec<DVec3> =
            reloaded_face.outer.iter().map(|&v| reloaded.mesh.position(v)).collect();
        original_positions.sort_by(|a, b| a.x.partial_cmp(&b.x).unwrap().then(a.y.partial_cmp(&b.y).unwrap()));
        reloaded_positions.sort_by(|a, b| a.x.partial_cmp(&b.x).unwrap().then(a.y.partial_cmp(&b.y).unwrap()));
        for (a, b) in original_positions.iter().zip(reloaded_positions.iter()) {
            assert!((*a - *b).length() < 1e-9);
        }
    }

    #[test]
    fn guides_round_trip_through_the_project_file() {
        let mut doc = Document::new();
        doc.add_guide(DVec3::new(1.0, 2.0, 3.0), DVec3::new(4.0, 5.0, 6.0));
        doc.add_guide(DVec3::new(-1.0, 0.0, 2.5), DVec3::new(7.0, -3.0, 0.0));

        let project = doc.to_project_file();
        let reloaded = Document::from_project_file(&project);

        assert_eq!(reloaded.guides.len(), 2);
        assert_eq!(reloaded.guides[0].a, DVec3::new(1.0, 2.0, 3.0));
        assert_eq!(reloaded.guides[0].b, DVec3::new(4.0, 5.0, 6.0));
        assert_eq!(reloaded.guides[1].a, DVec3::new(-1.0, 0.0, 2.5));
        assert_eq!(reloaded.guides[1].b, DVec3::new(7.0, -3.0, 0.0));
    }

    #[test]
    fn a_project_file_written_before_guides_existed_still_loads() {
        // No `guides` field at all - exactly what every project.json saved
        // before this feature existed looks like. Must go through serde
        // (not a struct literal, which always has the field) to actually
        // exercise `#[serde(default)]`.
        let project: ProjectFile =
            serde_json::from_str(r#"{"vertices":[],"faces":[],"groups":[]}"#).unwrap();
        assert!(project.guides.is_empty());
        assert_eq!(Document::from_project_file(&project).guides.len(), 0);
    }

    #[test]
    fn guides_are_not_moved_by_geometry_transforms() {
        let mut doc = Document::new();
        let plane = Plane::from_normal(DVec3::ZERO, DVec3::Z);
        let face_id = doc.draw_rectangle(&plane, DVec2::ZERO, DVec2::new(2.0, 2.0), None)[0];
        doc.add_guide(DVec3::new(0.0, 0.0, 0.0), DVec3::new(1.0, 1.0, 0.0));

        doc.translate_faces(&[face_id], DVec3::new(0.0, 0.0, 10.0));

        assert_eq!(doc.guides.len(), 1);
        assert_eq!(doc.guides[0].a, DVec3::new(0.0, 0.0, 0.0));
        assert_eq!(doc.guides[0].b, DVec3::new(1.0, 1.0, 0.0));
    }

    /// Draws a `size` x `size` box on the plane z = `z0` and pushes/pulls it
    /// by `height` (may be negative), for `arrange_for_print` tests below.
    fn make_box(doc: &mut Document, origin_xy: DVec2, size: f64, z0: f64, height: f64) -> Vec<FaceId> {
        let plane = Plane::from_normal(DVec3::new(0.0, 0.0, z0), DVec3::Z);
        let sketch_id = doc.draw_rectangle(&plane, origin_xy, origin_xy + DVec2::new(size, size), None)[0];
        doc.push_pull(sketch_id, height)
    }

    fn xy_overlaps(a: (DVec3, DVec3), b: (DVec3, DVec3)) -> bool {
        a.0.x < b.1.x && a.1.x > b.0.x && a.0.y < b.1.y && a.1.y > b.0.y
    }

    #[test]
    fn arrange_for_print_floors_and_separates_two_overlapping_boxes() {
        let mut doc = Document::new();
        // Same XY footprint (fully overlapping), one raised above the
        // ground, one with its cap left below z=0 - both must end up
        // floor-aligned and side by side, not overlapping.
        make_box(&mut doc, DVec2::new(0.0, 0.0), 2.0, 5.0, 2.0); // z in [5, 7]
        make_box(&mut doc, DVec2::new(0.0, 0.0), 2.0, 0.0, -3.0); // z in [-3, 0]

        doc.arrange_for_print();

        let solids = doc.solid_boundary_face_ids();
        let components = doc.mesh.connected_components(&solids);
        assert_eq!(components.len(), 2);

        let boxes: Vec<(DVec3, DVec3)> = components.iter().map(|c| doc.mesh.bounding_box(c)).collect();
        for (min, _) in &boxes {
            assert!(min.z.abs() < 1e-9, "part must be floor-aligned, got min.z = {}", min.z);
        }
        assert!(!xy_overlaps(boxes[0], boxes[1]), "arranged parts must not overlap in XY: {boxes:?}");
    }

    #[test]
    fn arrange_for_print_has_no_pairwise_overlap_across_varied_part_sizes() {
        let mut doc = Document::new();
        // All at the same overlapping XY origin, deliberately different
        // sizes - exercises the uniform-grid-cell overlap guarantee, not
        // just a coincidence of two equal-sized boxes.
        make_box(&mut doc, DVec2::new(0.0, 0.0), 1.0, 0.0, 1.0);
        make_box(&mut doc, DVec2::new(0.0, 0.0), 8.0, 0.0, 1.0);
        make_box(&mut doc, DVec2::new(0.0, 0.0), 3.0, 0.0, 1.0);
        make_box(&mut doc, DVec2::new(0.0, 0.0), 1.5, 0.0, 1.0);

        doc.arrange_for_print();

        let solids = doc.solid_boundary_face_ids();
        let components = doc.mesh.connected_components(&solids);
        assert_eq!(components.len(), 4);
        let boxes: Vec<(DVec3, DVec3)> = components.iter().map(|c| doc.mesh.bounding_box(c)).collect();

        for i in 0..boxes.len() {
            for j in (i + 1)..boxes.len() {
                assert!(!xy_overlaps(boxes[i], boxes[j]), "parts {i} and {j} overlap: {:?} vs {:?}", boxes[i], boxes[j]);
            }
        }
    }

    #[test]
    fn arrange_for_print_is_a_noop_with_no_printable_solids() {
        let mut doc = Document::new();
        let plane = Plane::from_normal(DVec3::new(0.0, 0.0, 5.0), DVec3::Z);
        doc.draw_rectangle(&plane, DVec2::ZERO, DVec2::new(1.0, 1.0), None);
        let positions_before: Vec<DVec3> = doc.mesh.vertices.values().map(|v| v.position).collect();

        doc.arrange_for_print();

        let positions_after: Vec<DVec3> = doc.mesh.vertices.values().map(|v| v.position).collect();
        assert_eq!(positions_before, positions_after, "a flat, un-extruded sketch must be left untouched");
    }

    #[test]
    fn drop_to_plate_floors_a_single_object_above_the_plate() {
        let mut doc = Document::new();
        let faces = make_box(&mut doc, DVec2::new(0.0, 0.0), 2.0, 5.0, 2.0); // z in [5, 7]

        doc.drop_to_plate(&faces);

        let (min, _) = doc.mesh.bounding_box(&faces);
        assert!(min.z.abs() < 1e-9, "part must be floor-aligned, got min.z = {}", min.z);
    }

    #[test]
    fn drop_to_plate_floors_a_single_object_below_the_plate() {
        let mut doc = Document::new();
        let faces = make_box(&mut doc, DVec2::new(0.0, 0.0), 2.0, 0.0, -3.0); // z in [-3, 0]

        doc.drop_to_plate(&faces);

        let (min, _) = doc.mesh.bounding_box(&faces);
        assert!(min.z.abs() < 1e-9, "part must be floor-aligned, got min.z = {}", min.z);
    }

    #[test]
    fn drop_to_plate_moves_each_disconnected_selected_object_independently() {
        let mut doc = Document::new();
        // Two separate boxes at different heights and different XY origins,
        // both selected together - each must land at its own min-Z = 0
        // without picking up the other's delta (no rigid-group move) and
        // without its X/Y position changing.
        let box_a = make_box(&mut doc, DVec2::new(0.0, 0.0), 2.0, 5.0, 2.0); // z in [5, 7]
        let box_b = make_box(&mut doc, DVec2::new(10.0, 10.0), 2.0, -4.0, 1.0); // z in [-4, -3]

        let (a_min_before, _) = doc.mesh.bounding_box(&box_a);
        let (b_min_before, _) = doc.mesh.bounding_box(&box_b);

        let mut selection = box_a.clone();
        selection.extend(box_b.clone());
        doc.drop_to_plate(&selection);

        let (a_min_after, _) = doc.mesh.bounding_box(&box_a);
        let (b_min_after, _) = doc.mesh.bounding_box(&box_b);
        assert!(a_min_after.z.abs() < 1e-9, "box a must be floor-aligned, got min.z = {}", a_min_after.z);
        assert!(b_min_after.z.abs() < 1e-9, "box b must be floor-aligned, got min.z = {}", b_min_after.z);
        assert!((a_min_after.x - a_min_before.x).abs() < 1e-9, "box a's X must be untouched");
        assert!((a_min_after.y - a_min_before.y).abs() < 1e-9, "box a's Y must be untouched");
        assert!((b_min_after.x - b_min_before.x).abs() < 1e-9, "box b's X must be untouched");
        assert!((b_min_after.y - b_min_before.y).abs() < 1e-9, "box b's Y must be untouched");
    }

    #[test]
    fn drop_to_plate_moves_a_partial_face_subset_via_shared_vertices() {
        let mut doc = Document::new();
        let faces = make_box(&mut doc, DVec2::new(0.0, 0.0), 2.0, 5.0, 2.0); // z in [5, 7]

        // Only pass a subset of the solid's faces - `translate_faces`
        // resolves through shared vertices, so the whole solid should still
        // move together as long as the passed faces reference every vertex
        // that needs to move (here, just dropping the whole face set works
        // the same as the full solid since connected_components merges them
        // by shared vertex into one component regardless of subset size).
        let subset = &faces[..faces.len() - 1];

        doc.drop_to_plate(subset);

        let (min, _) = doc.mesh.bounding_box(&faces);
        assert!(min.z.abs() < 1e-9, "solid must be floor-aligned via shared vertices, got min.z = {}", min.z);
    }
}
