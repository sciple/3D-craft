import * as THREE from "three";
import { documentStore, faceIdKey } from "../state/document-store";
import type { FaceId } from "../state/document-store";
import type { Tool, ToolContext } from "./types";
import { pointerToNdc } from "./types";
import { closestDistanceAlongAxis } from "./axis-drag";

type AxisName = "x" | "y" | "z";

const AXIS_VECTORS: Record<AxisName, THREE.Vector3> = {
  x: new THREE.Vector3(1, 0, 0),
  y: new THREE.Vector3(0, 1, 0),
  z: new THREE.Vector3(0, 0, 1),
};

// SketchUp's axis colors: red X, green Y, blue Z - shown on the guide line
// while an axis lock is active so the constraint is visible at a glance.
const AXIS_COLORS: Record<AxisName, number> = {
  x: 0xff3b30,
  y: 0x34c759,
  z: 0x3a7bff,
};

const GUIDE_LENGTH = 1000;

/// Click a face (or, if it's part of the current selection, every selected
/// face) and drag to translate it. Dragging moves within the horizontal
/// plane through the clicked point by default (X/Y - the common case of
/// repositioning a part on the workbench); holding Shift switches to a
/// vertical drag (Z only, using screen-space vertical mouse movement).
///
/// Pressing X, Y, or Z (before or during a drag) locks movement to that
/// world axis - the delta is the projection of the mouse ray onto the axis
/// line through the grab point, so parts stay exactly aligned instead of
/// drifting off-axis. Pressing the same key again unlocks; the lock
/// persists across drags until toggled off or the tool is switched, and an
/// axis-colored guide line through the grab point shows the constraint
/// while dragging.
export class MoveTool implements Tool {
  readonly name = "move";

  private raycaster = new THREE.Raycaster();
  private dragging = false;
  private targetFaceIds: FaceId[] = [];
  private startPoint = new THREE.Vector3();
  private startScreenY = 0;
  private currentDelta = new THREE.Vector3();
  private axisLock: AxisName | null = null;
  private lastMoveEvent: PointerEvent | null = null;

  private previewMesh: THREE.Mesh | null = null;
  private previewEdges: THREE.LineSegments | null = null;
  private basePositions: Float32Array | null = null;
  private guideLine: THREE.Line | null = null;

  onPointerDown(e: PointerEvent, ctx: ToolContext) {
    if (e.button !== 0) return;
    const ndc = pointerToNdc(e, ctx.domElement);
    this.raycaster.setFromCamera(ndc, ctx.camera);
    const hits = this.raycaster.intersectObject(ctx.meshRenderer.mesh);
    const hit = hits[0];
    if (!hit || hit.faceIndex == null) return;

    const clickedFaceId = ctx.meshRenderer.faceIdForTriangle(hit.faceIndex);
    if (!clickedFaceId) return;

    const selected = documentStore.getSnapshot().selected_face_ids;
    const clickedKey = faceIdKey(clickedFaceId);
    this.targetFaceIds = selected.some((f) => faceIdKey(f) === clickedKey) ? selected : [clickedFaceId];

    this.startPoint.copy(hit.point);
    this.startScreenY = e.clientY;
    this.currentDelta.set(0, 0, 0);
    this.dragging = true;
    this.buildPreview(ctx, this.targetFaceIds);
    this.updateGuideLine(ctx);
  }

  onPointerMove(e: PointerEvent, ctx: ToolContext) {
    if (!this.dragging) return;
    this.lastMoveEvent = e;
    this.applyPointerDelta(e, ctx);
  }

  private applyPointerDelta(e: PointerEvent, ctx: ToolContext) {
    if (this.axisLock) {
      const axis = AXIS_VECTORS[this.axisLock];
      const ndc = pointerToNdc(e, ctx.domElement);
      this.raycaster.setFromCamera(ndc, ctx.camera);
      const distance = closestDistanceAlongAxis(
        this.startPoint,
        axis,
        this.raycaster.ray.origin,
        this.raycaster.ray.direction,
      );
      this.currentDelta.copy(axis).multiplyScalar(distance);
    } else if (e.shiftKey) {
      // Vertical drag: 200 screen px of vertical movement = 1 world unit at
      // the current view - simple and resolution-independent enough for
      // nudging a part up/down without needing an on-screen axis handle.
      const dz = -(e.clientY - this.startScreenY) / 200;
      this.currentDelta.set(0, 0, dz);
    } else {
      const ndc = pointerToNdc(e, ctx.domElement);
      this.raycaster.setFromCamera(ndc, ctx.camera);
      const groundPlane = new THREE.Plane(new THREE.Vector3(0, 0, 1), -this.startPoint.z);
      const point = new THREE.Vector3();
      if (!this.raycaster.ray.intersectPlane(groundPlane, point)) return;
      this.currentDelta.set(point.x - this.startPoint.x, point.y - this.startPoint.y, 0);
    }
    this.updatePreviewPositions();
  }

  async onPointerUp() {
    if (!this.dragging) return;
    this.dragging = false;
    const faceIds = this.targetFaceIds;
    const delta = this.currentDelta.clone();
    this.targetFaceIds = [];
    this.clearPreview();
    if (faceIds.length > 0 && delta.length() > 1e-6) {
      await documentStore.moveFaces(faceIds, [delta.x, delta.y, delta.z]);
    }
  }

