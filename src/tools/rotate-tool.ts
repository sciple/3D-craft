import * as THREE from "three";
import { documentStore, faceIdKey } from "../state/document-store";
import type { FaceId } from "../state/document-store";
import type { Tool, ToolContext } from "./types";
import { pointerToNdc } from "./types";
import { NumericBuffer } from "./numeric-input";
import { measurementHud } from "../ui/measurement-hud";
import { AXIS_COLORS, AXIS_VECTORS, GUIDE_LENGTH } from "./axis-constraint";
import type { AxisName } from "./axis-constraint";

/// Click a face (or, if it's part of the current selection, every selected
/// face) and drag in a circle to rotate it about the spin axis through the
/// group's centroid. The axis defaults to vertical (world Z - the common
/// case of turning a part on the workbench); pressing X, Y, or Z (before or
/// during a drag) switches the spin axis to that world axis, with an
/// axis-colored guide line through the pivot showing which axis is active.
/// The axis choice persists across drags until changed or the tool is
/// switched. A typed number of degrees while dragging overrides the angle
/// (magnitude only - sign follows the current drag direction; an explicit
/// "-" forces it).
export class RotateTool implements Tool {
  readonly name = "rotate";

  private raycaster = new THREE.Raycaster();
  private dragging = false;
  private targetFaceIds: FaceId[] = [];
  private pivot = new THREE.Vector3();
  private axis: AxisName = "z";
  // Orthonormal basis (u, v) spanning the plane perpendicular to the spin
  // axis, right-handed about it (v = axis x u), so angleAt measures a
  // right-handed angle that matches the backend's from_axis_angle winding.
  private basisU = new THREE.Vector3(1, 0, 0);
  private basisV = new THREE.Vector3(0, 1, 0);
  private rotationPlane = new THREE.Plane();
  private startPoint = new THREE.Vector3();
  private startAngle = 0;
  private currentAngle = 0;
  private lastMoveEvent: PointerEvent | null = null;
  private numeric = new NumericBuffer();

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

    const snapshot = documentStore.getSnapshot();
    const facesById = new Map(snapshot.faces.map((f) => [faceIdKey(f.id), f]));
    const points: THREE.Vector3[] = [];
    for (const faceId of this.targetFaceIds) {
      const face = facesById.get(faceIdKey(faceId));
      if (!face) continue;
      for (const idx of face.outer) points.push(new THREE.Vector3(...snapshot.vertices[idx]));
      for (const hole of face.holes) for (const idx of hole) points.push(new THREE.Vector3(...snapshot.vertices[idx]));
    }
    if (points.length === 0) return;
    this.pivot = points.reduce((sum, p) => sum.add(p), new THREE.Vector3()).divideScalar(points.length);
    this.updateBasis();

