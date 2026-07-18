import * as THREE from "three";

/// Mirrors src-tauri/src/geometry/inset.rs's `offset_polygon` - a per-edge
/// miter inset used here only to draw the live preview loop while dragging
/// the Inset tool. The backend command re-derives (and authoritatively
/// validates) the real result; this copy only needs to look right, not be
/// the source of truth.
export function offsetPolygon2D(points: THREE.Vector2[], distance: number): THREE.Vector2[] | null {
  const n = points.length;
  if (n < 3) return null;
  if (Math.abs(distance) < 1e-9) return points.slice();

  const offsetLines: { point: THREE.Vector2; dir: THREE.Vector2 }[] = [];
  for (let i = 0; i < n; i++) {
    const a = points[i];
    const b = points[(i + 1) % n];
    const edge = new THREE.Vector2().subVectors(b, a);
    const len = edge.length();
    if (len < 1e-9) return null;
    const dir = edge.clone().divideScalar(len);
    const inwardNormal = new THREE.Vector2(-dir.y, dir.x);
    offsetLines.push({ point: a.clone().addScaledVector(inwardNormal, distance), dir });
  }

  const result: THREE.Vector2[] = [];
  for (let i = 0; i < n; i++) {
    const prev = offsetLines[(i + n - 1) % n];
    const curr = offsetLines[i];
    result.push(lineIntersection(prev.point, prev.dir, curr.point, curr.dir) ?? curr.point.clone());
  }

  for (let i = 0; i < n; i++) {
    const dir = offsetLines[i].dir;
    const newEdge = new THREE.Vector2().subVectors(result[(i + 1) % n], result[i]);
    if (newEdge.dot(dir) <= 1e-9) return null;
  }

  const originalArea = signedArea(points);
  const resultArea = signedArea(result);
  if (Math.abs(resultArea) < 1e-9 || Math.sign(originalArea) !== Math.sign(resultArea)) return null;

  return result;
}

function lineIntersection(
  p1: THREE.Vector2,
  d1: THREE.Vector2,
  p2: THREE.Vector2,
  d2: THREE.Vector2,
): THREE.Vector2 | null {
  const denom = d1.x * d2.y - d1.y * d2.x;
  if (Math.abs(denom) < 1e-12) return null;
  const diffX = p2.x - p1.x;
  const diffY = p2.y - p1.y;
  const t = (diffX * d2.y - diffY * d2.x) / denom;
  return p1.clone().addScaledVector(d1, t);
}

function signedArea(points: THREE.Vector2[]): number {
  const n = points.length;
  let area = 0;
  for (let i = 0; i < n; i++) {
    const a = points[i];
    const b = points[(i + 1) % n];
    area += a.x * b.y - b.x * a.y;
  }
  return area * 0.5;
}
