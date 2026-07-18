import * as THREE from "three";
import { documentStore, faceIdKey } from "../state/document-store";
import type { FaceId } from "../state/document-store";
import type { Tool, ToolContext } from "./types";
import { pointerToNdc } from "./types";
import { offsetPolygon2D } from "../geometry/inset-2d";
import { closestDistanceAlongAxis } from "./axis-drag";
import { NumericBuffer } from "./numeric-input";
import { measurementHud } from "../ui/measurement-hud";

/// Click a face, then drag toward its center to inset it by a variable
/// amount - releasing commits an `inset_face` call, which splits the face
/// into a shrunk inner face and an outer frame (the "Offset" workflow from
/// SketchUp, minus the separate duplicate-and-erase steps). Dragging away
/// from the center is clamped to zero: this tool only shrinks: growing a
/// face outward isn't a meaningful "inset" and the ring/erase workflow
/// already covers building things the other way around. A typed number
/// while dragging overrides the offset.
export class InsetTool implements Tool {
  readonly name = "inset";

  private raycaster = new THREE.Raycaster();
  private dragging = false;
  private faceId: FaceId | null = null;
  private axisOrigin = new THREE.Vector3();
  private axisDir = new THREE.Vector3();
  private currentOffset = 0;
  private numeric = new NumericBuffer();

  private basis: { origin: THREE.Vector3; u: THREE.Vector3; v: THREE.Vector3 } | null = null;
  private outer2d: THREE.Vector2[] = [];
  private preview: THREE.Line | null = null;

  onPointerDown(e: PointerEvent, ctx: ToolContext) {
    if (e.button !== 0) return;
    const ndc = pointerToNdc(e, ctx.domElement);
    this.raycaster.setFromCamera(ndc, ctx.camera);
    const hits = this.raycaster.intersectObject(ctx.meshRenderer.mesh);
    const hit = hits[0];
    if (!hit || hit.faceIndex == null || !hit.face) return;

    const faceId = ctx.meshRenderer.faceIdForTriangle(hit.faceIndex);
    if (!faceId) return;

    const snapshot = documentStore.getSnapshot();
    const face = snapshot.faces.find((f) => faceIdKey(f.id) === faceIdKey(faceId));
    if (!face || face.outer.length < 3) return;

    const outerPositions = face.outer.map((i) => new THREE.Vector3(...snapshot.vertices[i]));
    const centroid = outerPositions
      .reduce((sum, p) => sum.add(p), new THREE.Vector3())
      .divideScalar(outerPositions.length);

    const normal = new THREE.Vector3(...face.normal);
    const up = Math.abs(normal.x) < 0.9 ? new THREE.Vector3(1, 0, 0) : new THREE.Vector3(0, 1, 0);
    const v = new THREE.Vector3().crossVectors(normal, up).normalize();
    const u = new THREE.Vector3().crossVectors(v, normal).normalize();
    this.basis = { origin: outerPositions[0], u, v };
    this.outer2d = outerPositions.map((p) => this.to2d(p));

    this.faceId = faceId;
    this.axisOrigin.copy(hit.point);
    this.axisDir.copy(centroid).sub(hit.point).normalize();
    this.currentOffset = 0;
    this.numeric.clear();
    this.dragging = true;
    this.updatePreview(ctx);
  }

  onPointerMove(e: PointerEvent, ctx: ToolContext) {
    if (!this.dragging) return;
    const ndc = pointerToNdc(e, ctx.domElement);
    this.raycaster.setFromCamera(ndc, ctx.camera);
    const dragged = closestDistanceAlongAxis(
      this.axisOrigin,
      this.axisDir,
      this.raycaster.ray.origin,
      this.raycaster.ray.direction,
    );
    this.currentOffset = Math.max(0, dragged);
    this.updatePreview(ctx);
  }

  async onPointerUp() {
    if (!this.dragging) return;
    this.dragging = false;
    const faceId = this.faceId;
    const offset = this.effectiveOffset();
    this.faceId = null;
    this.clearPreview();
    if (faceId && offset > 1e-6) {
      await documentStore.insetFace(faceId, offset);
    }
  }

  onKeyDown(e: KeyboardEvent, ctx: ToolContext) {
    if (this.dragging) {
      if (e.key === "Enter") {
        if (!this.numeric.isEmpty) void this.onPointerUp();
        return;
      }
      if (e.key === "Escape" && !this.numeric.isEmpty) {
        this.numeric.clear();
        this.updatePreview(ctx);
        return;
      }
      if (this.numeric.type(e)) {
        this.updatePreview(ctx);
        return;
      }
    }
    if (e.key === "Escape" && this.dragging) {
      this.dragging = false;
      this.faceId = null;
      this.clearPreview();
    }
  }

  deactivate() {
    this.dragging = false;
    this.faceId = null;
    this.clearPreview();
  }

  private effectiveOffset(): number {
    if (this.numeric.isEmpty) return this.currentOffset;
    const v = this.numeric.values()[0];
    return v === undefined ? this.currentOffset : Math.max(0, Math.abs(v));
  }

  private to2d(p: THREE.Vector3): THREE.Vector2 {
    if (!this.basis) return new THREE.Vector2();
    const d = p.clone().sub(this.basis.origin);
    return new THREE.Vector2(d.dot(this.basis.u), d.dot(this.basis.v));
  }

  private to3d(p: THREE.Vector2): THREE.Vector3 {
    if (!this.basis) return new THREE.Vector3();
    return this.basis.origin
      .clone()
      .addScaledVector(this.basis.u, p.x)
      .addScaledVector(this.basis.v, p.y);
  }

  private updatePreview(ctx: ToolContext) {
    const offset = this.effectiveOffset();
    const inset2d = offsetPolygon2D(this.outer2d, offset);
    if (!inset2d) return; // offset too large for this shape - keep showing the last valid loop
    const points = [...inset2d, inset2d[0]].map((p) => this.to3d(p));
    const geometry = new THREE.BufferGeometry().setFromPoints(points);
    if (!this.preview) {
      this.preview = new THREE.Line(geometry, new THREE.LineBasicMaterial({ color: 0xff8c1a }));
      ctx.scene.add(this.preview);
    } else {
      this.preview.geometry.dispose();
      this.preview.geometry = geometry;
    }
    measurementHud.show("Inset", `${offset.toFixed(1)} mm`, this.numeric.isEmpty ? null : this.numeric.display);
  }

  private clearPreview() {
    if (this.preview) {
      this.preview.parent?.remove(this.preview);
      this.preview.geometry.dispose();
      (this.preview.material as THREE.Material).dispose();
      this.preview = null;
    }
    this.numeric.clear();
    measurementHud.hide();
  }
}
