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
}
