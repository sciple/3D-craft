import * as THREE from "three";
import type { DocumentSnapshot, FaceId } from "../state/document-store";

export interface ScreenRect {
  minX: number;
  minY: number;
  maxX: number;
  maxY: number;
}

/// Faces "crossed" by a screen-space rectangle: any face with at least one
/// outer/hole-loop vertex projecting inside `rect` (NDC space, matching
/// `pointerToNdc`'s convention). Crossing-style selection, not
/// window/full-enclosure - a face only needs to be touched, not wholly
/// contained. No occlusion/visibility test: a face behind other geometry is
/// still selectable if its projection lands in the rectangle - this is
/// intentional (simplest correct implementation), not a bug.
export function facesInRect(
  snapshot: DocumentSnapshot,
  rect: ScreenRect,
  camera: THREE.PerspectiveCamera,
): FaceId[] {
  const camPos = camera.position;
  const camForward = new THREE.Vector3();
  camera.getWorldDirection(camForward);

  const v = new THREE.Vector3();
  const result: FaceId[] = [];

  for (const face of snapshot.faces) {
    let inside = false;
    const indices = face.holes.length > 0 ? [...face.outer, ...face.holes.flat()] : face.outer;
    for (const idx of indices) {
      const p = snapshot.vertices[idx];
      v.set(p[0], p[1], p[2]);
      // Skip vertices behind the camera - NDC projection folds them back
      // into [-1, 1] via a negative w-divide, which would otherwise read
      // as a false "inside" hit.
      if (v.clone().sub(camPos).dot(camForward) <= 0) continue;
      v.project(camera);
      if (v.x >= rect.minX && v.x <= rect.maxX && v.y >= rect.minY && v.y <= rect.maxY) {
        inside = true;
        break;
      }
    }
    if (inside) result.push(face.id);
  }

  return result;
}
