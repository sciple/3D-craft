use std::collections::HashMap;

use super::mesh::{FaceId, Mesh, VertexId};
use super::triangulate::triangulate_face;

/// Extrudes `face_id` along its own normal by `distance`, replacing the flat
/// source face with a closed solid: a cap at the original position, a cap at
/// the offset position, and a side wall per boundary edge (for both the
/// outer loop and any hole loops, so a face-with-a-hole extrudes into a
/// hollow tube rather than a filled solid). Returns the newly created faces.
///
/// Extrudes a standalone flat sketch face. For a face that's already part of
/// a closed solid's boundary (a cap or wall), use `push_pull_attached`
/// instead - this variant would leave a coincident interior cap at the
/// source position, breaking the combined shell's manifoldness.
pub fn push_pull(mesh: &mut Mesh, face_id: FaceId, distance: f64) -> Vec<FaceId> {
    push_pull_impl(mesh, face_id, distance, false)
}

/// Push/pull for a face lying on an existing solid's boundary. The
/// surrounding solid already provides the surface around the source loop, so
/// no cap is emitted at the source position - the new side walls' base edges
/// pair directly with the neighboring faces' edges, keeping the merged shell
/// watertight. Winding is always "forward" regardless of the distance's
/// sign: pulling outward grows the solid, pushing inward carves a recess
/// (the walls/cap then face the recess interior, and any overlap with the
/// original walls nets out by winding, which slicers resolve correctly).
pub fn push_pull_attached(mesh: &mut Mesh, face_id: FaceId, distance: f64) -> Vec<FaceId> {
    push_pull_impl(mesh, face_id, distance, true)
}

fn push_pull_impl(mesh: &mut Mesh, face_id: FaceId, distance: f64, attached: bool) -> Vec<FaceId> {
    if distance.abs() < 1e-9 {
        return Vec::new();
    }
    let face = mesh.faces[face_id].clone();
    let offset = face.normal * distance;
    let extruding_forward = attached || distance >= 0.0;

    let offset_vertex = |mesh: &mut Mesh, v: VertexId| -> VertexId {
        let p = mesh.position(v) + offset;
        mesh.add_vertex(p)
    };

    let offset_outer: Vec<VertexId> = face.outer.iter().map(|&v| offset_vertex(mesh, v)).collect();
    let offset_holes: Vec<Vec<VertexId>> = face
        .holes
        .iter()
        .map(|h| h.iter().map(|&v| offset_vertex(mesh, v)).collect())
        .collect();

    let mut new_face_ids = Vec::new();

    // Side walls for every loop (outer + each hole): the quad winding is
    // flipped when extruding backward so the wall normal always faces away
    // from the solid, regardless of push/pull direction.
    let mut loop_pairs: Vec<(&Vec<VertexId>, &Vec<VertexId>)> = vec![(&face.outer, &offset_outer)];
    loop_pairs.extend(face.holes.iter().zip(offset_holes.iter()));
    for (source_loop, offset_loop) in loop_pairs {
        let n = source_loop.len();
        for i in 0..n {
            let (a, b) = (source_loop[i], source_loop[(i + 1) % n]);
            let (a2, b2) = (offset_loop[i], offset_loop[(i + 1) % n]);
            let quad = if extruding_forward {
                vec![a, b, b2, a2]
            } else {
                vec![a, a2, b2, b]
            };
            new_face_ids.push(mesh.add_face(quad, vec![]));
        }
    }

    let reverse_loop = |l: &[VertexId]| -> Vec<VertexId> {
        let mut r = l.to_vec();
        r.reverse();
        r
    };

    // Caps: whichever end sits at the smaller extent along the source
    // normal faces backward (reversed winding); the other faces forward.
    // An attached extrusion emits no source-position cap at all - the
    // surrounding solid's faces already border that loop.
    if !attached {
        let (source_outer, source_holes) = if extruding_forward {
            (reverse_loop(&face.outer), face.holes.iter().map(|h| reverse_loop(h)).collect::<Vec<_>>())
        } else {
            (face.outer.clone(), face.holes.clone())
        };
        new_face_ids.push(mesh.add_face(source_outer, source_holes));
    }
    let (far_outer, far_holes) = if extruding_forward {
        (offset_outer, offset_holes)
    } else {
        (reverse_loop(&offset_outer), offset_holes.iter().map(|h| reverse_loop(h)).collect::<Vec<_>>())
    };
    new_face_ids.push(mesh.add_face(far_outer, far_holes));

    mesh.remove_face(face_id);
    new_face_ids
}

