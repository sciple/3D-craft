import * as THREE from "three";
import { documentStore } from "../state/document-store";
import { PreviewLine } from "./preview-line";
import type { Tool, ToolContext } from "./types";
import { raycastGroundPlaneSnapped, SnapIndicator } from "./snapping";

const PLANE_ORIGIN: [number, number, number] = [0, 0, 0];
const PLANE_NORMAL: [number, number, number] = [0, 0, 1];

/// Click-click rectangle tool: first click sets one corner, second click sets
/// the opposite corner and commits. Draws on the ground plane only in v1 -
/// drawing on an arbitrary existing face's plane is a natural extension the
/// command layer already supports (draw_rectangle takes any plane), just not
/// wired up to a "click a face to set the active plane" UI yet.
export class DrawRectangleTool implements Tool {
  readonly name = "rectangle";
  private raycaster = new THREE.Raycaster();
  private startPoint: THREE.Vector3 | null = null;
  private preview = new PreviewLine();
  private snapIndicator = new SnapIndicator();

  /// Called after a shape is committed, so main.ts can switch back to the
  /// Select tool - otherwise a click meant to select the shape you just
  /// drew is instead read as the start of a new one.
  constructor(private onCommitted?: () => void) {}

  activate() {
    this.startPoint = null;
  }

  deactivate(ctx: ToolContext) {
    this.startPoint = null;
    this.preview.clear(ctx.scene);
    this.snapIndicator.hide();
  }

  onPointerDown(e: PointerEvent, ctx: ToolContext) {
    if (e.button !== 0) return;
    const hit = raycastGroundPlaneSnapped(e, ctx, this.raycaster);
    if (!hit) return;
    const point = hit.point;

    if (!this.startPoint) {
      this.startPoint = point;
      return;
    }

    const a = this.startPoint;
    const b = point;
    this.startPoint = null;
    this.preview.clear(ctx.scene);
    this.snapIndicator.hide();
    if (a.distanceTo(b) > 1e-6) {
      void documentStore.drawRectangle(PLANE_ORIGIN, PLANE_NORMAL, [a.x, a.y], [b.x, b.y]);
      this.onCommitted?.();
    }
  }

  onPointerMove(e: PointerEvent, ctx: ToolContext) {
    const hit = raycastGroundPlaneSnapped(e, ctx, this.raycaster);
    if (!hit) return;
    this.snapIndicator.update(ctx.scene, ctx.camera, hit.snap);
    if (!this.startPoint) return;
    const a = this.startPoint;
    const b = hit.point;
    this.preview.update(ctx.scene, [
      new THREE.Vector3(a.x, a.y, 0),
      new THREE.Vector3(b.x, a.y, 0),
      new THREE.Vector3(b.x, b.y, 0),
      new THREE.Vector3(a.x, b.y, 0),
      new THREE.Vector3(a.x, a.y, 0),
    ]);
  }

  onKeyDown(e: KeyboardEvent, ctx: ToolContext) {
    if (e.key === "Escape") {
      this.startPoint = null;
      this.preview.clear(ctx.scene);
    }
  }
}

/// Click-click circle tool: first click sets the center, second click sets
/// the radius (distance to the click point) and commits.
export class DrawCircleTool implements Tool {
  readonly name = "circle";
  private static readonly SEGMENTS = 32;

  private raycaster = new THREE.Raycaster();
  private center: THREE.Vector3 | null = null;
  private preview = new PreviewLine();
  private snapIndicator = new SnapIndicator();

  constructor(private onCommitted?: () => void) {}

  activate() {
    this.center = null;
  }

  deactivate(ctx: ToolContext) {
    this.center = null;
    this.preview.clear(ctx.scene);
    this.snapIndicator.hide();
  }

  onPointerDown(e: PointerEvent, ctx: ToolContext) {
    if (e.button !== 0) return;
    const hit = raycastGroundPlaneSnapped(e, ctx, this.raycaster);
    if (!hit) return;
    const point = hit.point;

    if (!this.center) {
      this.center = point;
      return;
    }

    const center = this.center;
    const radius = center.distanceTo(point);
    this.center = null;
    this.preview.clear(ctx.scene);
    this.snapIndicator.hide();
    if (radius > 1e-6) {
      void documentStore.drawCircle(
        PLANE_ORIGIN,
        PLANE_NORMAL,
        [center.x, center.y],
        radius,
        DrawCircleTool.SEGMENTS,
      );
      this.onCommitted?.();
    }
  }

