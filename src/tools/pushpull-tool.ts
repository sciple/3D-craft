import * as THREE from "three";
import { documentStore, faceIdKey } from "../state/document-store";
import type { FaceId } from "../state/document-store";
import type { Tool, ToolContext } from "./types";
import { pointerToNdc } from "./types";
import { closestDistanceAlongAxis } from "./axis-drag";

/// Click a face and drag to extrude it along its own normal, releasing to
/// commit. If the clicked face is part of the current selection, every
/// selected face is pushed/pulled together (each along its own normal, by
/// the same drag distance) rather than just the one under the cursor -
/// clicking an unselected face instead starts a fresh single-face drag.
///
/// The drag distance is computed as the closest point on the (pick point,
/// clicked face's normal) axis to the current mouse ray - the standard
/// "drag along a 3D axis with a 2D mouse" technique used by translate
/// gizmos - rather than a raw screen-space delta, so the preview tracks the
/// mouse naturally regardless of camera angle.
export class PushPullTool implements Tool {
  readonly name = "pushpull";

  private raycaster = new THREE.Raycaster();
  private dragging = false;
  private targetFaceIds: FaceId[] = [];
  private axisOrigin = new THREE.Vector3();
  private axisDir = new THREE.Vector3();
  private currentDistance = 0;

  private previewMesh: THREE.Mesh | null = null;
  private previewEdges: THREE.LineSegments | null = null;
  private basePositions: Float32Array | null = null;
  private perVertexNormal: Float32Array | null = null;

  onPointerDown(e: PointerEvent, ctx: ToolContext) {
    if (e.button !== 0) return;
    const ndc = pointerToNdc(e, ctx.domElement);
    this.raycaster.setFromCamera(ndc, ctx.camera);
    const hits = this.raycaster.intersectObject(ctx.meshRenderer.mesh);
    const hit = hits[0];
    if (!hit || hit.faceIndex == null || !hit.face) return;

    const clickedFaceId = ctx.meshRenderer.faceIdForTriangle(hit.faceIndex);
    if (!clickedFaceId) return;

    const selected = documentStore.getSnapshot().selected_face_ids;
    const clickedKey = faceIdKey(clickedFaceId);
    this.targetFaceIds = selected.some((f) => faceIdKey(f) === clickedKey) ? selected : [clickedFaceId];

    this.axisOrigin.copy(hit.point);
    this.axisDir.copy(hit.face.normal).normalize();
    this.currentDistance = 0;
    this.dragging = true;
    this.buildPreview(ctx, this.targetFaceIds);
  }

  onPointerMove(e: PointerEvent, ctx: ToolContext) {
    if (!this.dragging) return;
    const ndc = pointerToNdc(e, ctx.domElement);
    this.raycaster.setFromCamera(ndc, ctx.camera);
    this.currentDistance = closestDistanceAlongAxis(
      this.axisOrigin,
      this.axisDir,
      this.raycaster.ray.origin,
      this.raycaster.ray.direction,
    );
    this.updatePreviewPositions();
  }

  async onPointerUp() {
    if (!this.dragging) return;
    this.dragging = false;
    const faceIds = this.targetFaceIds;
    const distance = this.currentDistance;
    this.targetFaceIds = [];
    this.clearPreview();
    if (faceIds.length > 0 && Math.abs(distance) > 1e-6) {
      await documentStore.pushPullFaces(faceIds, distance);
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

  /// Builds one combined preview mesh from every target face's own
  /// triangles, tagging each vertex with its source face's normal so
  /// `updatePreviewPositions` can translate each face along its own normal
  /// (not just a single shared axis) every frame without touching the real
  /// document mesh until the drag commits.
  private buildPreview(ctx: ToolContext, faceIds: FaceId[]) {
    const snapshot = documentStore.getSnapshot();
    const facesById = new Map(snapshot.faces.map((f) => [faceIdKey(f.id), f]));

    const basePositions: number[] = [];
    const perVertexNormal: number[] = [];
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
          perVertexNormal.push(face.normal[0], face.normal[1], face.normal[2]);
        }
        return localIndex;
      };

      for (const tri of face.triangles) {
        triangles.push(remap(tri[0]), remap(tri[1]), remap(tri[2]));
      }
      for (const loop of [face.outer, ...face.holes]) {
        for (let i = 0; i < loop.length; i++) {
          edgeIndices.push(remap(loop[i]), remap(loop[(i + 1) % loop.length]));
        }
      }
    }

    this.basePositions = new Float32Array(basePositions);
    this.perVertexNormal = new Float32Array(perVertexNormal);

    const geometry = new THREE.BufferGeometry();
    geometry.setAttribute("position", new THREE.BufferAttribute(this.basePositions.slice(), 3));
    geometry.setIndex(triangles);
    geometry.computeVertexNormals();

    const material = new THREE.MeshBasicMaterial({
      color: 0xff8c1a,
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
    if (!this.previewMesh || !this.previewEdges || !this.basePositions || !this.perVertexNormal) return;
    const n = this.basePositions.length;
    const out = new Float32Array(n);
    for (let i = 0; i < n; i++) {
      out[i] = this.basePositions[i] + this.perVertexNormal[i] * this.currentDistance;
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
    this.perVertexNormal = null;
  }
}
