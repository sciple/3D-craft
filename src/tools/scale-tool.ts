import * as THREE from "three";
import { documentStore, faceIdKey } from "../state/document-store";
import type { FaceId } from "../state/document-store";
import type { Tool, ToolContext } from "./types";
import { pointerToNdc } from "./types";

const MIN_SCALE = 0.02;

/// Click a face (or, if it's part of the current selection, every selected
/// face) and drag to scale it uniformly about the group's centroid.
/// Dragging away from the centroid grows it, dragging toward the centroid
/// shrinks it - a screen-space radial drag (distance from the pivot's
/// on-screen position) rather than a 3D axis, since unlike Push/Pull there
/// is no single natural drag axis for a uniform scale; this mirrors how
/// SketchUp's own Scale handles read as "drag the corner toward/away from
/// the center."
export class ScaleTool implements Tool {
  readonly name = "scale";

  private raycaster = new THREE.Raycaster();
  private dragging = false;
  private targetFaceIds: FaceId[] = [];
  private pivot = new THREE.Vector3();
  private pivotScreenPx = new THREE.Vector2();
  private startDistancePx = 0;
  private currentScale = 1;

  private previewMesh: THREE.Mesh | null = null;
  private previewEdges: THREE.LineSegments | null = null;
  private basePositions: Float32Array | null = null;

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

    const pivotNdc = this.pivot.clone().project(ctx.camera);
    const rect = ctx.domElement.getBoundingClientRect();
    this.pivotScreenPx.set(((pivotNdc.x + 1) / 2) * rect.width + rect.left, ((1 - pivotNdc.y) / 2) * rect.height + rect.top);
    this.startDistancePx = Math.max(1, this.pivotScreenPx.distanceTo(new THREE.Vector2(e.clientX, e.clientY)));

    this.currentScale = 1;
    this.dragging = true;
    this.buildPreview(ctx, this.targetFaceIds);
  }

  onPointerMove(e: PointerEvent) {
    if (!this.dragging) return;
    const currentDistancePx = this.pivotScreenPx.distanceTo(new THREE.Vector2(e.clientX, e.clientY));
    this.currentScale = Math.max(MIN_SCALE, currentDistancePx / this.startDistancePx);
    this.updatePreviewPositions();
  }

  async onPointerUp() {
    if (!this.dragging) return;
    this.dragging = false;
    const faceIds = this.targetFaceIds;
    const scale = this.currentScale;
    this.targetFaceIds = [];
    this.clearPreview();
    if (faceIds.length > 0 && Math.abs(scale - 1) > 1e-4) {
      await documentStore.scaleFaces(faceIds, [this.pivot.x, this.pivot.y, this.pivot.z], [scale, scale, scale]);
    }
  }

  onKeyDown(e: KeyboardEvent) {
    if (e.key === "Escape" && this.dragging) {
      this.dragging = false;
      this.targetFaceIds = [];
      this.clearPreview();
    }
  }

  deactivate() {
    this.dragging = false;
    this.targetFaceIds = [];
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
      color: 0x4aa3ff,
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
      out[i] = this.pivot.x + (this.basePositions[i] - this.pivot.x) * this.currentScale;
      out[i + 1] = this.pivot.y + (this.basePositions[i + 1] - this.pivot.y) * this.currentScale;
      out[i + 2] = this.pivot.z + (this.basePositions[i + 2] - this.pivot.z) * this.currentScale;
    }
    for (const geometry of [this.previewMesh.geometry, this.previewEdges.geometry]) {
      const attr = geometry.getAttribute("position") as THREE.BufferAttribute;
      attr.set(out);
      attr.needsUpdate = true;
      geometry.computeBoundingSphere();
    }
    this.previewMesh.geometry.computeVertexNormals();
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
  }
}
