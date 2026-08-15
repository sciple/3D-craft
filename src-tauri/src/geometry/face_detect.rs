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
    let edges = split_edges_at_interior_points(points, edges);
    let loops = trace_ccw_loops(points, &edges);
    build_face_forest(points, &loops)
}

/// Splits any edge that another point lands on partway along its length (a
/// T-junction) into two edges meeting at that point. Without this, a new
/// shape sharing part of an edge with the face it's drawn on - most
/// basically, a rectangle sketched in the corner of a larger rectangular
/// face, the "stud built into the corner of a box's top face" workflow -
/// leaves the existing long edge and the new short edge exactly collinear
/// and overlapping for part of their length: `trace_ccw_loops`'s
/// neighbor-angle sort sees two outgoing edges from the shared vertex
/// pointing in the exact same direction with no tiebreaker between them, and
/// even with one, the graph has no representation for "this edge continues
/// past the new point" without an actual vertex there. Matches SketchUp's
/// own "sticky geometry": drawing a line that touches an existing edge
/// splits that edge at the touch point.
fn split_edges_at_interior_points(points: &[DVec2], edges: &[(usize, usize)]) -> Vec<(usize, usize)> {
    // Any point genuinely closer than this to an edge's own endpoint would
    // already have been merged into that endpoint's vertex by the caller's
    // own position weld (see `Mesh::WELD_TOLERANCE`, the same magnitude),
    // so only points meaningfully into an edge's interior reach this check.
    const ON_SEGMENT_TOLERANCE: f64 = 1e-3;

    let mut result = Vec::with_capacity(edges.len());
    for &(a, b) in edges {
        if a == b {
            continue;
        }
        let pa = points[a];
        let pb = points[b];
        let ab = pb - pa;
        let len_sq = ab.length_squared();

        let mut interior: Vec<(usize, f64)> = if len_sq > 1e-18 {
            points
                .iter()
                .enumerate()
                .filter(|&(i, _)| i != a && i != b)
                .filter_map(|(i, &p)| {
                    let t = (p - pa).dot(ab) / len_sq;
                    if !(1e-9..=1.0 - 1e-9).contains(&t) {
                        return None;
                    }
                    let closest = pa + ab * t;
                    ((p - closest).length_squared() < ON_SEGMENT_TOLERANCE * ON_SEGMENT_TOLERANCE).then_some((i, t))
                })
                .collect()
        } else {
            Vec::new()
        };

        if interior.is_empty() {
            result.push((a, b));
            continue;
        }
        interior.sort_by(|x, y| x.1.partial_cmp(&y.1).unwrap());
        let mut prev = a;
        for (idx, _) in interior {
            result.push((prev, idx));
            prev = idx;
        }
        result.push((prev, b));
    }
    result
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
    for face in &mut faces {
        face.holes = merge_holes_sharing_an_edge(std::mem::take(&mut face.holes));
    }
    faces
}

/// Fuses any two of a face's hole loops that run along a shared edge into
/// the single loop tracing the outside of their union.
///
/// Two *regions* adjacent along an edge are correctly traced as two separate
/// cycles - they really are two regions, and each may independently become
/// its own face or stay empty. But as *holes of the face around them* they
/// describe one connected area the face doesn't cover, and describing it with
/// two loops is wrong data, not merely an unusual encoding: `triangulate_face`
/// bridges each hole to the nearest polygon vertex, so the second bridge
/// lands on a vertex the first hole already contributed (distance zero),
/// producing a self-touching polygon that ear-clipping cannot resolve. It
/// bails out with a partial triangulation, whose edges then fail to
/// reconstruct the real boundary - reported downstream as open edges by
/// `check_manifold`, and rendered/exported with a chunk of the face missing.
/// `pushpull` would likewise raise one wall per loop and duplicate the shared
/// edge, so this is fixed here, once, rather than in each consumer.
///
/// Reachable as soon as a sketch can be drawn flush against an existing
/// stud's rim (see `split_edges_at_interior_points`, which is what lets that
/// alignment survive face detection at all).
///
/// Deliberately handles only edge-sharing, not loops meeting at a single
/// vertex: those pinch to a figure-eight that no single simple loop
/// represents, and no draw tool here produces them without also sharing an
/// edge.
fn merge_holes_sharing_an_edge(mut holes: Vec<LoopIndices>) -> Vec<LoopIndices> {
    'restart: loop {
        for x in 0..holes.len() {
            for y in (x + 1)..holes.len() {
                let Some(merged) = merge_two_loops_on_shared_edge(&holes[x], &holes[y]) else {
                    continue;
                };
                holes.remove(y); // y > x, so removing it first keeps x valid.
                holes[x] = merged;
                continue 'restart;
            }
        }
        return holes;
    }
}

