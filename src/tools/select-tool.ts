import * as THREE from "three";
import { documentStore, faceIdKey } from "../state/document-store";
import type { Tool, ToolContext } from "./types";
import { pointerToNdc } from "./types";
import { connectedFaceIds } from "./connectivity";

// Multi-click detection, matching Blender/SketchUp's convention of a
// double/triple-click "grabbing" the whole connected object instead of just
// the one clicked face. Tracked by wall-clock time + screen-space distance
// between consecutive left-clicks, not by browser dblclick (which doesn't
// give us a click count beyond 2, and we drive it off raycasts anyway).
const MULTI_CLICK_MS = 400;
const MULTI_CLICK_PX = 5;

export class SelectTool implements Tool {
  readonly name = "select";
  private raycaster = new THREE.Raycaster();
  private lastClickTime = 0;
  private lastClickX = 0;
  private lastClickY = 0;
  private clickCount = 0;

  onPointerDown(e: PointerEvent, ctx: ToolContext) {
    if (e.button !== 0) return; // left click only; middle/right stay free for camera/context use

    const ndc = pointerToNdc(e, ctx.domElement);
    this.raycaster.setFromCamera(ndc, ctx.camera);
    const hits = this.raycaster.intersectObject(ctx.meshRenderer.mesh);
    const hit = hits[0];

    if (!hit || hit.faceIndex == null) {
      this.clickCount = 0;
      if (!e.shiftKey) void documentStore.selectFaces([]);
      return;
    }

    const faceId = ctx.meshRenderer.faceIdForTriangle(hit.faceIndex);
    if (!faceId) return;

    const now = performance.now();
    const withinMultiClick =
      now - this.lastClickTime < MULTI_CLICK_MS &&
      Math.abs(e.clientX - this.lastClickX) < MULTI_CLICK_PX &&
      Math.abs(e.clientY - this.lastClickY) < MULTI_CLICK_PX;
    this.clickCount = withinMultiClick ? this.clickCount + 1 : 1;
    this.lastClickTime = now;
    this.lastClickX = e.clientX;
    this.lastClickY = e.clientY;

    const current = documentStore.getSnapshot().selected_face_ids;
    if (this.clickCount >= 2) {
      // Double (or further) click on the same spot: grab every face
      // connected to this one, i.e. the whole object.
      const connected = connectedFaceIds(documentStore.getSnapshot(), faceId);
      if (e.shiftKey) {
        const currentKeys = new Set(current.map(faceIdKey));
        const merged = [...current];
        for (const f of connected) if (!currentKeys.has(faceIdKey(f))) merged.push(f);
        void documentStore.selectFaces(merged);
      } else {
        void documentStore.selectFaces(connected);
      }
      return;
    }

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
    if ((e.ctrlKey || e.metaKey) && e.key.toLowerCase() === "d") {
      e.preventDefault();
      const selected = documentStore.getSnapshot().selected_face_ids;
      if (selected.length === 0) return;
      // A small visible nudge (rather than an exact overlap) so the copy
      // reads as a distinct object immediately - matches SketchUp's Copy,
      // and leaves the copy selected and ready for a follow-up Move.
      await documentStore.duplicateFaces(selected, [5, 5, 0]);
      return;
    }

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
