use std::collections::{HashMap, HashSet};

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

/// The specific edges that make a face set fail the watertightness check,
/// retained so the UI can point at the broken spot instead of only saying
/// "something is open somewhere". See `check_manifold`.
#[derive(Debug, Default, Clone)]
pub struct ManifoldIssues {
    /// Triangulated edges with no opposite-direction twin: an open border -
    /// a missing or erased face, or two faces meeting at a T-junction.
    pub open_edges: Vec<(VertexId, VertexId)>,
    /// Edges appearing more than once in the same direction: duplicated or
    /// inconsistently wound geometry. Reported once per undirected edge.
    pub duplicate_edges: Vec<(VertexId, VertexId)>,
    /// Every face that contributed one of the above, deduplicated.
    pub problem_faces: Vec<FaceId>,
}

impl ManifoldIssues {
    pub fn is_empty(&self) -> bool {
        self.open_edges.is_empty() && self.duplicate_edges.is_empty()
    }
}

/// Checks that `face_ids` form a closed, consistently-wound (watertight) 2D
/// manifold: every triangulated edge must appear exactly once in each
/// direction (i.e. shared by exactly two triangles with opposite winding).
/// A mesh that fails this check will fail to slice for 3D printing.
///
/// Note this does *not* catch 2D self-overlap within a single face's own
/// triangulation, only 3D edge pairing.
pub fn is_manifold(mesh: &Mesh, face_ids: &[FaceId]) -> bool {
    check_manifold(mesh, face_ids).is_empty()
}

/// The detail-returning form of `is_manifold`: same edge-pairing pass, but
/// it keeps the offending edges (and the faces they came from) instead of
/// collapsing everything to a bool, so a failed STL export can show the user
/// *where* the model is open.
pub fn check_manifold(mesh: &Mesh, face_ids: &[FaceId]) -> ManifoldIssues {
    // Value is (how many times this exact direction occurs, which faces
    // contributed it) - the face attribution is the only thing added over a
    // plain count.
    let mut directed_edges: HashMap<(VertexId, VertexId), (u32, Vec<FaceId>)> = HashMap::new();
    for &face_id in face_ids {
        let Some(face) = mesh.faces.get(face_id) else {
            continue;
        };
        for tri in triangulate_face(mesh, face) {
            for k in 0..3 {
                let a = tri[k];
                let b = tri[(k + 1) % 3];
                let entry = directed_edges.entry((a, b)).or_insert((0, Vec::new()));
                entry.0 += 1;
                if !entry.1.contains(&face_id) {
                    entry.1.push(face_id);
                }
            }
        }
    }

    let mut issues = ManifoldIssues::default();
    let mut problem_faces: HashSet<FaceId> = HashSet::new();
    let mut seen_duplicates: HashSet<(VertexId, VertexId)> = HashSet::new();
    for (&(a, b), (count, faces)) in directed_edges.iter() {
        let twin_count = directed_edges.get(&(b, a)).map(|(c, _)| *c).unwrap_or(0);
        if *count > 1 {
            // Both directions of a duplicated edge land here, so key the
            // report on the undirected edge to avoid listing it twice.
            let key = if a < b { (a, b) } else { (b, a) };
            if seen_duplicates.insert(key) {
                issues.duplicate_edges.push((a, b));
            }
        } else if twin_count == 0 {
            // Only one direction exists by definition - no dedup needed.
            issues.open_edges.push((a, b));
        } else {
            continue;
        }
        problem_faces.extend(faces.iter().copied());
    }
    issues.problem_faces = problem_faces.into_iter().collect();
    issues
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

    /// Builds a unit box and returns (mesh, its 6 face ids).
    fn unit_box() -> (Mesh, Vec<FaceId>) {
        let mut mesh = Mesh::new();
        let plane = Plane::from_normal(DVec3::ZERO, DVec3::Z);
        let face_id = add_rectangle(&mut mesh, &plane, DVec2::new(0.0, 0.0), DVec2::new(1.0, 1.0));
        let faces = push_pull(&mut mesh, face_id, 1.0);
        (mesh, faces)
    }

    #[test]
    fn check_manifold_reports_nothing_for_a_closed_solid() {
        let (mesh, faces) = unit_box();
        let issues = check_manifold(&mesh, &faces);
        assert!(issues.is_empty());
        assert!(issues.open_edges.is_empty());
        assert!(issues.duplicate_edges.is_empty());
        assert!(issues.problem_faces.is_empty());
    }

    #[test]
    fn erasing_a_face_reports_the_hole_rim_and_the_faces_around_it() {
        let (mut mesh, faces) = unit_box();
        // Erase one wall, the "I deleted part of my solid" case that makes
        // STL export refuse.
        let erased = faces[0];
        let erased_edge_count = mesh.faces[erased].outer.len();
        mesh.remove_face(erased);
        let remaining: Vec<FaceId> = faces.into_iter().filter(|&f| f != erased).collect();

        let issues = check_manifold(&mesh, &remaining);
        assert!(!issues.is_empty());
        assert!(issues.duplicate_edges.is_empty(), "a plain hole isn't duplicated geometry");
        // The rim of the hole: one open edge per boundary edge of the face
        // that used to close it.
        assert_eq!(issues.open_edges.len(), erased_edge_count);
        // Every reported face is one that's still in the mesh (the erased
        // one can't be pointed at - it's gone), and the rim of a box's wall
        // touches the four faces around it.
        assert_eq!(issues.problem_faces.len(), 4);
        for &fid in &issues.problem_faces {
            assert!(mesh.faces.contains_key(fid));
        }
    }

    #[test]
    fn a_doubled_face_is_reported_as_duplicate_not_open() {
        let (mut mesh, mut faces) = unit_box();
        // Same loop, same winding, added twice: every one of its edges now
        // occurs twice in the same direction.
        let doubled = mesh.faces[faces[0]].clone();
        faces.push(mesh.add_face(doubled.outer, doubled.holes));

        let issues = check_manifold(&mesh, &faces);
        assert!(!issues.is_empty());
        assert!(issues.open_edges.is_empty());
        // Every boundary edge of the doubled face is reported, once per
        // undirected edge rather than once per direction (which is why the
        // lookup below has to accept either orientation).
        let outer = mesh.faces[faces[0]].outer.clone();
        for i in 0..outer.len() {
            let (a, b) = (outer[i], outer[(i + 1) % outer.len()]);
            assert!(
                issues.duplicate_edges.contains(&(a, b)) || issues.duplicate_edges.contains(&(b, a)),
                "boundary edge {i} of the doubled face should be reported as duplicated"
            );
        }
    }

    #[test]
    fn is_manifold_still_agrees_with_check_manifold() {
        let (mut mesh, faces) = unit_box();
        assert!(is_manifold(&mesh, &faces));
        let erased = faces[0];
        mesh.remove_face(erased);
        let remaining: Vec<FaceId> = faces.into_iter().filter(|&f| f != erased).collect();
        assert!(!is_manifold(&mesh, &remaining));
    }
}