  onPointerMove(e: PointerEvent, ctx: ToolContext) {
    const hit = raycastGroundPlaneSnapped(e, ctx, this.raycaster);
    if (!hit) return;
    this.snapIndicator.update(ctx.scene, ctx.camera, hit.snap);
    if (!this.center) return;
    const radius = this.center.distanceTo(hit.point);
    const points: THREE.Vector3[] = [];
    for (let i = 0; i <= DrawCircleTool.SEGMENTS; i++) {
      const angle = (i / DrawCircleTool.SEGMENTS) * Math.PI * 2;
      points.push(
        new THREE.Vector3(
          this.center.x + Math.cos(angle) * radius,
          this.center.y + Math.sin(angle) * radius,
          0,
        ),
      );
    }
    this.preview.update(ctx.scene, points);
  }

  onKeyDown(e: KeyboardEvent, ctx: ToolContext) {
    if (e.key === "Escape") {
      this.center = null;
      this.preview.clear(ctx.scene);
    }
  }
}

/// Click-per-vertex polygon tool (SketchUp's "Line" tool, but oriented
/// around building a closed face rather than one edge at a time): each
/// click adds a point; clicking back near the start point, or pressing
/// Enter, closes the loop and commits it. Point click order (CW/CCW)
/// doesn't matter - resplit_plane's face_detect pass re-derives correct
/// orientation from the undirected edge graph either way.
export class DrawPolygonTool implements Tool {
  readonly name = "polygon";
  private static readonly CLOSE_DISTANCE_PX = 12;

  private raycaster = new THREE.Raycaster();
  private points: THREE.Vector3[] = [];
  private firstClickPixels: { x: number; y: number } | null = null;
  private preview = new PreviewLine();
  private snapIndicator = new SnapIndicator();

  constructor(private onCommitted?: () => void) {}

  activate() {
    this.points = [];
    this.firstClickPixels = null;
  }

  deactivate(ctx: ToolContext) {
    this.reset(ctx);
  }

  onPointerDown(e: PointerEvent, ctx: ToolContext) {
    if (e.button !== 0) return;

    if (this.points.length >= 3 && this.firstClickPixels && this.isNearFirstClick(e)) {
      this.commit(ctx);
      return;
    }

    const hit = raycastGroundPlaneSnapped(e, ctx, this.raycaster);
    if (!hit) return;

    if (this.points.length === 0) {
      this.firstClickPixels = { x: e.clientX, y: e.clientY };
    }
    this.points.push(hit.point);
  }

  onPointerMove(e: PointerEvent, ctx: ToolContext) {
    const hit = raycastGroundPlaneSnapped(e, ctx, this.raycaster);
    if (!hit) return;
    this.snapIndicator.update(ctx.scene, ctx.camera, hit.snap);
    if (this.points.length === 0) return;
    this.preview.update(ctx.scene, [...this.points, hit.point]);
  }

  onKeyDown(e: KeyboardEvent, ctx: ToolContext) {
    if (e.key === "Escape") {
      this.reset(ctx);
    } else if (e.key === "Enter") {
      this.commit(ctx);
    }
  }

  private isNearFirstClick(e: PointerEvent): boolean {
    if (!this.firstClickPixels) return false;
    const dx = e.clientX - this.firstClickPixels.x;
    const dy = e.clientY - this.firstClickPixels.y;
    return Math.sqrt(dx * dx + dy * dy) < DrawPolygonTool.CLOSE_DISTANCE_PX;
  }

  private commit(ctx: ToolContext) {
    if (this.points.length >= 3) {
      const points2d: [number, number][] = this.points.map((p) => [p.x, p.y]);
      void documentStore.drawPolygon(PLANE_ORIGIN, PLANE_NORMAL, points2d);
      this.onCommitted?.();
    }
    this.reset(ctx);
  }

  private reset(ctx: ToolContext) {
    this.points = [];
    this.firstClickPixels = null;
    this.preview.clear(ctx.scene);
    this.snapIndicator.hide();
  }
}
