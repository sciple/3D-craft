import * as THREE from "three";
import type { Tool, ToolContext } from "./types";
import { PreviewLine } from "./preview-line";
import { SnapIndicator, raycastSnapped3d } from "./snapping";
import { measurementHud } from "../ui/measurement-hud";
import { documentStore } from "../state/document-store";

/// A tape measure: click two points to read the straight-line distance between
/// them (plus the X/Y/Z component deltas), SketchUp-style. It creates no
/// document geometry, but the finished segment is recorded as a persistent
/// guide (`documentStore.addGuide`) - undoable, saved with the project, and
/// drawn/snapped-to by `GuideRenderer`/`snapping.ts` so a later shape can be
/// built exactly on a distance you just measured.
///
/// Points snap to model vertices/midpoints/edges - and existing guides - via
/// `raycastSnapped3d`, and otherwise fall on the hovered surface or the
/// ground plane, so you can measure between exact corners or to a free
/// point. First click sets the start; second click finalizes, commits the
/// guide, and leaves the readout on screen; the next click starts a fresh
/// measurement. Esc cancels only an in-progress measurement - it never
/// touches guides already committed; use Clear Guides (in the outliner) for
/// that.
export class MeasureTool implements Tool {
  readonly name = "measure";
  private raycaster = new THREE.Raycaster();
  private first: THREE.Vector3 | null = null;
  private line = new PreviewLine();
  private indicator = new SnapIndicator();

  deactivate(ctx: ToolContext) {
    this.reset(ctx);
    this.indicator.hide();
  }

  onPointerMove(e: PointerEvent, ctx: ToolContext) {
    const pick = raycastSnapped3d(e, ctx, this.raycaster);
    this.indicator.update(ctx.scene, ctx.camera, pick?.snap ?? null);
    if (!pick || !this.first) return;
    this.line.update(ctx.scene, [this.first, pick.point]);
    this.showReadout(this.first, pick.point);
  }

  onPointerDown(e: PointerEvent, ctx: ToolContext) {
    if (e.button !== 0) return;
    const pick = raycastSnapped3d(e, ctx, this.raycaster);
    if (!pick) return;
    if (!this.first) {
      this.first = pick.point.clone();
      this.showReadout(this.first, this.first);
    } else {
      const start = this.first;
      const end = pick.point.clone();
      this.showReadout(start, end);
      // Reset synchronously, before the (queued, async) invoke: a second
      // click landing while the round trip is still pending must already
      // see a clean state, ready to start the next measurement.
      this.first = null;
      // The finished segment becomes a persistent guide, drawn by
      // GuideRenderer off the next snapshot - drop the transient preview so
      // the segment isn't rendered twice (once solid amber, once dashed).
      this.line.clear(ctx.scene);
      if (start.distanceTo(end) > 1e-6) {
        void documentStore.addGuide([start.x, start.y, start.z], [end.x, end.y, end.z]);
      }
    }
  }

  onKeyDown(e: KeyboardEvent, ctx: ToolContext) {
    // Cancels only the in-progress measurement (the not-yet-clicked second
    // point). Committed guides are undoable/persistent document state now -
    // only Clear Guides (outliner) removes them, never Esc.
    if (e.key === "Escape") this.reset(ctx);
  }

  private reset(ctx: ToolContext) {
    this.first = null;
    this.line.clear(ctx.scene);
    measurementHud.hide();
  }

  private showReadout(a: THREE.Vector3, b: THREE.Vector3) {
    const fmt = (n: number) => n.toFixed(1);
    const dist = a.distanceTo(b);
    const dx = Math.abs(b.x - a.x);
    const dy = Math.abs(b.y - a.y);
    const dz = Math.abs(b.z - a.z);
    measurementHud.show("Distance", `${fmt(dist)} mm   ΔX ${fmt(dx)}  ΔY ${fmt(dy)}  ΔZ ${fmt(dz)}`, null);
  }
}
