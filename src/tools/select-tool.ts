import * as THREE from "three";
import { documentStore, faceIdKey } from "../state/document-store";
import type { Tool, ToolContext } from "./types";
import { pointerToNdc } from "./types";

export class SelectTool implements Tool {
  readonly name = "select";
  private raycaster = new THREE.Raycaster();

  onPointerDown(e: PointerEvent, ctx: ToolContext) {
    if (e.button !== 0) return; // left click only; middle/right stay free for camera/context use

    const ndc = pointerToNdc(e, ctx.domElement);
    this.raycaster.setFromCamera(ndc, ctx.camera);
    const hits = this.raycaster.intersectObject(ctx.meshRenderer.mesh);
    const hit = hits[0];

    if (!hit || hit.faceIndex == null) {
      if (!e.shiftKey) void documentStore.selectFaces([]);
      return;
    }

    const faceId = ctx.meshRenderer.faceIdForTriangle(hit.faceIndex);
    if (!faceId) return;

    const current = documentStore.getSnapshot().selected_face_ids;
    if (e.shiftKey) {
      const key = faceIdKey(faceId);
      const alreadySelected = current.some((f) => faceIdKey(f) === key);
      const next = alreadySelected
        ? current.filter((f) => faceIdKey(f) !== key)
        : [...current, faceId];
      void documentStore.selectFaces(next);
    } else {
      void documentStore.selectFaces([faceId]);
    }
  }

  async onKeyDown(e: KeyboardEvent) {
    if (e.key !== "Delete" && e.key !== "Backspace") return;
    const selected = [...documentStore.getSnapshot().selected_face_ids];
    // Sequential awaits: each erase call returns the full post-erase
    // snapshot, so awaiting in order guarantees the final applied snapshot
    // reflects every erasure rather than racing concurrent IPC calls.
    for (const faceId of selected) {
      await documentStore.eraseFace(faceId);
    }
  }
}
