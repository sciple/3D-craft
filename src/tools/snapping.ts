import * as THREE from "three";
import { documentStore } from "../state/document-store";
import type { DocumentSnapshot, Guide } from "../state/document-store";
import type { ToolContext } from "./types";
import { pointerToNdc, GROUND_PLANE } from "./types";
import type { SketchPlane } from "./plane";
import { toThreePlane } from "./plane";
import { GUIDE_COLOR } from "../viewport/guide-renderer";

export type SnapKind = "endpoint" | "midpoint" | "edge" | "guide";

export interface SnapResult {
  point: THREE.Vector3;
  kind: SnapKind;
}

const SNAP_PIXELS = 12;
const PLANE_EPS = 1e-4;

/// Turns a guide's two flat `[x,y,z]` endpoints into a pair of `Vector3`s,
/// shared by both `findPlanarSnap` and `findSnap3d`.
function guideEndpoints(guides: Guide[]): { a: THREE.Vector3; b: THREE.Vector3 }[] {
  return guides.map((g) => ({ a: new THREE.Vector3(...g.a), b: new THREE.Vector3(...g.b) }));
}

/// Picks the nearest candidate within `tolerance` of `target`, or null if
/// none qualify. Candidates already carry their own `SnapKind` (model
/// endpoint/midpoint/edge, or guide), so this has no per-tier branching -
/// it's shared verbatim between `findPlanarSnap` and `findSnap3d`.
function nearestWithin(candidates: SnapResult[], target: THREE.Vector3, tolerance: number): SnapResult | null {
  let best: SnapResult | null = null;
  let bestDist = tolerance;
  for (const c of candidates) {
    const d = c.point.distanceTo(target);
    if (d < bestDist) {
      bestDist = d;
      best = c;
    }
  }
  // Clone on the way out: candidates may be shared/reused vectors (guide
  // endpoints in particular), and callers are free to mutate what they get.
  return best && { point: best.point.clone(), kind: best.kind };
}

/// Finds the nearest snap candidate among vertices/edges of existing
/// geometry - and guides left by the Measure tool - that lie on `plane` (the
/// sketch plane the active draw tool is currently working on - the ground
/// plane by default, or a solid face's own plane when drawing directly on
/// top of it) within a tolerance sized in world units to read as a constant
/// ~12px on screen regardless of zoom. Priority matches SketchUp's inference
/// order: an endpoint within tolerance always wins over a midpoint or edge
/// point even if the midpoint/edge point is numerically closer to the
/// cursor, since endpoints are the more useful target. Model geometry and
/// guides share every tier - within a tier the nearest candidate wins
/// regardless of source, since a model corner and a guide endpoint are
/// equally deliberate targets.
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
  // Point-based (unlike the old index-based onPlane) so it can test guide
  // endpoints too, which have no index into snapshot.vertices. Must clone
  // before `sub`: unlike positionOf's throwaway, these points are also
  // pushed as snap candidates below and must not be mutated in place.
  const pointOnPlane = (p: THREE.Vector3) => Math.abs(p.clone().sub(plane.origin).dot(plane.normal)) < PLANE_EPS;

  const guides = guideEndpoints(snapshot.guides);

  const endpoints: SnapResult[] = [];
  for (let i = 0; i < snapshot.vertices.length; i++) {
    const p = positionOf(i);
    if (pointOnPlane(p)) endpoints.push({ point: p, kind: "endpoint" });
  }
  for (const g of guides) {
    if (pointOnPlane(g.a)) endpoints.push({ point: g.a, kind: "guide" });
    if (pointOnPlane(g.b)) endpoints.push({ point: g.b, kind: "guide" });
  }
  const endpointHit = nearestWithin(endpoints, planePoint, tolerance);
  if (endpointHit) return endpointHit;

  const midpoints: SnapResult[] = [];
  const edgePoints: SnapResult[] = [];
  for (const face of snapshot.faces) {
    for (const loop of [face.outer, ...face.holes]) {
      for (let i = 0; i < loop.length; i++) {
        const a = loop[i];
        const b = loop[(i + 1) % loop.length];
        const pa = positionOf(a);
        const pb = positionOf(b);
        if (!pointOnPlane(pa) || !pointOnPlane(pb)) continue;
        midpoints.push({ point: pa.clone().add(pb).multiplyScalar(0.5), kind: "midpoint" });
        edgePoints.push({ point: closestPointOnSegment(planePoint, pa, pb), kind: "edge" });
      }
    }
  }
  // A guide contributes a midpoint/along-point only when it lies wholly on
  // the sketch plane - same rule as model edges above. A guide that merely
  // crosses the plane has an off-plane midpoint, and snapping to it would
  // drag the drawn shape off the sketch plane.
  for (const g of guides) {
    if (!pointOnPlane(g.a) || !pointOnPlane(g.b)) continue;
    midpoints.push({ point: g.a.clone().add(g.b).multiplyScalar(0.5), kind: "guide" });
    edgePoints.push({ point: closestPointOnSegment(planePoint, g.a, g.b), kind: "guide" });
  }
  const midpointHit = nearestWithin(midpoints, planePoint, tolerance);
  if (midpointHit) return midpointHit;

  return nearestWithin(edgePoints, planePoint, tolerance);
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