/// Checks that `face_ids` form a closed, consistently-wound (watertight) 2D
/// manifold: every triangulated edge must appear exactly once in each
/// direction (i.e. shared by exactly two triangles with opposite winding).
/// A mesh that fails this check will fail to slice for 3D printing.
pub fn is_manifold(mesh: &Mesh, face_ids: &[FaceId]) -> bool {
    let mut directed_edge_counts: HashMap<(VertexId, VertexId), u32> = HashMap::new();
    for &face_id in face_ids {
        let Some(face) = mesh.faces.get(face_id) else {
            continue;
        };
        for tri in triangulate_face(mesh, face) {
            for k in 0..3 {
                let a = tri[k];
                let b = tri[(k + 1) % 3];
                *directed_edge_counts.entry((a, b)).or_insert(0) += 1;
            }
        }
    }
    directed_edge_counts.iter().all(|(&(a, b), &count)| {
        count == 1 && directed_edge_counts.get(&(b, a)).copied().unwrap_or(0) == 1
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::geometry::plane::Plane;
    use crate::geometry::primitives::{add_circle, add_rectangle};
    use glam::{DVec2, DVec3};

    #[test]
    fn push_pull_square_makes_a_manifold_box_with_six_faces() {
        let mut mesh = Mesh::new();
        let plane = Plane::from_normal(DVec3::ZERO, DVec3::Z);
        let face_id = add_rectangle(&mut mesh, &plane, DVec2::new(0.0, 0.0), DVec2::new(1.0, 1.0));
        let new_faces = push_pull(&mut mesh, face_id, 1.0);
        assert_eq!(new_faces.len(), 6);
        assert!(is_manifold(&mesh, &new_faces));
    }

    #[test]
    fn push_pull_negative_distance_is_still_manifold() {
        let mut mesh = Mesh::new();
        let plane = Plane::from_normal(DVec3::ZERO, DVec3::Z);
        let face_id = add_rectangle(&mut mesh, &plane, DVec2::new(0.0, 0.0), DVec2::new(1.0, 1.0));
        let new_faces = push_pull(&mut mesh, face_id, -2.0);
        assert_eq!(new_faces.len(), 6);
        assert!(is_manifold(&mesh, &new_faces));
    }

    #[test]
    fn push_pull_ring_makes_a_hollow_manifold_tube() {
        // Outer circle with an inner circle erased out, i.e. a washer/ring
        // profile - the scenario this feature exists for.
        let mut mesh = Mesh::new();
        let plane = Plane::from_normal(DVec3::ZERO, DVec3::Z);
        let outer_id = add_circle(&mut mesh, &plane, DVec2::ZERO, 5.0, 16);
        let inner_id = add_circle(&mut mesh, &plane, DVec2::ZERO, 2.0, 16);
        let inner_loop = mesh.faces[inner_id].outer.clone();
        mesh.remove_face(inner_id);
        let mut ring_loop = inner_loop;
        ring_loop.reverse(); // holes are wound opposite the outer loop
        let ring_outer = mesh.faces[outer_id].outer.clone();
        mesh.remove_face(outer_id);
        let ring_face_id = mesh.add_face(ring_outer, vec![ring_loop]);

        let new_faces = push_pull(&mut mesh, ring_face_id, 3.0);
        // 2 caps + 16 outer-wall quads + 16 inner-wall quads.
        assert_eq!(new_faces.len(), 2 + 16 + 16);
        assert!(is_manifold(&mesh, &new_faces));
    }
}
