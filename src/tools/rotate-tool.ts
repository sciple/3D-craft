import * as THREE from "three";
import { documentStore, faceIdKey } from "../state/document-store";
import type { FaceId } from "../state/document-store";
import type { Tool, ToolContext } from "./types";
import { pointerToNdc } from "./types";

/// Click a face (or, if it's part of the current selection, every selected
/// face) and drag in a circle to rotate it about a vertical (world Z) axis
/// through the group's centroid - the one rotation a workbench layout
/// actually needs most (turning a wing or hull segment to face the right
/// way), rather than a full 3-axis trackball gizmo.
export class RotateTool implements Tool {
  readonly name = "rotate";

  private raycaster = new THREE.Raycaster();
  private dragging = false;
  private targetFaceIds: FaceId[] = [];
  private pivot = new THREE.Vector3();
  private rotationPlane = new THREE.Plane();
  private startAngle = 0;
  private currentAngle = 0;

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
    this.rotationPlane.setFromNormalAndCoplanarPoint(new THREE.Vector3(0, 0, 1), this.pivot);

    this.startAngle = this.angleAt(hit.point);
    this.currentAngle = this.startAngle;
    this.dragging = true;
    this.buildPreview(ctx, this.targetFaceIds);
  }

  onPointerMove(e: PointerEvent, ctx: ToolContext) {
    if (!this.dragging) return;
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
    const angle = this.currentAngle - this.startAngle;
    this.targetFaceIds = [];
    this.clearPreview();
    if (faceIds.length > 0 && Math.abs(angle) > 1e-4) {
      await documentStore.rotateFaces(faceIds, [this.pivot.x, this.pivot.y, this.pivot.z], [0, 0, 1], angle);
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

  private angleAt(point: THREE.Vector3): number {
    return Math.atan2(point.y - this.pivot.y, point.x - this.pivot.x);
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
    const angle = this.currentAngle - this.startAngle;
    const cos = Math.cos(angle);
    const sin = Math.sin(angle);
    const n = this.basePositions.length;
    const out = new Float32Array(n);
    for (let i = 0; i < n; i += 3) {
      const dx = this.basePositions[i] - this.pivot.x;
      const dy = this.basePositions[i + 1] - this.pivot.y;
      out[i] = this.pivot.x + dx * cos - dy * sin;
      out[i + 1] = this.pivot.y + dx * sin + dy * cos;
      out[i + 2] = this.basePositions[i + 2];
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