/// Snap for tools that pick anywhere in the scene (e.g. the tape measure),
/// rather than being constrained to a single sketch plane like
/// `findPlanarSnap`. Anchored at the 3D point actually under the cursor
/// (`anchor` - the raycast hit on the hovered surface, or the ground point),
/// with a world-space `tolerance`, so it only considers geometry genuinely
/// near that point. This is deliberately NOT a pure screen-space test: a
/// distant vertex that merely projects near the cursor (e.g. a back corner of
/// a box lining up behind the front corner you're hovering) is far from the
/// anchor in 3D and so is correctly ignored - the earlier screen-space version
/// grabbed it, which collapsed a face diagonal to a side length. Priority
/// endpoint > midpoint > edge, matching `findPlanarSnap`; guides participate
/// in every tier here too (with no plane filter, so you can measure
/// guide-to-guide).
export function findSnap3d(
  snapshot: DocumentSnapshot,
  anchor: THREE.Vector3,
  tolerance: number,
): SnapResult | null {
  const positionOf = (i: number) => new THREE.Vector3(...snapshot.vertices[i]);
  const guides = guideEndpoints(snapshot.guides);

  const endpoints: SnapResult[] = snapshot.vertices.map((_, i) => ({ point: positionOf(i), kind: "endpoint" }));
  for (const g of guides) {
    endpoints.push({ point: g.a, kind: "guide" }, { point: g.b, kind: "guide" });
  }
  const endpointHit = nearestWithin(endpoints, anchor, tolerance);
  if (endpointHit) return endpointHit;

  const midpoints: SnapResult[] = [];
  const edgePoints: SnapResult[] = [];
  for (const face of snapshot.faces) {
    for (const loop of [face.outer, ...face.holes]) {
      for (let i = 0; i < loop.length; i++) {
        const pa = positionOf(loop[i]);
        const pb = positionOf(loop[(i + 1) % loop.length]);
        midpoints.push({ point: pa.clone().add(pb).multiplyScalar(0.5), kind: "midpoint" });
        edgePoints.push({ point: closestPointOnSegment(anchor, pa, pb), kind: "edge" });
      }
    }
  }
  for (const g of guides) {
    midpoints.push({ point: g.a.clone().add(g.b).multiplyScalar(0.5), kind: "guide" });
    edgePoints.push({ point: closestPointOnSegment(anchor, g.a, g.b), kind: "guide" });
  }
  const midpointHit = nearestWithin(midpoints, anchor, tolerance);
  if (midpointHit) return midpointHit;

  return nearestWithin(edgePoints, anchor, tolerance);
}

/// Full pick for a 3D measuring tool: the raycast hit on the hovered surface
/// (or the ground point) becomes the anchor and free-point fallback, then
/// `findSnap3d` refines it to nearby geometry within ~12px (sized in world
/// units at the anchor's depth). Returns null only when the ray hits nothing.
export function raycastSnapped3d(
  e: PointerEvent,
  ctx: ToolContext,
  raycaster: THREE.Raycaster,
): { point: THREE.Vector3; snap: SnapResult | null } | null {
  raycaster.setFromCamera(pointerToNdc(e, ctx.domElement), ctx.camera);

  const surfaceHit = raycaster.intersectObject(ctx.meshRenderer.mesh)[0];
  let anchor: THREE.Vector3 | null = surfaceHit ? surfaceHit.point.clone() : null;
  if (!anchor) {
    const ground = new THREE.Vector3();
    anchor = raycaster.ray.intersectPlane(GROUND_PLANE, ground) ? ground : null;
  }
  if (!anchor) return null;

  // World tolerance sized to read as a constant ~12px at the anchor's depth,
  // matching findPlanarSnap's zoom-independent feel.
  const rect = ctx.domElement.getBoundingClientRect();
  const distance = Math.max(0.001, ctx.camera.position.distanceTo(anchor));
  const worldPerPixel = (2 * distance * Math.tan((ctx.camera.fov * Math.PI) / 360)) / rect.height;
  const tolerance = SNAP_PIXELS * worldPerPixel;

  const snap = findSnap3d(documentStore.getSnapshot(), anchor, tolerance);
  return { point: snap ? snap.point : anchor, snap };
}

const SNAP_COLORS: Record<SnapKind, number> = {
  endpoint: 0x2ec4ff,
  midpoint: 0x33ff88,
  edge: 0xffd633,
  // Matches GuideRenderer's drawn guides - one color answers "model, or the
  // mark I left?" without needing separate endpoint/midpoint/along shades
  // for guides, since that distinction is already visible from where the
  // dot sits on the drawn segment.
  guide: GUIDE_COLOR,
};

/// A small on-screen marker shown at the active snap candidate while
/// drawing, color-coded by kind (endpoint/midpoint/edge/guide) - the "simple
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
