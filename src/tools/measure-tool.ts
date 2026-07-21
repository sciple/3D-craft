import * as THREE from "three";
import type { Tool, ToolContext } from "./types";
import { PreviewLine } from "./preview-line";
import { SnapIndicator, raycastSnapped3d } from "./snapping";
import { measurementHud } from "../ui/measurement-hud";

/// A tape measure: click two points to read the straight-line distance between
/// them (plus the X/Y/Z component deltas), SketchUp-style. It's a pure
/// reference tool - it never creates or mutates geometry, so it issues no
/// backend commands at all; the segment and readout are client-side overlays.
///
/// Points snap to model vertices/midpoints/edges (via `raycastSnapped3d`) and
/// otherwise fall on the hovered surface or the ground plane, so you can
/// measure between exact corners or to a free point. First click sets the
/// start; second click finalizes and leaves the segment + readout on screen;
/// the next click starts a fresh measurement. Esc cancels an in-progress one.
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
      // Start a fresh measurement, clearing any previous persisted segment.
      this.line.clear(ctx.scene);
      this.first = pick.point.clone();
      this.showReadout(this.first, this.first);
    } else {
      // Second click finalizes: keep the segment + readout on screen, and arm
      // for a new measurement on the next click.
      this.line.update(ctx.scene, [this.first, pick.point]);
      this.showReadout(this.first, pick.point);
      this.first = null;
    }
  }

  onKeyDown(e: KeyboardEvent, ctx: ToolContext) {
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
