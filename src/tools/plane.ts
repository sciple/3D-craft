import * as THREE from "three";
import { documentStore, faceIdKey } from "../state/document-store";
import type { FaceId } from "../state/document-store";
import type { ToolContext } from "./types";
import { pointerToNdc, raycastGroundPlane } from "./types";

/// An oriented sketch plane a draw tool is currently working on: either the
/// ground plane (Z=0, the v1 default) or an existing solid face's own plane
/// when the user starts drawing directly on top of it. `faceId`, when
/// present, is threaded through to the backend's `target_face_id` so the new
/// geometry splits just that face instead of triggering a document-wide
/// coplanar resplit.
export interface SketchPlane {
  origin: THREE.Vector3;
  normal: THREE.Vector3;
  u: THREE.Vector3;
  v: THREE.Vector3;
  faceId: FaceId | null;
}

/// Exact TS port of src-tauri/src/geometry/plane.rs's `Plane::from_normal` -
/// the basis is a pure function of `normal` alone (translation-independent),
/// so it must match the backend's construction precisely, or a shape's
/// on-screen preview (built client-side from this basis) would disagree
/// with what the backend actually commits.
export function planeFromNormal(origin: THREE.Vector3, normal: THREE.Vector3, faceId: FaceId | null = null): SketchPlane {
  const n = normal.clone().normalize();
  const reference = Math.abs(n.x) < 0.9 ? new THREE.Vector3(1, 0, 0) : new THREE.Vector3(0, 1, 0);
  const v = new THREE.Vector3().crossVectors(n, reference).normalize();
  const u = new THREE.Vector3().crossVectors(v, n);
  return { origin: origin.clone(), normal: n, u, v, faceId };
}

export const GROUND_SKETCH_PLANE = planeFromNormal(new THREE.Vector3(0, 0, 0), new THREE.Vector3(0, 0, 1));

export function to2d(plane: SketchPlane, p: THREE.Vector3): THREE.Vector2 {
  const d = p.clone().sub(plane.origin);
  return new THREE.Vector2(d.dot(plane.u), d.dot(plane.v));
}

export function to3d(plane: SketchPlane, p: THREE.Vector2): THREE.Vector3 {
  return plane.origin.clone().addScaledVector(plane.u, p.x).addScaledVector(plane.v, p.y);
}

export function toThreePlane(plane: SketchPlane): THREE.Plane {
  return new THREE.Plane().setFromNormalAndCoplanarPoint(plane.normal, plane.origin);
}

/// Resolves what a draw tool's first click should sketch on: a hit against
/// existing geometry sketches directly on that face's own plane (origin =
/// the raycast hit point, which by construction lies on the face); a miss
/// falls back to the ground plane - mirroring the backend's `resplit`
/// fallback for when a target face id turns out to be stale.
export function resolveSketchTarget(e: PointerEvent, ctx: ToolContext, raycaster: THREE.Raycaster): { plane: SketchPlane; point: THREE.Vector3 } | null {
  const ndc = pointerToNdc(e, ctx.domElement);
  raycaster.setFromCamera(ndc, ctx.camera);
  const hits = raycaster.intersectObject(ctx.meshRenderer.mesh);
  const hit = hits[0];
  if (hit && hit.faceIndex != null) {
    const faceId = ctx.meshRenderer.faceIdForTriangle(hit.faceIndex);
    if (faceId) {
      const face = documentStore.getSnapshot().faces.find((f) => faceIdKey(f.id) === faceIdKey(faceId));
      if (face) {
        return { plane: planeFromNormal(hit.point, new THREE.Vector3(...face.normal), faceId), point: hit.point.clone() };
      }
    }
  }
  const groundPoint = raycastGroundPlane(e, ctx, raycaster);
  if (!groundPoint) return null;
  return { plane: GROUND_SKETCH_PLANE, point: groundPoint };
}
