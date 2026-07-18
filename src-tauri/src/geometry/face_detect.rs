use std::collections::HashSet;

use glam::DVec2;

/// An ordered, closed boundary loop of indices into the points slice passed
/// to `detect_faces`.
pub type LoopIndices = Vec<usize>;

/// A face reconstructed from a coplanar edge graph: an outer boundary plus
/// any loops nested immediately inside it (holes). Matches SketchUp's
/// "sticky geometry" behavior - drawing a closed loop inside an existing
/// coplanar face splits it into an inner face and an outer face-with-a-hole,
/// both of which are returned here.
#[derive(Debug, Clone, PartialEq)]
pub struct DetectedFace {
    pub outer: LoopIndices,
    pub holes: Vec<LoopIndices>,
}

/// Reconstructs the planar faces (including holes from nested loops) bound
/// by a set of coplanar points and the undirected edges connecting them.
///
/// Runs whenever new coplanar edges are added to a drawing plane, so
/// drawing a circle inside an existing face auto-splits it. Uses the
/// standard half-edge "rotate to the next-clockwise neighbor" algorithm to
/// trace every boundary cycle in the planar graph, keeps only the
/// positive-area (counter-clockwise) cycles as candidate faces, then nests
/// them by point-in-polygon containment: a candidate immediately inside
/// another becomes both its own face and a hole of the containing one.
pub fn detect_faces(points: &[DVec2], edges: &[(usize, usize)]) -> Vec<DetectedFace> {
    let loops = trace_ccw_loops(points, edges);
    build_face_forest(points, &loops)
}

fn trace_ccw_loops(points: &[DVec2], edges: &[(usize, usize)]) -> Vec<LoopIndices> {
    let n = points.len();
    let mut outgoing: Vec<Vec<usize>> = vec![Vec::new(); n];
    for &(a, b) in edges {
        if a == b {
            continue;
        }
        if !outgoing[a].contains(&b) {
            outgoing[a].push(b);
        }
        if !outgoing[b].contains(&a) {
            outgoing[b].push(a);
        }
    }
    // Sort each vertex's neighbors by descending angle (clockwise order),
    // which is what the next-pointer rule below rotates through.
    for (v, neighbors) in outgoing.iter_mut().enumerate() {
        neighbors.sort_by(|&x, &y| {
            let ax = angle_at(points, v, x);
            let ay = angle_at(points, v, y);
            ay.partial_cmp(&ax).unwrap_or(std::cmp::Ordering::Equal)
        });
    }

    let mut visited: HashSet<(usize, usize)> = HashSet::new();
    let mut loops = Vec::new();

    for &(a, b) in edges {
        if a == b {
            continue;
        }
        for start in [(a, b), (b, a)] {
            if visited.contains(&start) {
                continue;
            }
            let mut cycle = vec![start.0];
            let (mut origin, mut dest) = start;
            loop {
                visited.insert((origin, dest));
                cycle.push(dest);

                let neighbors = &outgoing[dest];
                let twin_idx = neighbors.iter().position(|&x| x == origin).expect(
                    "dest must have origin as a neighbor since (origin, dest) is an edge",
                );
                let next_idx = (twin_idx + 1) % neighbors.len();
                let next_vertex = neighbors[next_idx];

                let (new_origin, new_dest) = (dest, next_vertex);
                if (new_origin, new_dest) == start {
                    break;
                }
                origin = new_origin;
                dest = new_dest;
            }
            cycle.pop(); // drop the duplicate closing vertex
            if cycle.len() >= 3 && polygon_area(points, &cycle) > 1e-9 {
                loops.push(cycle);
            }
        }
    }

    loops
}

fn build_face_forest(points: &[DVec2], loops: &[LoopIndices]) -> Vec<DetectedFace> {
    let areas: Vec<f64> = loops.iter().map(|l| polygon_area(points, l).abs()).collect();
    let centroids: Vec<DVec2> = loops.iter().map(|l| centroid(points, l)).collect();

    // immediate_parent[i] = the smallest-area loop that contains loop i's
    // centroid (its nearest enclosing loop), if any.
    let mut immediate_parent: Vec<Option<usize>> = vec![None; loops.len()];
    for i in 0..loops.len() {
        let mut best: Option<(usize, f64)> = None;
        for j in 0..loops.len() {
            // A container must be strictly larger: for concentric loops (e.g.
            // concentric circles) both centroids coincide at the same point,
            // so a plain "is my centroid inside the other loop" test is
            // symmetric and would let the smaller loop look like it contains
            // the bigger one too. Requiring area(j) > area(i) picks the
            // correct direction.
            if i == j || areas[j] <= areas[i] {
                continue;
            }
            if point_in_polygon(centroids[i], points, &loops[j])
                && best.is_none_or(|(_, best_area)| areas[j] < best_area)
            {
                best = Some((j, areas[j]));
            }
        }
        immediate_parent[i] = best.map(|(j, _)| j);
    }

    let mut faces: Vec<DetectedFace> = loops
        .iter()
        .map(|l| DetectedFace { outer: l.clone(), holes: Vec::new() })
        .collect();
    for (i, parent) in immediate_parent.iter().enumerate() {
        if let Some(p) = parent {
            // `trace_ccw_loops` only ever keeps positive-area (CCW) cycles,
            // so `loops[i]` here is wound the same way as its containing
            // face's outer loop. A hole must be wound the opposite way (see
            // `Face::holes`'s doc comment) - `triangulate_face`'s
            // hole-bridging produces a self-overlapping (not simple)
            // polygon otherwise, silently doubling up triangle area.
            let mut hole = loops[i].clone();
            hole.reverse();
            faces[*p].holes.push(hole);
        }
    }
    faces
}

