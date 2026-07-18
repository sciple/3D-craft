import * as THREE from "three";

/// Standard "drag along a 3D axis with a 2D mouse" technique used by
/// translate gizmos: the closest point on (axisOrigin, axisDir) to the
/// current mouse ray, expressed as a signed distance along axisDir from
/// axisOrigin. Used by both the Push/Pull and Inset tools so the preview
/// tracks the mouse naturally regardless of camera angle, instead of a raw
/// screen-space delta.
export function closestDistanceAlongAxis(
  axisOrigin: THREE.Vector3,
  axisDir: THREE.Vector3,
  rayOrigin: THREE.Vector3,
  rayDir: THREE.Vector3,
): number {
  const w = new THREE.Vector3().subVectors(axisOrigin, rayOrigin);
  const a = axisDir.dot(axisDir);
  const b = axisDir.dot(rayDir);
  const c = rayDir.dot(rayDir);
  const dd = axisDir.dot(w);
  const ee = rayDir.dot(w);
  const denom = a * c - b * b;
  if (Math.abs(denom) < 1e-9) return 0; // mouse ray parallel to the drag axis
  return (b * ee - c * dd) / denom;
}
