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

/// Builds a regular polygon (5-8 sides), counter-clockwise as seen from
/// `plane.normal`, and adds it as a face. Structurally identical to
/// `add_circle` (evenly-spaced points on a circle *is* a regular polygon)
/// except `start_angle_deg` is exposed so the draw tool can set the
/// polygon's rotation from a single click - the same "click defines a
/// vertex" convention `add_arc` uses for its start angle.
pub fn add_ngon(mesh: &mut Mesh, plane: &Plane, center: DVec2, radius: f64, sides: usize, start_angle_deg: f64) -> FaceId {
    let sides = sides.clamp(5, 8);
    let start = start_angle_deg.to_radians();
    let loop_ids: Vec<VertexId> = (0..sides)
        .map(|i| {
            let angle = start + (i as f64 / sides as f64) * std::f64::consts::TAU;
            let p = center + DVec2::new(angle.cos(), angle.sin()) * radius;
            mesh.add_vertex(plane.to_3d(p))
        })
        .collect();
    mesh.add_face(loop_ids, vec![])
}

/// Builds a chord-closed circular segment (an arc, closed by a straight line
/// between its two endpoints rather than through a center point), wound
/// counter-clockwise as seen from `plane.normal` for a positive sweep, and
/// adds it as a face. At a 180 degree sweep this is an exact semicircle
/// ("D" shape) - a half-pipe cross-section once pushed/pulled or inset.
/// Point order doesn't need to be CCW-correct on its own: like
/// `add_polyline_loop`, callers route this through `Document::resplit`,
/// which re-derives winding from the undirected edge graph regardless of
/// input order.
pub fn add_arc(mesh: &mut Mesh, plane: &Plane, center: DVec2, radius: f64, start_angle_deg: f64, sweep_deg: f64, segments: usize) -> FaceId {
    let segments = segments.max(2); // segments+1 points, at least 3 for a valid face
    let start = start_angle_deg.to_radians();
    let sweep = sweep_deg.to_radians();
    let loop_ids: Vec<VertexId> = (0..=segments)
        .map(|i| {
            let angle = start + sweep * (i as f64 / segments as f64);
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
    fn arc_has_requested_point_count_and_correct_endpoints() {
        let mut mesh = Mesh::new();
        let plane = Plane::from_normal(DVec3::ZERO, DVec3::Z);
        let face_id = add_arc(&mut mesh, &plane, DVec2::ZERO, 5.0, 0.0, 180.0, 16);
        let face = &mesh.faces[face_id];
        assert_eq!(face.outer.len(), 17); // segments + 1

        for &v in &face.outer {
            let p = mesh.position(v);
            assert!((p.length() - 5.0).abs() < 1e-9);
        }

        let start = mesh.position(face.outer[0]);
        assert!((start - DVec3::new(5.0, 0.0, 0.0)).length() < 1e-9);
        let end = mesh.position(*face.outer.last().unwrap());
        assert!((end - DVec3::new(-5.0, 0.0, 0.0)).length() < 1e-6);
    }

    #[test]
    fn ngon_has_requested_side_count_and_correct_rotation() {
        let mut mesh = Mesh::new();
        let plane = Plane::from_normal(DVec3::ZERO, DVec3::Z);
        let face_id = add_ngon(&mut mesh, &plane, DVec2::ZERO, 5.0, 6, 0.0);
        let face = &mesh.faces[face_id];
        assert_eq!(face.outer.len(), 6);
        for &v in &face.outer {
            let p = mesh.position(v);
            assert!((p.length() - 5.0).abs() < 1e-9);
        }
        let first = mesh.position(face.outer[0]);
        assert!((first - DVec3::new(5.0, 0.0, 0.0)).length() < 1e-9);
    }

    #[test]
    fn ngon_clamps_sides_to_five_and_eight() {
        let mut mesh = Mesh::new();
        let plane = Plane::from_normal(DVec3::ZERO, DVec3::Z);
        let too_few = add_ngon(&mut mesh, &plane, DVec2::ZERO, 5.0, 3, 0.0);
        assert_eq!(mesh.faces[too_few].outer.len(), 5);
        let too_many = add_ngon(&mut mesh, &plane, DVec2::ZERO, 5.0, 20, 0.0);
        assert_eq!(mesh.faces[too_many].outer.len(), 8);
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
