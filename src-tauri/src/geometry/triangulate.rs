use glam::DVec2;

use super::mesh::{Face, Mesh, VertexId};
use super::plane::Plane;

/// Triangulates a (possibly holed) planar face into triangles referencing
/// the mesh's existing vertices. Holes are merged into the outer loop via
/// nearest-vertex bridging (a simple, robust-enough approach for the convex
/// and near-convex shapes v1's draw tools produce - circles, rectangles,
/// ring/washer profiles - rather than a general visibility-safe bridge),
/// then the resulting simple polygon is ear-clipped.
pub fn triangulate_face(mesh: &Mesh, face: &Face) -> Vec<[VertexId; 3]> {
    if face.outer.len() < 3 {
        return Vec::new();
    }
    let plane = Plane::from_normal(mesh.position(face.outer[0]), face.normal);
    let mut polygon: Vec<(VertexId, DVec2)> = face
        .outer
        .iter()
        .map(|&v| (v, plane.to_2d(mesh.position(v))))
        .collect();

    for hole in &face.holes {
        if hole.len() < 3 {
            continue;
        }
        let hole2d: Vec<(VertexId, DVec2)> = hole
            .iter()
            .map(|&v| (v, plane.to_2d(mesh.position(v))))
            .collect();
        bridge_hole(&mut polygon, &hole2d);
    }

    ear_clip(polygon)
}

/// Splices `hole` into `outer` at the pair of vertices (one from each loop)
/// with the smallest distance between them, producing a single simple
/// polygon boundary with a zero-width bridge cut.
fn bridge_hole(outer: &mut Vec<(VertexId, DVec2)>, hole: &[(VertexId, DVec2)]) {
    let mut best = (0usize, 0usize, f64::MAX);
    for (i, &(_, op)) in outer.iter().enumerate() {
        for (j, &(_, hp)) in hole.iter().enumerate() {
            let d = op.distance_squared(hp);
            if d < best.2 {
                best = (i, j, d);
            }
        }
    }
    let (oi, hj, _) = best;
    let m = hole.len();
    let mut insertion: Vec<(VertexId, DVec2)> = Vec::with_capacity(m + 2);
    for k in 0..m {
        insertion.push(hole[(hj + k) % m]);
    }
    insertion.push(hole[hj]); // close the hole loop back to its start
    insertion.push(outer[oi]); // close the bridge back to the outer vertex
    outer.splice(oi + 1..oi + 1, insertion);
}

/// Standard O(n^2)-per-ear ear-clipping triangulation of a simple polygon
/// wound counter-clockwise. Degenerate (near-zero-area) triangles produced
/// by bridge duplicate vertices are silently dropped.
fn ear_clip(mut poly: Vec<(VertexId, DVec2)>) -> Vec<[VertexId; 3]> {
    let mut triangles = Vec::new();
    let max_iterations = poly.len() * poly.len() + 16;
    let mut iterations = 0;

    while poly.len() > 3 && iterations < max_iterations {
        iterations += 1;
        let n = poly.len();
        let mut clipped_an_ear = false;

        for i in 0..n {
            let prev = poly[(i + n - 1) % n];
            let curr = poly[i];
            let next = poly[(i + 1) % n];

            if !is_convex(prev.1, curr.1, next.1) {
                continue;
            }

            let contains_other = poly.iter().enumerate().any(|(j, &(_, p))| {
                j != (i + n - 1) % n
                    && j != i
                    && j != (i + 1) % n
                    && point_in_triangle(p, prev.1, curr.1, next.1)
            });
            if contains_other {
                continue;
            }

            push_triangle_if_nondegenerate(&mut triangles, prev, curr, next);
            poly.remove(i);
            clipped_an_ear = true;
            break;
        }

        if !clipped_an_ear {
            // Remaining loop is self-intersecting or degenerate beyond what
            // ear-clipping resolves; stop instead of looping forever.
            break;
        }
    }

    if poly.len() == 3 {
        push_triangle_if_nondegenerate(&mut triangles, poly[0], poly[1], poly[2]);
    }

    triangles
}

fn push_triangle_if_nondegenerate(
    triangles: &mut Vec<[VertexId; 3]>,
    a: (VertexId, DVec2),
    b: (VertexId, DVec2),
    c: (VertexId, DVec2),
) {
    let area2 = cross2(c.1 - a.1, b.1 - a.1).abs();
    if area2 > 1e-12 {
        triangles.push([a.0, b.0, c.0]);
    }
}

fn cross2(a: DVec2, b: DVec2) -> f64 {
    a.x * b.y - a.y * b.x
}

fn is_convex(prev: DVec2, curr: DVec2, next: DVec2) -> bool {
    cross2(curr - prev, next - curr) > 1e-12
}

