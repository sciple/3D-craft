import * as THREE from "three";
import type { SketchPlane } from "./plane";

export type AxisName = "x" | "y" | "z";

export const AXIS_VECTORS: Record<AxisName, THREE.Vector3> = {
  x: new THREE.Vector3(1, 0, 0),
  y: new THREE.Vector3(0, 1, 0),
  z: new THREE.Vector3(0, 0, 1),
};

// SketchUp's axis colors: red X, green Y, blue Z - shown on guide lines
// while an axis lock is active so the constraint is visible at a glance.
export const AXIS_COLORS: Record<AxisName, number> = {
  x: 0xff3b30,
  y: 0x34c759,
  z: 0x3a7bff,
};

export const GUIDE_LENGTH = 1000;

/// Projects a sketch-plane-local point onto whichever of the plane's own
/// u/v directions the delta from `from` is more aligned with, guaranteeing a
/// perpendicular (90°) segment regardless of how the plane itself is
/// oriented in world space - unlike projecting world X/Y/Z into the plane,
/// which only stays perpendicular when the plane happens to be axis-aligned.
/// Returns null for a degenerate (near-zero) delta so callers can fall back
/// to unconstrained behavior.
export function constrainUvToPlaneAxis(
  plane: SketchPlane,
  from: THREE.Vector2,
  to: THREE.Vector2,
): { uv: THREE.Vector2; axis: THREE.Vector3 } | null {
  const d = to.clone().sub(from);
  if (d.lengthSq() < 1e-12) return null;
  if (Math.abs(d.x) >= Math.abs(d.y)) {
    return { uv: new THREE.Vector2(from.x + d.x, from.y), axis: plane.u.clone() };
  }
  return { uv: new THREE.Vector2(from.x, from.y + d.y), axis: plane.v.clone() };
}

const WORLD_AXES: THREE.Vector3[] = [AXIS_VECTORS.x, AXIS_VECTORS.y, AXIS_VECTORS.z];

/// SketchUp-style axis color when `dir` is (near enough) a world axis
/// direction, so ground-plane and box-face tracing gets the familiar
/// red/green/blue feedback; falls back to a neutral amber (matching the
/// draw tools' preview line color) on a tilted plane where the constraint
/// axis isn't actually a world axis.
export function guideColorForDirection(dir: THREE.Vector3): number {
  const n = dir.clone().normalize();
  for (const axis of WORLD_AXES) {
    if (Math.abs(n.dot(axis)) > 0.999) {
      const name = axis === AXIS_VECTORS.x ? "x" : axis === AXIS_VECTORS.y ? "y" : "z";
      return AXIS_COLORS[name];
    }
  }
  return 0xffcc55;
}