/// Joins `first` and `second` along the first directed edge they share in
/// opposite directions (`first` traverses `a -> b`, `second` traverses
/// `b -> a`), dropping that now-interior edge. Returns `None` when they share
/// no such edge. Both loops are wound the same way, so the result is too.
fn merge_two_loops_on_shared_edge(first: &[usize], second: &[usize]) -> Option<LoopIndices> {
    let (n, m) = (first.len(), second.len());
    for i in 0..n {
        let (a, b) = (first[i], first[(i + 1) % n]);
        for j in 0..m {
            if second[j] != b || second[(j + 1) % m] != a {
                continue;
            }
            // Walk `first` up to `a`, detour the whole of `second` the long
            // way round from `a` back to `b` (i.e. every vertex except the
            // two ends of the shared edge), then resume `first` at `b`.
            let mut merged = Vec::with_capacity(n + m - 2);
            merged.extend(first[..=i].iter().copied());
            merged.extend((2..m).map(|k| second[(j + k) % m]));
            merged.extend(first[i + 1..].iter().copied());
            return Some(merged);
        }
    }
    None
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

/// Distance-tolerant point-in-polygon test: true if `p` is inside `polygon`
/// or within `tolerance` of its boundary. Used to validate that a
/// freshly-drawn loop actually belongs to the face it's about to be
/// resplit against - see `Document::resplit_face_with_loops`, which rejects
/// a loop that fails this rather than handing `detect_faces` a loop whose
/// edges cross the target face's own boundary instead of merely touching it
/// at shared vertices (something the half-edge tracing above has no
/// representation for, and silently corrupts on).
pub(crate) fn point_in_or_near_polygon(p: DVec2, polygon: &[DVec2], tolerance: f64) -> bool {
    let n = polygon.len();
    let mut inside = false;
    let mut j = n - 1;
    for i in 0..n {
        let pi = polygon[i];
        let pj = polygon[j];
        if ((pi.y > p.y) != (pj.y > p.y)) && (p.x < (pj.x - pi.x) * (p.y - pi.y) / (pj.y - pi.y) + pi.x) {
            inside = !inside;
        }
        j = i;
    }
    if inside {
        return true;
    }
    (0..n).any(|i| distance_to_segment(p, polygon[i], polygon[(i + 1) % n]) < tolerance)
}

fn distance_to_segment(p: DVec2, a: DVec2, b: DVec2) -> f64 {
    let ab = b - a;
    let len_sq = ab.length_squared();
    if len_sq < 1e-18 {
        return (p - a).length();
    }
    let t = ((p - a).dot(ab) / len_sq).clamp(0.0, 1.0);
    (p - (a + ab * t)).length()
}

/// True if `p` lies within `tolerance` of any EDGE of `loop_points` (a closed
/// ring) - unlike `point_in_or_near_polygon`, does not also match points
/// merely inside the polygon's interior. Used by `Document::draw_line_segment`
/// to require a chord's endpoints land ON the target face's own boundary (its
/// outer ring or a hole ring), not just anywhere inside it: the latter is
/// what `point_in_or_near_polygon` allows and is correct for a closed shape
/// sketched inside a face, but a chord that doesn't terminate on the boundary
/// can't actually bisect the face into two closed regions.
pub(crate) fn point_near_boundary(p: DVec2, loop_points: &[DVec2], tolerance: f64) -> bool {
    let n = loop_points.len();
    (0..n).any(|i| distance_to_segment(p, loop_points[i], loop_points[(i + 1) % n]) < tolerance)
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
    fn point_near_boundary_matches_edges_not_the_interior() {
        let square = vec![
            DVec2::new(0.0, 0.0),
            DVec2::new(10.0, 0.0),
            DVec2::new(10.0, 10.0),
            DVec2::new(0.0, 10.0),
        ];
        assert!(point_near_boundary(DVec2::new(5.0, 0.0), &square, 1e-2), "midpoint of an edge should match");
        assert!(point_near_boundary(DVec2::new(0.0, 0.0), &square, 1e-2), "a corner should match");
        assert!(!point_near_boundary(DVec2::new(5.0, 5.0), &square, 1e-2), "the center is inside, not near any edge");
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
    fn a_rectangle_sharing_a_corner_with_a_larger_one_splits_it_without_a_tie() {
        // The corner-stud workflow: a smaller rectangle drawn flush into the
        // corner of a larger one. Point 4 lands partway along edge 0-1 and
        // point 6 lands partway along edge 3-0 - both T-junctions - so
        // without `split_edges_at_interior_points`, vertex 0 would have two
        // pairs of exactly-collinear outgoing edges (to 1 and 4, to 3 and 6)
        // that the neighbor-angle sort has no tiebreaker for.
        let points = vec![
            DVec2::new(0.0, 0.0),   // 0: shared corner
            DVec2::new(10.0, 0.0),  // 1
            DVec2::new(10.0, 10.0), // 2
            DVec2::new(0.0, 10.0),  // 3
            DVec2::new(5.0, 0.0),   // 4: on segment 0-1
            DVec2::new(5.0, 5.0),   // 5
            DVec2::new(0.0, 5.0),   // 6: on segment 3-0
        ];
        let mut edges = loop_edges(&[0, 1, 2, 3]);
        edges.extend(loop_edges(&[0, 4, 5, 6]));

        let faces = detect_faces(&points, &edges);

        assert_eq!(faces.len(), 2, "the corner-stud footprint and the remaining L-shaped area");
        assert!(faces.iter().all(|f| f.holes.is_empty()), "a corner-shared split must not nest as a hole");

        let inner = faces.iter().find(|f| f.outer.contains(&4)).unwrap();
        assert!((polygon_area(&points, &inner.outer).abs() - 25.0).abs() < 1e-9);

        let outer = faces.iter().find(|f| f.outer.contains(&2)).unwrap();
        assert!((polygon_area(&points, &outer.outer).abs() - 75.0).abs() < 1e-9);
    }

    #[test]
    fn a_point_partway_along_an_edge_splits_it_even_with_no_shared_vertex() {
        // A slot notch: both of the new loop's touching points land strictly
        // inside an existing edge's interior, with no shared vertex at all.
        let points = vec![
            DVec2::new(0.0, 0.0),  // 0
            DVec2::new(10.0, 0.0), // 1
            DVec2::new(10.0, 5.0), // 2
            DVec2::new(0.0, 5.0),  // 3
            DVec2::new(3.0, 0.0),  // 4: on segment 0-1
            DVec2::new(7.0, 0.0),  // 5: on segment 0-1
            DVec2::new(7.0, 2.0),  // 6
            DVec2::new(3.0, 2.0),  // 7
        ];
        let mut edges = loop_edges(&[0, 1, 2, 3]);
        edges.extend(loop_edges(&[4, 5, 6, 7]));

        let faces = detect_faces(&points, &edges);

        assert_eq!(faces.len(), 2);
        assert!(faces.iter().all(|f| f.holes.is_empty()));
        // The outer U-shape's own trace hugs three of the notch's four
        // walls (it detours up and around rather than passing straight
        // through), so every one of the notch's vertices - including 6 and
        // 7 - is also referenced by the outer loop; only the notch's own
        // loop length (4 vs. 8) tells them apart.
        let notch = faces.iter().find(|f| f.outer.len() == 4).unwrap();
        assert!((polygon_area(&points, &notch.outer).abs() - 8.0).abs() < 1e-9);
        let outer = faces.iter().find(|f| f.outer.len() == 8).unwrap();
        assert!((polygon_area(&points, &outer.outer).abs() - 42.0).abs() < 1e-9);
    }

    #[test]
    fn two_adjacent_inner_regions_become_one_merged_hole_not_two() {
        // Two rectangles sharing a full edge, both sitting inside a larger
        // square. Each is legitimately its own region (and its own face),
        // but as holes of the square around them they describe a single
        // connected uncovered area - and must be reported as one loop.
        // Two separate loops sharing an edge is data `triangulate_face`'s
        // hole-bridging cannot represent: the second bridge lands on a
        // vertex the first hole already contributed, ear-clipping bails out,
        // and the face comes back partially triangulated (open edges
        // downstream, a visible chunk missing when rendered/exported).
        let points = vec![
            DVec2::new(0.0, 0.0),   // 0  outer
            DVec2::new(20.0, 0.0),  // 1
            DVec2::new(20.0, 20.0), // 2
            DVec2::new(0.0, 20.0),  // 3
            DVec2::new(5.0, 5.0),   // 4  left rectangle
            DVec2::new(10.0, 5.0),  // 5  shared edge, low
            DVec2::new(10.0, 15.0), // 6  shared edge, high
            DVec2::new(5.0, 15.0),  // 7
            DVec2::new(15.0, 5.0),  // 8  right rectangle
            DVec2::new(15.0, 15.0), // 9
        ];
        let mut edges = loop_edges(&[0, 1, 2, 3]);
        edges.extend(loop_edges(&[4, 5, 6, 7]));
        edges.extend(loop_edges(&[5, 8, 9, 6]));

        let faces = detect_faces(&points, &edges);

        assert_eq!(faces.len(), 3, "both inner rectangles plus the square around them");
        let outer_face = faces.iter().find(|f| f.outer.contains(&0)).unwrap();
        assert_eq!(outer_face.holes.len(), 1, "the two adjacent regions must fuse into a single hole loop");
        let hole_area = polygon_area(&points, &outer_face.holes[0]).abs();
        assert!(
            (hole_area - 100.0).abs() < 1e-9,
            "the merged hole must trace the union of both rectangles (10x10), got {hole_area}"
        );
        assert!(hole_area > 0.0);
        // The shared edge's endpoints survive as collinear vertices on the
        // union's boundary, so the merged loop is a hexagon, not a quad.
        assert_eq!(outer_face.holes[0].len(), 6);
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
