import * as THREE from "three";
import type { MeshRenderer } from "../viewport/mesh-renderer";

export interface ToolContext {
  scene: THREE.Scene;
  camera: THREE.PerspectiveCamera;
  domElement: HTMLElement;
  meshRenderer: MeshRenderer;
}

/// A modeling tool driving left/right mouse interaction in the viewport.
/// Camera navigation (middle mouse + wheel) is handled separately by
/// CadCameraControls and never reaches tools.
export interface Tool {
  readonly name: string;
  activate?(ctx: ToolContext): void;
  deactivate?(ctx: ToolContext): void;
  onPointerDown?(e: PointerEvent, ctx: ToolContext): void;
  onPointerMove?(e: PointerEvent, ctx: ToolContext): void;
  onPointerUp?(e: PointerEvent, ctx: ToolContext): void;
  onKeyDown?(e: KeyboardEvent, ctx: ToolContext): void;
  onKeyUp?(e: KeyboardEvent, ctx: ToolContext): void;
}

export function pointerToNdc(e: PointerEvent, domElement: HTMLElement): THREE.Vector2 {
  const rect = domElement.getBoundingClientRect();
  return new THREE.Vector2(
    ((e.clientX - rect.left) / rect.width) * 2 - 1,
    -((e.clientY - rect.top) / rect.height) * 2 + 1,
  );
}

/// The default drawing surface: the world XY ground plane (Z=0), matching
/// this app's Z-up convention. Draw tools intersect the pointer ray against
/// this plane; a "draw on top of an existing face" mode is a natural future
/// extension but isn't implemented in v1.
export const GROUND_PLANE = new THREE.Plane(new THREE.Vector3(0, 0, 1), 0);

export function raycastGroundPlane(
  e: PointerEvent,
  ctx: ToolContext,
  raycaster: THREE.Raycaster,
): THREE.Vector3 | null {
  raycaster.setFromCamera(pointerToNdc(e, ctx.domElement), ctx.camera);
  const point = new THREE.Vector3();
  return raycaster.ray.intersectPlane(GROUND_PLANE, point) ? point : null;
}
