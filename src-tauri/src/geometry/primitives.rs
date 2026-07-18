use glam::DVec2;

use super::mesh::{FaceId, Mesh, VertexId};
use super::plane::Plane;

/// Builds an axis-aligned (in plane-local coordinates) rectangle loop,
/// counter-clockwise as seen from `plane.normal`, and adds it as a face.
pub fn add_rectangle(mesh: &mut Mesh, plane: &Plane, corner_a: DVec2, corner_b: DVec2) -> FaceId {
    let min = corner_a.min(corner_b);
    let max = corner_a.max(corner_b);
    let corners_2d = [
        DVec2::new(min.x, min.y),
        DVec2::new(max.x, min.y),
        DVec2::new(max.x, max.y),
        DVec2::new(min.x, max.y),
    ];
    let loop_ids: Vec<VertexId> = corners_2d.iter().map(|&p| mesh.add_vertex(plane.to_3d(p))).collect();
    mesh.add_face(loop_ids, vec![])
}

/// Builds a regular-polygon approximation of a circle, counter-clockwise as
/// seen from `plane.normal`, and adds it as a face.
pub fn add_circle(mesh: &mut Mesh, plane: &Plane, center: DVec2, radius: f64, segments: usize) -> FaceId {
    let segments = segments.max(3);
    let loop_ids: Vec<VertexId> = (0..segments)
        .map(|i| {
            let angle = (i as f64 / segments as f64) * std::f64::consts::TAU;
            let p = center + DVec2::new(angle.cos(), angle.sin()) * radius;
            mesh.add_vertex(plane.to_3d(p))
        })
        .collect();
    mesh.add_face(loop_ids, vec![])
}

/// Builds a face from an explicit closed loop of plane-local points, e.g.
/// once the line/polyline draw tool closes back on its start point.
pub fn add_polyline_loop(mesh: &mut Mesh, plane: &Plane, points_2d: &[DVec2]) -> Option<FaceId> {
    if points_2d.len() < 3 {
        return None;
    }
    let loop_ids: Vec<VertexId> = points_2d.iter().map(|&p| mesh.add_vertex(plane.to_3d(p))).collect();
    Some(mesh.add_face(loop_ids, vec![]))
}

#[cfg(test)]
mod tests {
    use super::*;
    use glam::DVec3;

    #[test]
    fn rectangle_has_four_vertices_and_faces_plane_normal() {
        let mut mesh = Mesh::new();
        let plane = Plane::from_normal(DVec3::ZERO, DVec3::Z);
        let face_id = add_rectangle(&mut mesh, &plane, DVec2::new(0.0, 0.0), DVec2::new(2.0, 1.0));
        let face = &mesh.faces[face_id];
        assert_eq!(face.outer.len(), 4);
        assert!((face.normal - DVec3::Z).length() < 1e-9);
    }

    #[test]
    fn circle_has_requested_segment_count() {
        let mut mesh = Mesh::new();
        let plane = Plane::from_normal(DVec3::ZERO, DVec3::Z);
        let face_id = add_circle(&mut mesh, &plane, DVec2::ZERO, 5.0, 24);
        let face = &mesh.faces[face_id];
        assert_eq!(face.outer.len(), 24);
        for &v in &face.outer {
            let p = mesh.position(v);
            assert!((p.length() - 5.0).abs() < 1e-9);
        }
    }
}
