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

        let mut loops: Vec<Vec<VertexId>> = coplanar_face_ids
            .iter()
            .flat_map(|&fid| {
                let face = &self.mesh.faces[fid];
                std::iter::once(face.outer.clone()).chain(face.holes.iter().cloned())
            })
            .collect();
        loops.push(new_loop);

        for fid in &coplanar_face_ids {
            self.erase_face(*fid);
        }

        self.resplit_loops(plane, loops)
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

        let mut loops = vec![face.outer.clone()];
        loops.extend(face.holes.iter().cloned());
        loops.extend(extra_loops);

        let was_grouped = self.face_to_group.get(&face_id).copied();
        let was_solid = self.solid_face_ids.contains(&face_id);
        self.erase_face(face_id);

        let new_faces = self.resplit_loops(&plane, loops);
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

    /// Builds the combined 2D edge graph for `loops` (each a closed ring of
    /// mesh vertices, already known to be coplanar with `plane`), re-detects
    /// faces/holes over it, and creates the resulting faces. Shared by
    /// `resplit_plane` (sticky-geometry auto-split while drawing) and
    /// `inset_face` (splitting one face into an inner face + offset frame).
    fn resplit_loops(&mut self, plane: &Plane, loops: Vec<Vec<VertexId>>) -> Vec<FaceId> {
        let mut index_of: HashMap<VertexId, usize> = HashMap::new();
        let mut points: Vec<DVec2> = Vec::new();
        let mut vertex_by_index: Vec<VertexId> = Vec::new();
        let mut edges: Vec<(usize, usize)> = Vec::new();

        for loop_vertices in &loops {
            let local_indices: Vec<usize> = loop_vertices
                .iter()
                .map(|&vid| {
                    *index_of.entry(vid).or_insert_with(|| {
                        points.push(plane.to_2d(self.mesh.position(vid)));
                        vertex_by_index.push(vid);
                        points.len() - 1
                    })
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
            .map(|df| {
                let outer: Vec<VertexId> = df.outer.iter().map(|&i| vertex_by_index[i]).collect();
                let holes: Vec<Vec<VertexId>> = df
                    .holes
                    .iter()
                    .map(|h| h.iter().map(|&i| vertex_by_index[i]).collect())
                    .collect();
                self.mesh.add_face(outer, holes)
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

        ProjectFile { vertices, faces, groups }
    }

    /// Rebuilds a document from a loaded project file, re-interning every
    /// vertex/face/group into fresh mesh/slotmap ids.
    pub fn from_project_file(project: &ProjectFile) -> Self {
        let mut doc = Document::new();

        let vertex_ids: Vec<VertexId> =
            project.vertices.iter().map(|p| doc.mesh.add_vertex(DVec3::new(p[0], p[1], p[2]))).collect();

        let face_ids: Vec<FaceId> = project
            .faces
            .iter()
            .map(|pf| {
                let outer: Vec<VertexId> = pf.outer.iter().map(|&i| vertex_ids[i as usize]).collect();
                let holes: Vec<Vec<VertexId>> =
                    pf.holes.iter().map(|h| h.iter().map(|&i| vertex_ids[i as usize]).collect()).collect();
                let face_id = doc.mesh.add_face(outer, holes);
                if pf.solid {
                    doc.solid_face_ids.insert(face_id);
                }
                face_id
            })
            .collect();

        for group in &project.groups {
            let member_ids: Vec<FaceId> = group.face_indices.iter().map(|&i| face_ids[i as usize]).collect();
            doc.group_faces(&member_ids, group.name.clone());
        }

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
}
