import * as THREE from "three";
import { documentStore, faceIdKey } from "../state/document-store";
import type { FaceId } from "../state/document-store";
import type { Tool, ToolContext } from "./types";
import { pointerToNdc } from "./types";

/// Click a face (or, if it's part of the current selection, every selected
/// face) and drag to translate it. Dragging moves within the horizontal
/// plane through the clicked point by default (X/Y - the common case of
/// repositioning a part on the workbench); holding Shift switches to a
/// vertical drag (Z only, using screen-space vertical mouse movement),
/// covering the other axis most spaceship-part layout needs without a full
/// 3-axis gizmo.
export class MoveTool implements Tool {
  readonly name = "move";

  private raycaster = new THREE.Raycaster();
  private dragging = false;
  private targetFaceIds: FaceId[] = [];
  private startPoint = new THREE.Vector3();
  private startScreenY = 0;
  private currentDelta = new THREE.Vector3();

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

    this.startPoint.copy(hit.point);
    this.startScreenY = e.clientY;
    this.currentDelta.set(0, 0, 0);
    this.dragging = true;
    this.buildPreview(ctx, this.targetFaceIds);
  }

  onPointerMove(e: PointerEvent, ctx: ToolContext) {
    if (!this.dragging) return;

    if (e.shiftKey) {
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