fn angle_at(points: &[DVec2], from: usize, to: usize) -> f64 {
    let d = points[to] - points[from];
    d.y.atan2(d.x)
}

fn polygon_area(points: &[DVec2], loop_indices: &[usize]) -> f64 {
    let n = loop_indices.len();
    let mut area = 0.0;
    for i in 0..n {
        let p0 = points[loop_indices[i]];
        let p1 = points[loop_indices[(i + 1) % n]];
        area += p0.x * p1.y - p1.x * p0.y;
    }
    area / 2.0
}

fn centroid(points: &[DVec2], loop_indices: &[usize]) -> DVec2 {
    let sum = loop_indices
        .iter()
        .map(|&i| points[i])
        .fold(DVec2::ZERO, |acc, p| acc + p);
    sum / loop_indices.len() as f64
}

fn point_in_polygon(p: DVec2, points: &[DVec2], loop_indices: &[usize]) -> bool {
    let n = loop_indices.len();
    let mut inside = false;
    let mut j = n - 1;
    for i in 0..n {
        let pi = points[loop_indices[i]];
        let pj = points[loop_indices[j]];
        if ((pi.y > p.y) != (pj.y > p.y))
            && (p.x < (pj.x - pi.x) * (p.y - pi.y) / (pj.y - pi.y) + pi.x)
        {
            inside = !inside;
        }
        j = i;
    }
    inside
}

#[cfg(test)]
mod tests {
    use super::*;

    fn loop_edges(indices: &[usize]) -> Vec<(usize, usize)> {
        (0..indices.len())
            .map(|i| (indices[i], indices[(i + 1) % indices.len()]))
            .collect()
    }

    #[test]
    fn single_triangle_is_one_face_no_holes() {
        let points = vec![DVec2::new(0.0, 0.0), DVec2::new(1.0, 0.0), DVec2::new(0.0, 1.0)];
        let edges = loop_edges(&[0, 1, 2]);
        let faces = detect_faces(&points, &edges);
        assert_eq!(faces.len(), 1);
        assert_eq!(faces[0].holes.len(), 0);
    }

    #[test]
    fn diagonal_splits_rectangle_into_two_faces() {
        // Rectangle 0,1,2,3 (CCW) plus a diagonal edge 0-2 splitting it into
        // two triangles - the "line splits an existing face" workflow.
        let points = vec![
            DVec2::new(0.0, 0.0),
            DVec2::new(1.0, 0.0),
            DVec2::new(1.0, 1.0),
            DVec2::new(0.0, 1.0),
        ];
        let mut edges = loop_edges(&[0, 1, 2, 3]);
        edges.push((0, 2));
        let faces = detect_faces(&points, &edges);
        assert_eq!(faces.len(), 2);
        assert!(faces.iter().all(|f| f.holes.is_empty()));
    }

    #[test]
    fn hole_loop_is_wound_opposite_the_outer_loop() {
        // mesh.rs documents that a face's `holes` loops must be wound
        // clockwise, opposite `outer`'s counter-clockwise winding -
        // triangulate.rs's hole-bridging depends on that to build a valid
        // simple polygon. `trace_ccw_loops` only ever keeps positive-area
        // (CCW) cycles, so a hole assigned straight from `loops[i]` without
        // reversing comes out wound the *same* way as outer - this must not
        // happen.
        let points = vec![
            DVec2::new(-2.0, -2.0),
            DVec2::new(2.0, -2.0),
            DVec2::new(2.0, 2.0),
            DVec2::new(-2.0, 2.0),
            DVec2::new(-1.0, -1.0),
            DVec2::new(1.0, -1.0),
            DVec2::new(1.0, 1.0),
            DVec2::new(-1.0, 1.0),
        ];
        let mut edges = loop_edges(&[0, 1, 2, 3]);
        edges.extend(loop_edges(&[4, 5, 6, 7]));

        let faces = detect_faces(&points, &edges);
        let outer_face = faces.iter().find(|f| f.outer.contains(&0)).unwrap();
        let outer_area = polygon_area(&points, &outer_face.outer);
        let hole_area = polygon_area(&points, &outer_face.holes[0]);
        assert!(outer_area > 0.0, "outer loop must be CCW (positive area)");
        assert!(hole_area < 0.0, "hole loop must be CW (negative area) - opposite the outer loop");
    }

    #[test]
    fn nested_square_becomes_ring_hole_and_own_face() {
        // Disconnected outer + inner square, like two concentric circles:
        // must produce the outer-with-hole ring face AND the inner face.
        let points = vec![
            // outer square
            DVec2::new(-2.0, -2.0),
            DVec2::new(2.0, -2.0),
            DVec2::new(2.0, 2.0),
            DVec2::new(-2.0, 2.0),
            // inner square
            DVec2::new(-1.0, -1.0),
            DVec2::new(1.0, -1.0),
            DVec2::new(1.0, 1.0),
            DVec2::new(-1.0, 1.0),
        ];
        let mut edges = loop_edges(&[0, 1, 2, 3]);
        edges.extend(loop_edges(&[4, 5, 6, 7]));

        let faces = detect_faces(&points, &edges);
        assert_eq!(faces.len(), 2);

        let outer_face = faces.iter().find(|f| f.outer.contains(&0)).unwrap();
        assert_eq!(outer_face.holes.len(), 1);
        assert!(outer_face.holes[0].contains(&4));

        let inner_face = faces.iter().find(|f| f.outer.contains(&4)).unwrap();
        assert_eq!(inner_face.holes.len(), 0);
    }
}