  onKeyDown(e: KeyboardEvent, ctx: ToolContext) {
    const key = e.key.toLowerCase();
    if ((key === "x" || key === "y" || key === "z") && !e.ctrlKey && !e.metaKey && !e.altKey) {
      this.axisLock = this.axisLock === key ? null : key;
      this.updateGuideLine(ctx);
      if (this.dragging && this.lastMoveEvent) {
        // Re-derive the delta immediately so toggling mid-drag snaps the
        // preview onto (or off) the axis without waiting for a mouse move.
        this.applyPointerDelta(this.lastMoveEvent, ctx);
      }
      return;
    }
    if (e.key === "Escape" && this.dragging) {
      this.dragging = false;
      this.targetFaceIds = [];
      this.clearPreview();
    }
  }

  deactivate() {
    this.dragging = false;
    this.targetFaceIds = [];
    this.axisLock = null;
    this.clearPreview();
  }

  private buildPreview(ctx: ToolContext, faceIds: FaceId[]) {
    const snapshot = documentStore.getSnapshot();
    const facesById = new Map(snapshot.faces.map((f) => [faceIdKey(f.id), f]));

    const basePositions: number[] = [];
    const triangles: number[] = [];
    const edgeIndices: number[] = [];

    for (const faceId of faceIds) {
      const face = facesById.get(faceIdKey(faceId));
      if (!face) continue;

      const localIndexByOriginal = new Map<number, number>();
      const remap = (originalIndex: number): number => {
        let localIndex = localIndexByOriginal.get(originalIndex);
        if (localIndex === undefined) {
          localIndex = basePositions.length / 3;
          localIndexByOriginal.set(originalIndex, localIndex);
          const v = snapshot.vertices[originalIndex];
          basePositions.push(v[0], v[1], v[2]);
        }
        return localIndex;
      };

      for (const tri of face.triangles) triangles.push(remap(tri[0]), remap(tri[1]), remap(tri[2]));
      for (const loop of [face.outer, ...face.holes]) {
        for (let i = 0; i < loop.length; i++) edgeIndices.push(remap(loop[i]), remap(loop[(i + 1) % loop.length]));
      }
    }

    this.basePositions = new Float32Array(basePositions);

    const geometry = new THREE.BufferGeometry();
    geometry.setAttribute("position", new THREE.BufferAttribute(this.basePositions.slice(), 3));
    geometry.setIndex(triangles);
    geometry.computeVertexNormals();

    const material = new THREE.MeshBasicMaterial({
      color: 0x4aff8c,
      transparent: true,
      opacity: 0.5,
      side: THREE.DoubleSide,
      polygonOffset: true,
      polygonOffsetFactor: 1,
      polygonOffsetUnits: 1,
    });
    this.previewMesh = new THREE.Mesh(geometry, material);
    ctx.scene.add(this.previewMesh);

    const edgeGeometry = new THREE.BufferGeometry();
    edgeGeometry.setAttribute("position", new THREE.BufferAttribute(this.basePositions.slice(), 3));
    edgeGeometry.setIndex(edgeIndices);
    this.previewEdges = new THREE.LineSegments(edgeGeometry, new THREE.LineBasicMaterial({ color: 0x1a1a1a }));
    ctx.scene.add(this.previewEdges);
  }

  private updatePreviewPositions() {
    if (!this.previewMesh || !this.previewEdges || !this.basePositions) return;
    const n = this.basePositions.length;
    const out = new Float32Array(n);
    for (let i = 0; i < n; i += 3) {
      out[i] = this.basePositions[i] + this.currentDelta.x;
      out[i + 1] = this.basePositions[i + 1] + this.currentDelta.y;
      out[i + 2] = this.basePositions[i + 2] + this.currentDelta.z;
    }
    for (const geometry of [this.previewMesh.geometry, this.previewEdges.geometry]) {
      const attr = geometry.getAttribute("position") as THREE.BufferAttribute;
      attr.set(out);
      attr.needsUpdate = true;
      geometry.computeBoundingSphere();
    }
  }

  /// Shows an axis-colored line through the grab point along the locked
  /// axis while dragging (or removes it when unlocked/idle) so the active
  /// constraint is always visible.
  private updateGuideLine(ctx: ToolContext) {
    this.removeGuideLine();
    if (!this.dragging || !this.axisLock) return;
    const axis = AXIS_VECTORS[this.axisLock];
    const a = this.startPoint.clone().addScaledVector(axis, -GUIDE_LENGTH);
    const b = this.startPoint.clone().addScaledVector(axis, GUIDE_LENGTH);
    const geometry = new THREE.BufferGeometry().setFromPoints([a, b]);
    const material = new THREE.LineBasicMaterial({
      color: AXIS_COLORS[this.axisLock],
      transparent: true,
      opacity: 0.6,
    });
    this.guideLine = new THREE.Line(geometry, material);
    ctx.scene.add(this.guideLine);
  }

  private removeGuideLine() {
    if (!this.guideLine) return;
    this.guideLine.parent?.remove(this.guideLine);
    this.guideLine.geometry.dispose();
    (this.guideLine.material as THREE.Material).dispose();
    this.guideLine = null;
  }

  private clearPreview() {
    if (this.previewMesh) {
      this.previewMesh.parent?.remove(this.previewMesh);
      this.previewMesh.geometry.dispose();
      (this.previewMesh.material as THREE.Material).dispose();
      this.previewMesh = null;
    }
    if (this.previewEdges) {
      this.previewEdges.parent?.remove(this.previewEdges);
      this.previewEdges.geometry.dispose();
      (this.previewEdges.material as THREE.Material).dispose();
      this.previewEdges = null;
    }
    this.basePositions = null;
    this.removeGuideLine();
  }
}
