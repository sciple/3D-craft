use glam::DVec2;

/// Computes a miter (straight-skeleton-style) offset of a closed 2D polygon
/// loop. `distance` is measured along each edge's *inward* normal (left of
/// the edge direction for a CCW loop, matching the winding convention used
/// throughout this crate) - so a positive distance shrinks a CCW outer loop
/// toward its own interior.
///
/// This is a per-edge miter join, not a true straight skeleton: it handles
/// convex and mildly concave polygons correctly (which covers every shape
/// this app's primitives and polygon tool can produce) but can produce
/// self-intersecting output for sharply concave shapes offset past their
/// local feature size. Returns `None` if the offset is degenerate (fewer
/// than 3 points, a zero-length edge, or an offset large enough to collapse
/// or invert the polygon) rather than returning that garbage.
pub fn offset_polygon(points: &[DVec2], distance: f64) -> Option<Vec<DVec2>> {
    let n = points.len();
    if n < 3 {
        return None;
    }
    if distance.abs() < 1e-12 {
        return Some(points.to_vec());
    }

    // For each edge (points[i] -> points[i+1]), the line it lies on after
    // being pushed `distance` along its inward normal.
    let mut offset_lines: Vec<(DVec2, DVec2)> = Vec::with_capacity(n);
    for i in 0..n {
        let a = points[i];
        let b = points[(i + 1) % n];
        let edge = b - a;
        let len = edge.length();
        if len < 1e-9 {
            return None;
        }
        let dir = edge / len;
        let inward_normal = DVec2::new(-dir.y, dir.x);
        offset_lines.push((a + inward_normal * distance, dir));
    }

    // Each new vertex is the intersection of the two offset lines meeting at
    // the corresponding original vertex.
    let mut result = Vec::with_capacity(n);
    for i in 0..n {
        let (p1, d1) = offset_lines[(i + n - 1) % n];
        let (p2, d2) = offset_lines[i];
        result.push(line_intersection(p1, d1, p2, d2).unwrap_or(p2));
    }

    // Each new edge must still point the same way along its offset line as
    // the original edge it came from. A too-large offset makes the two
    // vertices bounding an edge cross past each other on that line - the
    // edge effectively inverts - which for symmetric shapes (e.g. a square
    // offset past its half-width) can otherwise leave the *overall* signed
    // area positive and looking deceptively valid, even though the polygon
    // has folded through itself. This per-edge check catches that case; the
    // overall-area check below catches the rest (e.g. total collapse).
    for i in 0..n {
        let (_, dir) = offset_lines[i];
        let new_edge = result[(i + 1) % n] - result[i];
        if new_edge.dot(dir) <= 1e-9 {
            return None;
        }
    }

    let original_area = signed_area(points);
    let result_area = signed_area(&result);
    if result_area.abs() < 1e-9 || original_area.signum() != result_area.signum() {
        // The offset collapsed the polygon to nothing or flipped its
        // winding - too large for this shape's local feature size.
        return None;
    }

    Some(result)
}

fn line_intersection(p1: DVec2, d1: DVec2, p2: DVec2, d2: DVec2) -> Option<DVec2> {
    let denom = d1.x * d2.y - d1.y * d2.x;
    if denom.abs() < 1e-12 {
        return None; // parallel edges meeting at a straight-through vertex
    }
    let diff = p2 - p1;
    let t = (diff.x * d2.y - diff.y * d2.x) / denom;
    Some(p1 + d1 * t)
}

fn signed_area(points: &[DVec2]) -> f64 {
    let n = points.len();
    let mut area = 0.0;
    for i in 0..n {
        let a = points[i];
        let b = points[(i + 1) % n];
        area += a.x * b.y - b.x * a.y;
    }
    area * 0.5
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shrinks_a_unit_square_inward_by_the_given_distance() {
        let square = vec![DVec2::new(0.0, 0.0), DVec2::new(1.0, 0.0), DVec2::new(1.0, 1.0), DVec2::new(0.0, 1.0)];
        let inset = offset_polygon(&square, 0.25).unwrap();
        for p in &inset {
            assert!(p.x > 0.2 && p.x < 0.8, "x={}", p.x);
            assert!(p.y > 0.2 && p.y < 0.8, "y={}", p.y);
        }
        // A centered square inset by 0.25 on every side should end up a
        // 0.5x0.5 square, area 0.25.
        assert!((signed_area(&inset).abs() - 0.25).abs() < 1e-9);
    }

    #[test]
    fn offset_larger_than_half_the_shape_is_rejected() {
        let square = vec![DVec2::new(0.0, 0.0), DVec2::new(1.0, 0.0), DVec2::new(1.0, 1.0), DVec2::new(0.0, 1.0)];
        assert!(offset_polygon(&square, 0.6).is_none());
    }

    #[test]
    fn negative_distance_grows_the_polygon_outward() {
        let square = vec![DVec2::new(0.0, 0.0), DVec2::new(1.0, 0.0), DVec2::new(1.0, 1.0), DVec2::new(0.0, 1.0)];
        let grown = offset_polygon(&square, -0.5).unwrap();
        assert!((signed_area(&grown).abs() - 4.0).abs() < 1e-9);
    }
}