/// Strictly-interior containment (excludes the boundary and vertices). This
/// must stay strict rather than boundary-inclusive: hole-bridging
/// deliberately duplicates vertices at the bridge's endpoints, so a
/// boundary-inclusive test would see those coincident duplicates as
/// "another vertex inside the ear" and block every ear near the bridge.
/// `a`, `b`, `c` are assumed counter-clockwise (guaranteed by the `is_convex`
/// check callers perform first), so a strictly-interior point yields the
/// same sign (positive) for all three edge tests.
fn point_in_triangle(p: DVec2, a: DVec2, b: DVec2, c: DVec2) -> bool {
    let d1 = cross2(b - a, p - a);
    let d2 = cross2(c - b, p - b);
    let d3 = cross2(a - c, p - c);
    d1 > 1e-12 && d2 > 1e-12 && d3 > 1e-12
}

#[cfg(test)]
mod tests {
    use super::*;
    use glam::DVec3;

    #[test]
    fn triangulates_simple_square_into_two_triangles() {
        let mut mesh = Mesh::new();
        let a = mesh.add_vertex(DVec3::new(0.0, 0.0, 0.0));
        let b = mesh.add_vertex(DVec3::new(1.0, 0.0, 0.0));
        let c = mesh.add_vertex(DVec3::new(1.0, 1.0, 0.0));
        let d = mesh.add_vertex(DVec3::new(0.0, 1.0, 0.0));
        let face_id = mesh.add_face(vec![a, b, c, d], vec![]);
        let tris = triangulate_face(&mesh, &mesh.faces[face_id]);
        assert_eq!(tris.len(), 2);
    }

    #[test]
    fn triangulates_ring_with_hole() {
        // Outer square (CCW) with an inner square hole (CW), like the
        // washer/ring profile from the erase-inner-face workflow.
        let mut mesh = Mesh::new();
        let o0 = mesh.add_vertex(DVec3::new(-2.0, -2.0, 0.0));
        let o1 = mesh.add_vertex(DVec3::new(2.0, -2.0, 0.0));
        let o2 = mesh.add_vertex(DVec3::new(2.0, 2.0, 0.0));
        let o3 = mesh.add_vertex(DVec3::new(-2.0, 2.0, 0.0));

        let h0 = mesh.add_vertex(DVec3::new(-1.0, 1.0, 0.0));
        let h1 = mesh.add_vertex(DVec3::new(1.0, 1.0, 0.0));
        let h2 = mesh.add_vertex(DVec3::new(1.0, -1.0, 0.0));
        let h3 = mesh.add_vertex(DVec3::new(-1.0, -1.0, 0.0));

        let face_id = mesh.add_face(vec![o0, o1, o2, o3], vec![vec![h0, h1, h2, h3]]);
        let tris = triangulate_face(&mesh, &mesh.faces[face_id]);

        // Ring area = outer(16) - inner(4) = 12; sum of triangle areas should match.
        let total_area: f64 = tris
            .iter()
            .map(|tri| {
                let p: Vec<DVec2> = tri.iter().map(|&v| mesh.position(v)).map(|p3| DVec2::new(p3.x, p3.y)).collect();
                (cross2(p[1] - p[0], p[2] - p[0])).abs() / 2.0
            })
            .sum();
        assert!((total_area - 12.0).abs() < 1e-6, "total_area = {total_area}");
    }

    #[test]
    fn triangulates_16_segment_circle_ring_covering_full_area_no_overlap() {
        // Realistic case (unlike the 4-vertex square ring above): two
        // 16-segment circles, the shape the reported "dimple instead of a
        // hole" bug actually came from.
        use super::super::plane::Plane;
        use super::super::primitives::add_circle;

        let mut mesh = Mesh::new();
        let plane = Plane::from_normal(DVec3::new(0.0, 0.0, 0.0), DVec3::Z);
        let outer_id = add_circle(&mut mesh, &plane, DVec2::ZERO, 5.0, 16);
        let inner_id = add_circle(&mut mesh, &plane, DVec2::ZERO, 2.0, 16);
        let mut inner_loop = mesh.faces[inner_id].outer.clone();
        inner_loop.reverse();
        mesh.remove_face(inner_id);
        let outer_loop = mesh.faces[outer_id].outer.clone();
        mesh.remove_face(outer_id);
        let ring_id = mesh.add_face(outer_loop, vec![inner_loop]);

        let tris = triangulate_face(&mesh, &mesh.faces[ring_id]);

        let mut total_area = 0.0;
        for tri in &tris {
            let p: Vec<DVec2> = tri.iter().map(|&v| mesh.position(v)).map(|p3| DVec2::new(p3.x, p3.y)).collect();
            let signed_area = cross2(p[1] - p[0], p[2] - p[0]) / 2.0;
            // Every triangle must wind the same way as the outer loop (CCW,
            // positive signed area); a flipped triangle here would mean the
            // triangulation crosses itself.
            assert!(signed_area > 0.0, "inconsistently wound triangle: {tri:?} at {p:?}");
            total_area += signed_area;
        }

        let expected = std::f64::consts::PI * (5.0_f64.powi(2) - 2.0_f64.powi(2));
        // Loose tolerance: 16-gon approximations of the circles, not true circles.
        assert!(
            (total_area - expected).abs() / expected < 0.05,
            "total_area = {total_area}, expected ~= {expected}"
        );
    }
}
