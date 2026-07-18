use glam::DVec3;

use crate::geometry::mesh::{FaceId, Mesh};
use crate::geometry::triangulate::triangulate_face;

/// Serializes `face_ids` to the binary STL format: an 80-byte header, a
/// little-endian triangle count, then 50 bytes per triangle (a facet normal,
/// its 3 vertices, each as 3 little-endian f32s, plus a 2-byte attribute
/// count left at 0). Per-triangle normals are recomputed from the triangle's
/// own winding rather than copied from the source face, since that's what
/// every slicer actually reads and it keeps this function correct even if a
/// face's stored normal and its triangle winding were ever to disagree.
pub fn write_binary_stl(mesh: &Mesh, face_ids: &[FaceId]) -> Vec<u8> {
    let triangles: Vec<[DVec3; 3]> = face_ids
        .iter()
        .filter_map(|&id| mesh.faces.get(id))
        .flat_map(|face| triangulate_face(mesh, face))
        .map(|tri| [mesh.position(tri[0]), mesh.position(tri[1]), mesh.position(tri[2])])
        .collect();

    let mut out = Vec::with_capacity(84 + triangles.len() * 50);
    out.extend_from_slice(&[0u8; 80]);
    out.extend_from_slice(&(triangles.len() as u32).to_le_bytes());

    for tri in &triangles {
        let normal = (tri[1] - tri[0]).cross(tri[2] - tri[0]).normalize_or_zero();
        for component in [normal.x, normal.y, normal.z, tri[0].x, tri[0].y, tri[0].z, tri[1].x, tri[1].y, tri[1].z, tri[2].x, tri[2].y, tri[2].z] {
            out.extend_from_slice(&(component as f32).to_le_bytes());
        }
        out.extend_from_slice(&[0u8; 2]);
    }

    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::geometry::plane::Plane;
    use crate::geometry::primitives::add_rectangle;
    use crate::geometry::pushpull::push_pull;
    use glam::DVec2;

    #[test]
    fn writes_a_valid_binary_stl_header_and_triangle_count_for_a_cube() {
        let mut mesh = Mesh::new();
        let plane = Plane::from_normal(DVec3::ZERO, DVec3::Z);
        let face_id = add_rectangle(&mut mesh, &plane, DVec2::ZERO, DVec2::new(1.0, 1.0));
        let face_ids = push_pull(&mut mesh, face_id, 1.0);

        let bytes = write_binary_stl(&mesh, &face_ids);
        assert!(bytes.len() > 84);

        let triangle_count = u32::from_le_bytes(bytes[80..84].try_into().unwrap());
        // A unit cube: 6 faces x 2 triangles each (the two cap quads plus
        // four wall quads, each ear-clipped into 2 triangles).
        assert_eq!(triangle_count, 12);
        assert_eq!(bytes.len(), 84 + (triangle_count as usize) * 50);
    }

    #[test]
    fn every_triangle_normal_is_unit_length_and_points_away_from_the_cube_center() {
        let mut mesh = Mesh::new();
        let plane = Plane::from_normal(DVec3::ZERO, DVec3::Z);
        let face_id = add_rectangle(&mut mesh, &plane, DVec2::new(-1.0, -1.0), DVec2::new(1.0, 1.0));
        let face_ids = push_pull(&mut mesh, face_id, 2.0);
        let center = DVec3::new(0.0, 0.0, 1.0);

        let bytes = write_binary_stl(&mesh, &face_ids);
        let triangle_count = u32::from_le_bytes(bytes[80..84].try_into().unwrap()) as usize;
        for i in 0..triangle_count {
            let base = 84 + i * 50;
            let read_f32 = |offset: usize| f32::from_le_bytes(bytes[base + offset..base + offset + 4].try_into().unwrap());
            let normal = DVec3::new(read_f32(0) as f64, read_f32(4) as f64, read_f32(8) as f64);
            assert!((normal.length() - 1.0).abs() < 1e-4);
            let v0 = DVec3::new(read_f32(12) as f64, read_f32(16) as f64, read_f32(20) as f64);
            assert!(normal.dot(v0 - center) > 0.0, "normal should point outward from the solid's interior");
        }
    }
}
