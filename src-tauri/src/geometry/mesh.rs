use std::collections::{HashMap, HashSet};

use glam::DVec3;
use slotmap::{new_key_type, SlotMap};

new_key_type! {
    pub struct VertexId;
    pub struct FaceId;
}

#[derive(Debug, Clone, Copy)]
pub struct Vertex {
    pub position: DVec3,
}

/// A planar polygon face, possibly with holes. `outer` is an ordered vertex
/// ring wound counter-clockwise as seen from the `normal` side; each loop in
/// `holes` is wound clockwise (opposite of `outer`), matching the convention
/// produced by `face_detect` and expected by `pushpull`/`triangulate`.
#[derive(Debug, Clone)]
pub struct Face {
    pub outer: Vec<VertexId>,
    pub holes: Vec<Vec<VertexId>>,
    pub normal: DVec3,
}

/// The document's persistent geometry: vertices plus independent polygon
/// faces. Faces do not share topology (no half-edge twins) — each face owns
/// its own loop of vertex references. This keeps push/pull, erase, and
/// export simple; the cost is that touching/adjacent solids don't share
/// vertices, which is fine since v1 has no boolean/union operations.
#[derive(Debug, Clone, Default)]
pub struct Mesh {
    pub vertices: SlotMap<VertexId, Vertex>,
    pub faces: SlotMap<FaceId, Face>,
}

impl Mesh {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn add_vertex(&mut self, position: DVec3) -> VertexId {
        self.vertices.insert(Vertex { position })
    }

    pub fn position(&self, id: VertexId) -> DVec3 {
        self.vertices[id].position
    }

    /// Adds a face from an outer loop and hole loops, computing the normal
    /// via Newell's method over the outer loop.
    pub fn add_face(&mut self, outer: Vec<VertexId>, holes: Vec<Vec<VertexId>>) -> FaceId {
        let points: Vec<DVec3> = outer.iter().map(|&v| self.position(v)).collect();
        let normal = newell_normal(&points);
        self.faces.insert(Face { outer, holes, normal })
    }

    pub fn remove_face(&mut self, id: FaceId) {
        self.faces.remove(id);
    }

    /// Recomputes a face's normal from its current (possibly just
    /// transformed) outer loop positions. Newell's method works directly
    /// from point positions, so this is correct after any combination of
    /// translation, rotation, or scale without needing separate normal
    /// transform math.
    pub fn recompute_normal(&mut self, id: FaceId) {
        let points: Vec<DVec3> = self.faces[id].outer.iter().map(|&v| self.position(v)).collect();
        self.faces[id].normal = newell_normal(&points);
    }

    /// Groups `face_ids` into connected components by shared vertices. Faces
    /// created by unrelated draw/push-pull operations never share vertices
    /// (see the struct doc comment above), so two faces sharing any vertex
    /// are necessarily part of the same solid - this is what lets "arrange
    /// for print" (`Document::arrange_for_print`) treat each disconnected
    /// solid in the document as its own printable part, without needing
    /// edge-based adjacency.
    pub fn connected_components(&self, face_ids: &[FaceId]) -> Vec<Vec<FaceId>> {
        let mut vertex_to_faces: HashMap<VertexId, Vec<FaceId>> = HashMap::new();
        for &fid in face_ids {
            let face = &self.faces[fid];
            for &v in face.outer.iter().chain(face.holes.iter().flatten()) {
                vertex_to_faces.entry(v).or_default().push(fid);
            }
        }

        let mut visited: HashSet<FaceId> = HashSet::new();
        let mut components = Vec::new();
        for &start in face_ids {
            if !visited.insert(start) {
                continue;
            }
            let mut stack = vec![start];
            let mut component = Vec::new();
            while let Some(fid) = stack.pop() {
                component.push(fid);
                let face = &self.faces[fid];
                for &v in face.outer.iter().chain(face.holes.iter().flatten()) {
                    for &neighbor in vertex_to_faces.get(&v).into_iter().flatten() {
                        if visited.insert(neighbor) {
                            stack.push(neighbor);
                        }
                    }
                }
            }
            components.push(component);
        }
        components
    }

    /// Axis-aligned bounding box (min, max corners) over every vertex
    /// `face_ids` reference (outer + hole loops).
    pub fn bounding_box(&self, face_ids: &[FaceId]) -> (DVec3, DVec3) {
        let mut min = DVec3::splat(f64::INFINITY);
        let mut max = DVec3::splat(f64::NEG_INFINITY);
        for &fid in face_ids {
            let face = &self.faces[fid];
            for &v in face.outer.iter().chain(face.holes.iter().flatten()) {
                let p = self.position(v);
                min = min.min(p);
                max = max.max(p);
            }
        }
        (min, max)
    }
}

