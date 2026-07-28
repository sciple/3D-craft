import * as THREE from "three";
import { documentStore, faceIdKey } from "../state/document-store";
import type { FaceId } from "../state/document-store";
import type { Tool, ToolContext } from "./types";
import { pointerToNdc } from "./types";
import { connectedFaceIds, expandToConnectedObjects } from "./connectivity";
import { facesInRect } from "./rect-select";
import type { ScreenRect } from "./rect-select";
import { selectionMarquee } from "../ui/selection-marquee";

// Multi-click detection, matching Blender/SketchUp's convention of a
// double/triple-click "grabbing" the whole connected object instead of just
// the one clicked face. Tracked by wall-clock time + screen-space distance
// between consecutive left-clicks, not by browser dblclick (which doesn't
// give us a click count beyond 2, and we drive it off raycasts anyway).
const MULTI_CLICK_MS = 400;
const MULTI_CLICK_PX = 5;

// How far the pointer must move (in client pixels) after a miss before a
// box-select drag starts, rather than being treated as a plain click on
// empty space. Distinct constant from MULTI_CLICK_PX even though the value
// matches - this one gates drag-vs-click, not click-repeat detection.
const DRAG_THRESHOLD_PX = 5;

function ndcRectFrom(x0: number, y0: number, x1: number, y1: number, domElement: HTMLElement): ScreenRect {
  const rect = domElement.getBoundingClientRect();
  const toNdc = (x: number, y: number) => ({
    x: ((x - rect.left) / rect.width) * 2 - 1,
    y: -((y - rect.top) / rect.height) * 2 + 1,
  });
  const a = toNdc(x0, y0);
  const b = toNdc(x1, y1);
  return {
    minX: Math.min(a.x, b.x),
    maxX: Math.max(a.x, b.x),
    minY: Math.min(a.y, b.y),
    maxY: Math.max(a.y, b.y),
  };
}

export class SelectTool implements Tool {
  readonly name = "select";
  private raycaster = new THREE.Raycaster();
  private lastClickTime = 0;
  private lastClickX = 0;
  private lastClickY = 0;
  private clickCount = 0;

  // Box-select drag state, only meaningful between a missed pointerdown and
  // the matching pointerup.
  private pointerDownWasMiss = false;
  private dragging = false;
  private dragStartX = 0;
  private dragStartY = 0;
  private dragShift = false;

  private mergeIntoSelection(current: FaceId[], additions: FaceId[]): FaceId[] {
    const currentKeys = new Set(current.map(faceIdKey));
    const merged = [...current];
    for (const f of additions) if (!currentKeys.has(faceIdKey(f))) merged.push(f);
    return merged;
  }

  onPointerDown(e: PointerEvent, ctx: ToolContext) {
    if (e.button !== 0) return; // left click only; middle/right stay free for camera/context use

    const ndc = pointerToNdc(e, ctx.domElement);
    this.raycaster.setFromCamera(ndc, ctx.camera);
    const hits = this.raycaster.intersectObject(ctx.meshRenderer.mesh);
    const hit = hits[0];

    if (!hit || hit.faceIndex == null) {
      this.clickCount = 0;
      // Don't decide the outcome yet - a drag past the threshold turns this
      // into a box-select instead of a click-to-clear; see onPointerUp.
      this.pointerDownWasMiss = true;
      this.dragging = false;
      this.dragStartX = e.clientX;
      this.dragStartY = e.clientY;
      this.dragShift = e.shiftKey;
      return;
    }
    this.pointerDownWasMiss = false;

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
        void documentStore.selectFaces(this.mergeIntoSelection(current, connected));
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

  onPointerMove(e: PointerEvent) {
    if (!this.pointerDownWasMiss) return;
    const dx = e.clientX - this.dragStartX;
    const dy = e.clientY - this.dragStartY;
    if (!this.dragging && Math.hypot(dx, dy) < DRAG_THRESHOLD_PX) return;
    this.dragging = true;
    selectionMarquee.show(this.dragStartX, this.dragStartY, e.clientX, e.clientY);
  }

  onPointerUp(e: PointerEvent, ctx: ToolContext) {
    if (!this.pointerDownWasMiss) return;
    selectionMarquee.hide();

    if (!this.dragging) {
      // Never crossed the drag threshold: a plain click on empty space,
      // same as before this feature existed.
      if (!this.dragShift) void documentStore.selectFaces([]);
      this.pointerDownWasMiss = false;
      return;
    }

    this.dragging = false;
    this.pointerDownWasMiss = false;

    const rect = ndcRectFrom(this.dragStartX, this.dragStartY, e.clientX, e.clientY, ctx.domElement);
    const snapshot = documentStore.getSnapshot();
    const touched = facesInRect(snapshot, rect, ctx.camera);
    // Touching any part of an object brings its whole connected object
    // into the selection, not just the faces the rectangle happened to
    // cross - mirrors double-click's "grab the whole object" behavior.
    const raw = expandToConnectedObjects(snapshot, touched);

    const current = snapshot.selected_face_ids;
    if (this.dragShift) {
      void documentStore.selectFaces(this.mergeIntoSelection(current, raw));
    } else {
      void documentStore.selectFaces(raw);
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
