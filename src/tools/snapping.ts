import * as THREE from "three";
import { documentStore } from "../state/document-store";
import type { DocumentSnapshot } from "../state/document-store";
import type { ToolContext } from "./types";
import { pointerToNdc, GROUND_PLANE } from "./types";
import type { SketchPlane } from "./plane";
import { toThreePlane } from "./plane";

export type SnapKind = "endpoint" | "midpoint" | "edge";

export interface SnapResult {
  point: THREE.Vector3;
  kind: SnapKind;
}

const SNAP_PIXELS = 12;
const PLANE_EPS = 1e-4;

/// Finds the nearest snap candidate among vertices/edges of existing
/// geometry that lie on `plane` (the sketch plane the active draw tool is
/// currently working on - the ground plane by default, or a solid face's
/// own plane when drawing directly on top of it) within a tolerance sized
/// in world units to read as a constant ~12px on screen regardless of zoom.
/// Priority matches SketchUp's inference order: an endpoint within
/// tolerance always wins over a midpoint or edge point even if the
/// midpoint/edge point is numerically closer to the cursor, since endpoints
/// are the more useful target.
export function findPlanarSnap(
  snapshot: DocumentSnapshot,
  planePoint: THREE.Vector3,
  plane: SketchPlane,
  camera: THREE.PerspectiveCamera,
  domElement: HTMLElement,
): SnapResult | null {
  const rect = domElement.getBoundingClientRect();
  const distance = Math.max(0.001, camera.position.distanceTo(planePoint));
  const worldPerPixel = (2 * distance * Math.tan((camera.fov * Math.PI) / 360)) / rect.height;
  const tolerance = SNAP_PIXELS * worldPerPixel;

  const positionOf = (i: number) => new THREE.Vector3(...snapshot.vertices[i]);
  const onPlane = (i: number) => Math.abs(positionOf(i).sub(plane.origin).dot(plane.normal)) < PLANE_EPS;

  const nearestWithin = (points: THREE.Vector3[], kind: SnapKind): SnapResult | null => {
    let best: SnapResult | null = null;
    let bestDist = tolerance;
    for (const p of points) {
      const d = p.distanceTo(planePoint);
      if (d < bestDist) {
        bestDist = d;
        best = { point: p, kind };
      }
    }
    return best;
  };

  const endpoints: THREE.Vector3[] = [];
  for (let i = 0; i < snapshot.vertices.length; i++) {
    if (onPlane(i)) endpoints.push(positionOf(i));
  }
  const endpointHit = nearestWithin(endpoints, "endpoint");
  if (endpointHit) return endpointHit;

  const midpoints: THREE.Vector3[] = [];
  const edgePoints: THREE.Vector3[] = [];
  for (const face of snapshot.faces) {
    for (const loop of [face.outer, ...face.holes]) {
      for (let i = 0; i < loop.length; i++) {
        const a = loop[i];
        const b = loop[(i + 1) % loop.length];
        if (!onPlane(a) || !onPlane(b)) continue;
        const pa = positionOf(a);
        const pb = positionOf(b);
        midpoints.push(pa.clone().add(pb).multiplyScalar(0.5));
        edgePoints.push(closestPointOnSegment(planePoint, pa, pb));
      }
    }
  }
  const midpointHit = nearestWithin(midpoints, "midpoint");
  if (midpointHit) return midpointHit;

  return nearestWithin(edgePoints, "edge");
}

function closestPointOnSegment(p: THREE.Vector3, a: THREE.Vector3, b: THREE.Vector3): THREE.Vector3 {
  const ab = new THREE.Vector3().subVectors(b, a);
  const lenSq = ab.lengthSq();
  if (lenSq < 1e-12) return a.clone();
  const t = THREE.MathUtils.clamp(new THREE.Vector3().subVectors(p, a).dot(ab) / lenSq, 0, 1);
  return a.clone().addScaledVector(ab, t);
}

/// Combines a raycast against `plane` with `findPlanarSnap` - the one call
/// draw tools need on every pointer event once a sketch plane is locked in.
/// Falls back to the raw raycast point when nothing is within snap
/// tolerance.
export function raycastPlaneSnapped(
  e: PointerEvent,
  ctx: ToolContext,
  raycaster: THREE.Raycaster,
  plane: SketchPlane,
): { point: THREE.Vector3; snap: SnapResult | null } | null {
  const ndc = pointerToNdc(e, ctx.domElement);
  raycaster.setFromCamera(ndc, ctx.camera);
  const raw = new THREE.Vector3();
  if (!raycaster.ray.intersectPlane(toThreePlane(plane), raw)) return null;
  const snap = findPlanarSnap(documentStore.getSnapshot(), raw, plane, ctx.camera, ctx.domElement);
  return { point: snap ? snap.point : raw, snap };
}