/// Computes a polygon's normal via Newell's method, which is robust for
/// non-triangular and near-degenerate planar polygons. Returns a zero
/// vector for degenerate (collinear or <3-point) input.
pub fn newell_normal(points: &[DVec3]) -> DVec3 {
    let len = points.len();
    if len < 3 {
        return DVec3::ZERO;
    }
    let mut n = DVec3::ZERO;
    for i in 0..len {
        let curr = points[i];
        let next = points[(i + 1) % len];
        n.x += (curr.y - next.y) * (curr.z + next.z);
        n.y += (curr.z - next.z) * (curr.x + next.x);
        n.z += (curr.x - next.x) * (curr.y + next.y);
    }
    n.normalize_or_zero()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn newell_normal_of_unit_square_points_up() {
        let points = vec![
            DVec3::new(0.0, 0.0, 0.0),
            DVec3::new(1.0, 0.0, 0.0),
            DVec3::new(1.0, 1.0, 0.0),
            DVec3::new(0.0, 1.0, 0.0),
        ];
        let n = newell_normal(&points);
        assert!((n - DVec3::Z).length() < 1e-9);
    }

    #[test]
    fn add_face_computes_normal() {
        let mut mesh = Mesh::new();
        let a = mesh.add_vertex(DVec3::new(0.0, 0.0, 0.0));
        let b = mesh.add_vertex(DVec3::new(1.0, 0.0, 0.0));
        let c = mesh.add_vertex(DVec3::new(1.0, 1.0, 0.0));
        let d = mesh.add_vertex(DVec3::new(0.0, 1.0, 0.0));
        let face_id = mesh.add_face(vec![a, b, c, d], vec![]);
        let face = &mesh.faces[face_id];
        assert!((face.normal - DVec3::Z).length() < 1e-9);
    }

    #[test]
    fn connected_components_splits_faces_with_no_shared_vertices() {
        let mut mesh = Mesh::new();
        let a = mesh.add_vertex(DVec3::new(0.0, 0.0, 0.0));
        let b = mesh.add_vertex(DVec3::new(1.0, 0.0, 0.0));
        let c = mesh.add_vertex(DVec3::new(1.0, 1.0, 0.0));
        let face_1 = mesh.add_face(vec![a, b, c], vec![]);

        let d = mesh.add_vertex(DVec3::new(10.0, 10.0, 0.0));
        let e = mesh.add_vertex(DVec3::new(11.0, 10.0, 0.0));
        let f = mesh.add_vertex(DVec3::new(11.0, 11.0, 0.0));
        let face_2 = mesh.add_face(vec![d, e, f], vec![]);

        let components = mesh.connected_components(&[face_1, face_2]);
        assert_eq!(components.len(), 2);
        for component in &components {
            assert_eq!(component.len(), 1);
        }
    }

    #[test]
    fn connected_components_merges_faces_sharing_a_vertex() {
        let mut mesh = Mesh::new();
        let a = mesh.add_vertex(DVec3::new(0.0, 0.0, 0.0));
        let b = mesh.add_vertex(DVec3::new(1.0, 0.0, 0.0));
        let c = mesh.add_vertex(DVec3::new(1.0, 1.0, 0.0));
        let face_1 = mesh.add_face(vec![a, b, c], vec![]);

        // Shares vertex `c` with face_1.
        let d = mesh.add_vertex(DVec3::new(2.0, 2.0, 0.0));
        let e = mesh.add_vertex(DVec3::new(3.0, 2.0, 0.0));
        let face_2 = mesh.add_face(vec![c, d, e], vec![]);

        let components = mesh.connected_components(&[face_1, face_2]);
        assert_eq!(components.len(), 1);
        assert_eq!(components[0].len(), 2);
    }

    #[test]
    fn bounding_box_covers_every_referenced_vertex() {
        let mut mesh = Mesh::new();
        let a = mesh.add_vertex(DVec3::new(-1.0, 2.0, 0.0));
        let b = mesh.add_vertex(DVec3::new(5.0, 0.0, -3.0));
        let c = mesh.add_vertex(DVec3::new(1.0, 1.0, 4.0));
        let face_id = mesh.add_face(vec![a, b, c], vec![]);

        let (min, max) = mesh.bounding_box(&[face_id]);
        assert_eq!(min, DVec3::new(-1.0, 0.0, -3.0));
        assert_eq!(max, DVec3::new(5.0, 2.0, 4.0));
    }
}