    this.startPoint.copy(hit.point);
    this.startAngle = this.angleAt(this.startPoint);
    this.currentAngle = this.startAngle;
    this.lastMoveEvent = null;
    this.numeric.clear();
    this.dragging = true;
    this.buildPreview(ctx, this.targetFaceIds);
    this.updateGuideLine(ctx);
  }

  onPointerMove(e: PointerEvent, ctx: ToolContext) {
    if (!this.dragging) return;
    this.lastMoveEvent = e;
    this.applyPointerAngle(e, ctx);
  }

  private applyPointerAngle(e: PointerEvent, ctx: ToolContext) {
    const ndc = pointerToNdc(e, ctx.domElement);
    this.raycaster.setFromCamera(ndc, ctx.camera);
    const point = new THREE.Vector3();
    if (!this.raycaster.ray.intersectPlane(this.rotationPlane, point)) return;
    this.currentAngle = this.angleAt(point);
    this.updatePreviewPositions();
  }

  async onPointerUp() {
    if (!this.dragging) return;
    this.dragging = false;
    const faceIds = this.targetFaceIds;
    const angle = this.effectiveAngle();
    const axis = AXIS_VECTORS[this.axis];
    this.targetFaceIds = [];
    this.clearPreview();
    if (faceIds.length > 0 && Math.abs(angle) > 1e-4) {
      await documentStore.rotateFaces(faceIds, [this.pivot.x, this.pivot.y, this.pivot.z], [axis.x, axis.y, axis.z], angle);
    }
  }

  onKeyDown(e: KeyboardEvent, ctx: ToolContext) {
    const key = e.key.toLowerCase();
    // Axis keys take priority over numeric typing (checked first, and
    // NumericBuffer never sees them) since 'x' would otherwise also read as
    // a valid buffer character (a "WxH"-style separator, unused here but
    // shared code with the draw tools). Unlike Move there's no "free" mode -
    // rotation is always about some axis - so these set the axis rather than
    // toggling a lock off.
    if ((key === "x" || key === "y" || key === "z") && !e.ctrlKey && !e.metaKey && !e.altKey) {
      this.axis = key;
      this.updateBasis();
      if (this.dragging) {
        // Re-anchor the drag on the new axis: recompute the start angle from
        // the original grab point in the new plane, then re-derive the
        // current angle from the last mouse position so the preview snaps to
        // the new axis immediately without waiting for a mouse move.
        this.startAngle = this.angleAt(this.startPoint);
        this.currentAngle = this.startAngle;
        if (this.lastMoveEvent) this.applyPointerAngle(this.lastMoveEvent, ctx);
        this.updateGuideLine(ctx);
        this.updatePreviewPositions();
      }
      return;
    }

    if (this.dragging) {
      if (e.key === "Enter") {
        if (!this.numeric.isEmpty) void this.onPointerUp();
        return;
      }
      if (e.key === "Escape" && !this.numeric.isEmpty) {
        this.numeric.clear();
        this.updatePreviewPositions();
        return;
      }
      if (this.numeric.type(e)) {
        this.updatePreviewPositions();
        return;
      }
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
    this.axis = "z";
    this.clearPreview();
  }

  /// Rebuilds the perpendicular basis and rotation plane for the current
  /// spin axis and pivot. `ref` avoids being parallel to the axis; the
  /// cross-product chain yields a right-handed (u, v, axis) frame, and for
  /// the default Z axis reproduces exactly the old (u = X, v = Y) behavior.
  private updateBasis() {
    const n = AXIS_VECTORS[this.axis];
    const ref = Math.abs(n.y) < 0.9 ? AXIS_VECTORS.y : AXIS_VECTORS.x;
    this.basisU = new THREE.Vector3().crossVectors(ref, n).normalize();
    this.basisV = new THREE.Vector3().crossVectors(n, this.basisU).normalize();
    this.rotationPlane.setFromNormalAndCoplanarPoint(n, this.pivot);
  }

  private angleAt(point: THREE.Vector3): number {
    const d = new THREE.Vector3().subVectors(point, this.pivot);
    return Math.atan2(d.dot(this.basisV), d.dot(this.basisU));
  }

  /// The typed buffer holds degrees (more natural to type than radians);
  /// converted to radians here, the sole point where the drag's radian
  /// angle and a typed override reconcile into one effective value.
  private effectiveAngle(): number {
    const dragAngle = this.currentAngle - this.startAngle;
    if (this.numeric.isEmpty) return dragAngle;
    const v = this.numeric.values()[0];
    if (v === undefined) return dragAngle;
    const magnitudeRad = THREE.MathUtils.degToRad(Math.abs(v));
    if (this.numeric.display.includes("-")) return -magnitudeRad;
    return dragAngle < 0 ? -magnitudeRad : magnitudeRad;
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
      color: 0xd24aff,
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
    const angle = this.effectiveAngle();
    // Rotate each vertex about the (pivot, axis) line - a quaternion handles
    // any axis, where the old code's inline cos/sin only ever rotated in XY.
    const quat = new THREE.Quaternion().setFromAxisAngle(AXIS_VECTORS[this.axis], angle);
    const n = this.basePositions.length;
    const out = new Float32Array(n);
    const p = new THREE.Vector3();
    for (let i = 0; i < n; i += 3) {
      p.set(
        this.basePositions[i] - this.pivot.x,
        this.basePositions[i + 1] - this.pivot.y,
        this.basePositions[i + 2] - this.pivot.z,
      );
      p.applyQuaternion(quat);
      out[i] = p.x + this.pivot.x;
      out[i + 1] = p.y + this.pivot.y;
      out[i + 2] = p.z + this.pivot.z;
    }
    for (const geometry of [this.previewMesh.geometry, this.previewEdges.geometry]) {
      const attr = geometry.getAttribute("position") as THREE.BufferAttribute;
      attr.set(out);
      attr.needsUpdate = true;
      geometry.computeBoundingSphere();
    }
    this.previewMesh.geometry.computeVertexNormals();
    measurementHud.show(
      `Rotate (${this.axis.toUpperCase()})`,
      `${THREE.MathUtils.radToDeg(angle).toFixed(1)}°`,
      this.numeric.isEmpty ? null : this.numeric.display,
    );
  }

  /// Draws an axis-colored line through the pivot along the spin axis while
  /// dragging (or removes it when idle) so the active rotation axis is
  /// always visible - the feedback the Z-only version never had.
  private updateGuideLine(ctx: ToolContext) {
    this.removeGuideLine();
    if (!this.dragging) return;
    const axis = AXIS_VECTORS[this.axis];
    const a = this.pivot.clone().addScaledVector(axis, -GUIDE_LENGTH);
    const b = this.pivot.clone().addScaledVector(axis, GUIDE_LENGTH);
    const geometry = new THREE.BufferGeometry().setFromPoints([a, b]);
    const material = new THREE.LineBasicMaterial({
      color: AXIS_COLORS[this.axis],
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
    this.numeric.clear();
    measurementHud.hide();
  }
}