/// Screen-space snap for tools that pick anywhere in the scene (e.g. the tape
/// measure), rather than being constrained to a single sketch plane like
/// `findPlanarSnap`. Every candidate vertex/midpoint/edge point is projected
/// to the viewport and compared to the cursor in pixels, so the ~12px
/// tolerance reads the same at any zoom. Priority matches the planar snap:
/// endpoint beats midpoint beats along-edge. Returns null when nothing is
/// within tolerance.
export function findSnap3d(
  snapshot: DocumentSnapshot,
  cursorPx: THREE.Vector2,
  camera: THREE.PerspectiveCamera,
  domElement: HTMLElement,
  ray: THREE.Ray,
): SnapResult | null {
  const rect = domElement.getBoundingClientRect();
  const toPixels = (p: THREE.Vector3): THREE.Vector2 | null => {
    const n = p.clone().project(camera);
    if (n.z <= -1 || n.z >= 1) return null; // behind the camera / outside the depth range
    return new THREE.Vector2((n.x * 0.5 + 0.5) * rect.width, (-n.y * 0.5 + 0.5) * rect.height);
  };
  const nearestWithin = (points: THREE.Vector3[], kind: SnapKind): SnapResult | null => {
    let best: SnapResult | null = null;
    let bestDist = SNAP_PIXELS;
    for (const p of points) {
      const px = toPixels(p);
      if (!px) continue;
      const d = px.distanceTo(cursorPx);
      if (d < bestDist) {
        bestDist = d;
        best = { point: p.clone(), kind };
      }
    }
    return best;
  };

  const positionOf = (i: number) => new THREE.Vector3(...snapshot.vertices[i]);

  const endpoints = snapshot.vertices.map((_, i) => positionOf(i));
  const endpointHit = nearestWithin(endpoints, "endpoint");
  if (endpointHit) return endpointHit;

  const midpoints: THREE.Vector3[] = [];
  const edgePoints: THREE.Vector3[] = [];
  for (const face of snapshot.faces) {
    for (const loop of [face.outer, ...face.holes]) {
      for (let i = 0; i < loop.length; i++) {
        const pa = positionOf(loop[i]);
        const pb = positionOf(loop[(i + 1) % loop.length]);
        midpoints.push(pa.clone().add(pb).multiplyScalar(0.5));
        edgePoints.push(closestPointOnSegmentToRay(pa, pb, ray));
      }
    }
  }
  const midpointHit = nearestWithin(midpoints, "midpoint");
  if (midpointHit) return midpointHit;

  return nearestWithin(edgePoints, "edge");
}

/// Closest point on segment [a,b] to the (infinite) pointer ray - the 3D
/// analogue of `closestPointOnSegment`, used for "along an edge" snapping when
/// there's no sketch plane to project onto.
function closestPointOnSegmentToRay(a: THREE.Vector3, b: THREE.Vector3, ray: THREE.Ray): THREE.Vector3 {
  const u = new THREE.Vector3().subVectors(b, a);
  const w0 = new THREE.Vector3().subVectors(a, ray.origin);
  const A = u.dot(u);
  const B = u.dot(ray.direction);
  const C = ray.direction.dot(ray.direction); // 1 for a normalized ray, kept general
  const D = u.dot(w0);
  const E = ray.direction.dot(w0);
  const denom = A * C - B * B;
  const s = THREE.MathUtils.clamp(denom < 1e-9 ? 0 : (B * E - C * D) / denom, 0, 1);
  return a.clone().addScaledVector(u, s);
}

/// Full pick for a 3D measuring tool: snap to nearby geometry if within
/// tolerance, otherwise a free point on the hovered surface, otherwise the
/// ground plane. Returns null only when the ray hits nothing at all.
export function raycastSnapped3d(
  e: PointerEvent,
  ctx: ToolContext,
  raycaster: THREE.Raycaster,
): { point: THREE.Vector3; snap: SnapResult | null } | null {
  const rect = ctx.domElement.getBoundingClientRect();
  const cursorPx = new THREE.Vector2(e.clientX - rect.left, e.clientY - rect.top);
  raycaster.setFromCamera(pointerToNdc(e, ctx.domElement), ctx.camera);

  const snap = findSnap3d(documentStore.getSnapshot(), cursorPx, ctx.camera, ctx.domElement, raycaster.ray);
  if (snap) return { point: snap.point, snap };

  const surfaceHit = raycaster.intersectObject(ctx.meshRenderer.mesh)[0];
  if (surfaceHit) return { point: surfaceHit.point.clone(), snap: null };

  const ground = new THREE.Vector3();
  if (raycaster.ray.intersectPlane(GROUND_PLANE, ground)) return { point: ground, snap: null };
  return null;
}

const SNAP_COLORS: Record<SnapKind, number> = {
  endpoint: 0x2ec4ff,
  midpoint: 0x33ff88,
  edge: 0xffd633,
};

/// A small on-screen marker shown at the active snap candidate while
/// drawing, color-coded by kind (endpoint/midpoint/edge) - the "simple
/// on-screen cue" this app's snapping needs, short of SketchUp's full
/// inference-line system.
export class SnapIndicator {
  private mesh: THREE.Mesh | null = null;

  update(scene: THREE.Scene, camera: THREE.PerspectiveCamera, result: SnapResult | null) {
    if (!result) {
      this.hide();
      return;
    }
    const size = camera.position.distanceTo(result.point) * 0.012;
    if (!this.mesh) {
      const geometry = new THREE.SphereGeometry(1, 8, 8);
      const material = new THREE.MeshBasicMaterial({ depthTest: false });
      this.mesh = new THREE.Mesh(geometry, material);
      this.mesh.renderOrder = 999;
      scene.add(this.mesh);
    }
    this.mesh.position.copy(result.point);
    this.mesh.scale.setScalar(size);
    (this.mesh.material as THREE.MeshBasicMaterial).color.setHex(SNAP_COLORS[result.kind]);
  }

  hide() {
    if (!this.mesh) return;
    this.mesh.parent?.remove(this.mesh);
    this.mesh.geometry.dispose();
    (this.mesh.material as THREE.Material).dispose();
    this.mesh = null;
  }
}
