import * as THREE from "three";
import { documentStore } from "../state/document-store";
import { PreviewLine } from "./preview-line";
import type { Tool, ToolContext } from "./types";
import { findPlanarSnap, raycastPlaneSnapped, SnapIndicator } from "./snapping";
import type { SketchPlane } from "./plane";
import { resolveSketchTarget, to2d, to3d } from "./plane";
import { NumericBuffer } from "./numeric-input";
import { measurementHud } from "../ui/measurement-hud";

/// Click-click rectangle tool: first click resolves and locks the sketch
/// plane (an existing solid face if the click hit one, else the ground
/// plane - see `resolveSketchTarget`) and sets one corner; second click
/// sets the opposite corner and commits. A typed number (or "W,H"/"WxH")
/// while dragging overrides the mouse-driven size - see `NumericBuffer`.
export class DrawRectangleTool implements Tool {
  readonly name = "rectangle";
  private raycaster = new THREE.Raycaster();
  private activePlane: SketchPlane | null = null;
  private startUv: THREE.Vector2 | null = null;
  private currentUv: THREE.Vector2 | null = null;
  private numeric = new NumericBuffer();
  private preview = new PreviewLine();
  private snapIndicator = new SnapIndicator();

  /// Called after a shape is committed, so main.ts can switch back to the
  /// Select tool - otherwise a click meant to select the shape you just
  /// drew is instead read as the start of a new one.
  constructor(private onCommitted?: () => void) {}

  activate() {
    this.activePlane = null;
    this.startUv = null;
    this.currentUv = null;
    this.numeric.clear();
  }

  deactivate(ctx: ToolContext) {
    this.reset(ctx);
  }

  onPointerDown(e: PointerEvent, ctx: ToolContext) {
    if (e.button !== 0) return;

    if (!this.activePlane) {
      const target = resolveSketchTarget(e, ctx, this.raycaster);
      if (!target) return;
      const snap = findPlanarSnap(documentStore.getSnapshot(), target.point, target.plane, ctx.camera, ctx.domElement);
      const point = snap ? snap.point : target.point;
      this.activePlane = target.plane;
      this.startUv = to2d(target.plane, point);
      this.currentUv = this.startUv.clone();
      return;
    }

    this.commit(ctx);
  }

  onPointerMove(e: PointerEvent, ctx: ToolContext) {
    if (!this.activePlane) {
      const target = resolveSketchTarget(e, ctx, this.raycaster);
      if (!target) return;
      const snap = findPlanarSnap(documentStore.getSnapshot(), target.point, target.plane, ctx.camera, ctx.domElement);
      this.snapIndicator.update(ctx.scene, ctx.camera, snap);
      return;
    }

    const hit = raycastPlaneSnapped(e, ctx, this.raycaster, this.activePlane);
    if (!hit) return;
    this.snapIndicator.update(ctx.scene, ctx.camera, hit.snap);
    this.currentUv = to2d(this.activePlane, hit.point);
    this.updatePreview(ctx);
  }

  onKeyDown(e: KeyboardEvent, ctx: ToolContext) {
    if (this.activePlane && this.startUv) {
      if (e.key === "Enter") {
        if (!this.numeric.isEmpty) this.commit(ctx);
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
    if (e.key === "Escape") {
      this.reset(ctx);
    }
  }

  private effectiveEndUv(): THREE.Vector2 | null {
    if (!this.startUv || !this.currentUv) return null;
    if (this.numeric.isEmpty) return this.currentUv;
    const values = this.numeric.values();
    const w = values[0];
    if (w === undefined) return this.currentUv;
    const h = values[1] ?? Math.abs(this.currentUv.y - this.startUv.y);
    const sx = this.currentUv.x >= this.startUv.x ? 1 : -1;
    const sy = this.currentUv.y >= this.startUv.y ? 1 : -1;
    return new THREE.Vector2(this.startUv.x + sx * w, this.startUv.y + sy * h);
  }

  private updatePreview(ctx: ToolContext) {
    const plane = this.activePlane;
    const a = this.startUv;
    const b = this.effectiveEndUv();
    if (!plane || !a || !b) return;
    this.preview.update(ctx.scene, [
      to3d(plane, new THREE.Vector2(a.x, a.y)),
      to3d(plane, new THREE.Vector2(b.x, a.y)),
      to3d(plane, new THREE.Vector2(b.x, b.y)),
      to3d(plane, new THREE.Vector2(a.x, b.y)),
      to3d(plane, new THREE.Vector2(a.x, a.y)),
    ]);
    const w = Math.abs(b.x - a.x);
    const h = Math.abs(b.y - a.y);
    measurementHud.show("Rect", `${w.toFixed(1)} × ${h.toFixed(1)} mm`, this.numeric.isEmpty ? null : this.numeric.display);
  }

  private commit(ctx: ToolContext) {
    const plane = this.activePlane;
    const a = this.startUv;
    const b = this.effectiveEndUv();
    this.reset(ctx);
    if (!plane || !a || !b || a.distanceTo(b) < 1e-6) return;
    void documentStore.drawRectangle(
      [plane.origin.x, plane.origin.y, plane.origin.z],
      [plane.normal.x, plane.normal.y, plane.normal.z],
      [a.x, a.y],
      [b.x, b.y],
      plane.faceId ?? undefined,
    );
    this.onCommitted?.();
  }

  private reset(ctx: ToolContext) {
    this.activePlane = null;
    this.startUv = null;
    this.currentUv = null;
    this.numeric.clear();
    this.preview.clear(ctx.scene);
    this.snapIndicator.hide();
    measurementHud.hide();
  }
}

/// Click-click circle tool: first click resolves the sketch plane and sets
/// the center, second click sets the radius (distance to the click point)
/// and commits. A typed number while dragging overrides the radius.
export class DrawCircleTool implements Tool {
  readonly name = "circle";
  private static readonly SEGMENTS = 32;

  private raycaster = new THREE.Raycaster();
  private activePlane: SketchPlane | null = null;
  private center: THREE.Vector2 | null = null;
  private currentUv: THREE.Vector2 | null = null;
  private numeric = new NumericBuffer();
  private preview = new PreviewLine();
  private snapIndicator = new SnapIndicator();

  constructor(private onCommitted?: () => void) {}

  activate() {
    this.activePlane = null;
    this.center = null;
    this.currentUv = null;
    this.numeric.clear();
  }

  deactivate(ctx: ToolContext) {
    this.reset(ctx);
  }

  onPointerDown(e: PointerEvent, ctx: ToolContext) {
    if (e.button !== 0) return;

    if (!this.activePlane) {
      const target = resolveSketchTarget(e, ctx, this.raycaster);
      if (!target) return;
      const snap = findPlanarSnap(documentStore.getSnapshot(), target.point, target.plane, ctx.camera, ctx.domElement);
      const point = snap ? snap.point : target.point;
      this.activePlane = target.plane;
      this.center = to2d(target.plane, point);
      this.currentUv = this.center.clone();
      return;
    }

    this.commit(ctx);
  }

  onPointerMove(e: PointerEvent, ctx: ToolContext) {
    if (!this.activePlane) {
      const target = resolveSketchTarget(e, ctx, this.raycaster);
      if (!target) return;
      const snap = findPlanarSnap(documentStore.getSnapshot(), target.point, target.plane, ctx.camera, ctx.domElement);
      this.snapIndicator.update(ctx.scene, ctx.camera, snap);
      return;
    }

    const hit = raycastPlaneSnapped(e, ctx, this.raycaster, this.activePlane);
    if (!hit) return;
    this.snapIndicator.update(ctx.scene, ctx.camera, hit.snap);
    this.currentUv = to2d(this.activePlane, hit.point);
    this.updatePreview(ctx);
  }

  onKeyDown(e: KeyboardEvent, ctx: ToolContext) {
    if (this.activePlane && this.center) {
      if (e.key === "Enter") {
        if (!this.numeric.isEmpty) this.commit(ctx);
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
    if (e.key === "Escape") {
      this.reset(ctx);
    }
  }

  private effectiveRadius(): number | null {
    if (!this.center || !this.currentUv) return null;
    if (this.numeric.isEmpty) return this.center.distanceTo(this.currentUv);
    const r = this.numeric.values()[0];
    return r === undefined ? this.center.distanceTo(this.currentUv) : r;
  }

  private updatePreview(ctx: ToolContext) {
    const plane = this.activePlane;
    const center = this.center;
    const radius = this.effectiveRadius();
    if (!plane || !center || radius === null) return;
    const points: THREE.Vector3[] = [];
    for (let i = 0; i <= DrawCircleTool.SEGMENTS; i++) {
      const angle = (i / DrawCircleTool.SEGMENTS) * Math.PI * 2;
      points.push(to3d(plane, new THREE.Vector2(center.x + Math.cos(angle) * radius, center.y + Math.sin(angle) * radius)));
    }
    this.preview.update(ctx.scene, points);
    measurementHud.show("Circle radius", `${radius.toFixed(1)} mm`, this.numeric.isEmpty ? null : this.numeric.display);
  }

  private commit(ctx: ToolContext) {
    const plane = this.activePlane;
    const center = this.center;
    const radius = this.effectiveRadius();
    this.reset(ctx);
    if (!plane || !center || radius === null || radius < 1e-6) return;
    void documentStore.drawCircle(
      [plane.origin.x, plane.origin.y, plane.origin.z],
      [plane.normal.x, plane.normal.y, plane.normal.z],
      [center.x, center.y],
      radius,
      DrawCircleTool.SEGMENTS,
      plane.faceId ?? undefined,
    );
    this.onCommitted?.();
  }

  private reset(ctx: ToolContext) {
    this.activePlane = null;
    this.center = null;
    this.currentUv = null;
    this.numeric.clear();
    this.preview.clear(ctx.scene);
    this.snapIndicator.hide();
    measurementHud.hide();
  }
}

/// Click-click regular-polygon tool (5-8 sides): first click resolves the
/// sketch plane and sets the center (like Circle); second click sets BOTH
/// the radius and the rotation from the vector center->click point, so one
/// vertex lands exactly under the cursor (SketchUp's polygon convention,
/// and the same "click defines a vertex" idea `DrawArcTool` uses for its
/// start angle). Side count defaults to 6 and is changed with Arrow
/// Up/Down while the tool is active - arrow keys pass straight through
/// `NumericBuffer.type()` untouched, so there's no conflict with a typed
/// radius override.
export class DrawNgonTool implements Tool {
  readonly name = "ngon";
  private static readonly MIN_SIDES = 5;
  private static readonly MAX_SIDES = 8;
  private static readonly SIDE_NAMES: Record<number, string> = {
    5: "Pentagon",
    6: "Hexagon",
    7: "Heptagon",
    8: "Octagon",
  };

  private raycaster = new THREE.Raycaster();
  private activePlane: SketchPlane | null = null;
  private center: THREE.Vector2 | null = null;
  private currentUv: THREE.Vector2 | null = null;
  private sides = 6;
  private numeric = new NumericBuffer();
  private preview = new PreviewLine();
  private snapIndicator = new SnapIndicator();

  constructor(private onCommitted?: () => void) {}

  activate() {
    this.activePlane = null;
    this.center = null;
    this.currentUv = null;
    this.sides = 6;
    this.numeric.clear();
  }

  deactivate(ctx: ToolContext) {
    this.reset(ctx);
  }

  onPointerDown(e: PointerEvent, ctx: ToolContext) {
    if (e.button !== 0) return;

    if (!this.activePlane) {
      const target = resolveSketchTarget(e, ctx, this.raycaster);
      if (!target) return;
      const snap = findPlanarSnap(documentStore.getSnapshot(), target.point, target.plane, ctx.camera, ctx.domElement);
      const point = snap ? snap.point : target.point;
      this.activePlane = target.plane;
      this.center = to2d(target.plane, point);
      this.currentUv = this.center.clone();
      return;
    }

    this.commit(ctx);
  }

  onPointerMove(e: PointerEvent, ctx: ToolContext) {
    if (!this.activePlane) {
      const target = resolveSketchTarget(e, ctx, this.raycaster);
      if (!target) return;
      const snap = findPlanarSnap(documentStore.getSnapshot(), target.point, target.plane, ctx.camera, ctx.domElement);
      this.snapIndicator.update(ctx.scene, ctx.camera, snap);
      return;
    }

    const hit = raycastPlaneSnapped(e, ctx, this.raycaster, this.activePlane);
    if (!hit) return;
    this.snapIndicator.update(ctx.scene, ctx.camera, hit.snap);
    this.currentUv = to2d(this.activePlane, hit.point);
    this.updatePreview(ctx);
  }

  onKeyDown(e: KeyboardEvent, ctx: ToolContext) {
    if (this.activePlane && this.center) {
      if (e.key === "ArrowUp" || e.key === "ArrowDown") {
        const delta = e.key === "ArrowUp" ? 1 : -1;
        this.sides = THREE.MathUtils.clamp(this.sides + delta, DrawNgonTool.MIN_SIDES, DrawNgonTool.MAX_SIDES);
        this.updatePreview(ctx);
        return;
      }
      if (e.key === "Enter") {
        if (!this.numeric.isEmpty) this.commit(ctx);
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
    if (e.key === "Escape") {
      this.reset(ctx);
    }
  }

  private effectiveRadius(): number | null {
    if (!this.center || !this.currentUv) return null;
    if (this.numeric.isEmpty) return this.center.distanceTo(this.currentUv);
    const r = this.numeric.values()[0];
    return r === undefined ? this.center.distanceTo(this.currentUv) : r;
  }

  /// Rotation always follows the mouse, even when the radius is typed -
  /// mirrors `DrawArcTool.currentAngleRad`.
  private currentAngleRad(): number {
    if (!this.center || !this.currentUv) return 0;
    return Math.atan2(this.currentUv.y - this.center.y, this.currentUv.x - this.center.x);
  }

  private updatePreview(ctx: ToolContext) {
    const plane = this.activePlane;
    const center = this.center;
    const radius = this.effectiveRadius();
    if (!plane || !center || radius === null) return;
    const startAngle = this.currentAngleRad();
    const points: THREE.Vector3[] = [];
    for (let i = 0; i <= this.sides; i++) {
      const angle = startAngle + (i / this.sides) * Math.PI * 2;
      points.push(to3d(plane, new THREE.Vector2(center.x + Math.cos(angle) * radius, center.y + Math.sin(angle) * radius)));
    }
    this.preview.update(ctx.scene, points);
    const name = DrawNgonTool.SIDE_NAMES[this.sides] ?? `${this.sides}-gon`;
    measurementHud.show(`${name} radius (↑↓ sides)`, `${radius.toFixed(1)} mm`, this.numeric.isEmpty ? null : this.numeric.display);
  }

  private commit(ctx: ToolContext) {
    const plane = this.activePlane;
    const center = this.center;
    const radius = this.effectiveRadius();
    const startAngleDeg = THREE.MathUtils.radToDeg(this.currentAngleRad());
    const sides = this.sides;
    this.reset(ctx);
    if (!plane || !center || radius === null || radius < 1e-6) return;
    void documentStore.drawNgon(
      [plane.origin.x, plane.origin.y, plane.origin.z],
      [plane.normal.x, plane.normal.y, plane.normal.z],
      [center.x, center.y],
      radius,
      sides,
      startAngleDeg,
      plane.faceId ?? undefined,
    );
    this.onCommitted?.();
  }

  private reset(ctx: ToolContext) {
    this.activePlane = null;
    this.center = null;
    this.currentUv = null;
    this.sides = 6;
    this.numeric.clear();
    this.preview.clear(ctx.scene);
    this.snapIndicator.hide();
    measurementHud.hide();
  }
}

/// Click-click-click arc tool: first click resolves the sketch plane and
/// sets the center (like Circle); second click sets the radius and start
/// angle (a typed number overrides the radius, mirroring
/// `DrawCircleTool.effectiveRadius`); third click (or a typed degree value +
/// Enter) sweeps the arc from the start angle and commits. The arc is closed
/// with a straight chord between its two endpoints rather than through a
/// center vertex - see `add_arc` on the Rust side for why (a center-vertex
/// "pie" closure has a numerically fragile case at exactly 180 degrees, the
/// sweep a half-pipe cross-section needs most).
export class DrawArcTool implements Tool {
  readonly name = "arc";
  private static readonly FULL_CIRCLE_SEGMENTS = 32;

  private raycaster = new THREE.Raycaster();
  private activePlane: SketchPlane | null = null;
  private center: THREE.Vector2 | null = null;
  private currentUv: THREE.Vector2 | null = null;
  private radius: number | null = null;
  private startAngleRad: number | null = null;
  private numeric = new NumericBuffer();
  private preview = new PreviewLine();
  private snapIndicator = new SnapIndicator();

  constructor(private onCommitted?: () => void) {}

  activate() {
    this.activePlane = null;
    this.center = null;
    this.currentUv = null;
    this.radius = null;
    this.startAngleRad = null;
    this.numeric.clear();
  }

  deactivate(ctx: ToolContext) {
    this.reset(ctx);
  }

  onPointerDown(e: PointerEvent, ctx: ToolContext) {
    if (e.button !== 0) return;

    if (!this.activePlane) {
      const target = resolveSketchTarget(e, ctx, this.raycaster);
      if (!target) return;
      const snap = findPlanarSnap(documentStore.getSnapshot(), target.point, target.plane, ctx.camera, ctx.domElement);
      const point = snap ? snap.point : target.point;
      this.activePlane = target.plane;
      this.center = to2d(target.plane, point);
      this.currentUv = this.center.clone();
      return;
    }

    if (this.radius === null) {
      this.commitRadiusStage(ctx);
      return;
    }

    this.commit(ctx);
  }

  onPointerMove(e: PointerEvent, ctx: ToolContext) {
    if (!this.activePlane) {
      const target = resolveSketchTarget(e, ctx, this.raycaster);
      if (!target) return;
      const snap = findPlanarSnap(documentStore.getSnapshot(), target.point, target.plane, ctx.camera, ctx.domElement);
      this.snapIndicator.update(ctx.scene, ctx.camera, snap);
      return;
    }

    const hit = raycastPlaneSnapped(e, ctx, this.raycaster, this.activePlane);
    if (!hit) return;
    if (this.radius === null) {
      this.snapIndicator.update(ctx.scene, ctx.camera, hit.snap);
    } else {
      this.snapIndicator.hide();
    }
    this.currentUv = to2d(this.activePlane, hit.point);
    this.updatePreview(ctx);
  }

  onKeyDown(e: KeyboardEvent, ctx: ToolContext) {
    if (this.activePlane && this.center) {
      if (e.key === "Enter") {
        if (!this.numeric.isEmpty) {
          if (this.radius === null) this.commitRadiusStage(ctx);
          else this.commit(ctx);
        }
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
    if (e.key === "Escape") {
      this.reset(ctx);
    }
  }

  private effectiveRadius(): number | null {
    if (!this.center || !this.currentUv) return null;
    if (this.numeric.isEmpty) return this.center.distanceTo(this.currentUv);
    const r = this.numeric.values()[0];
    return r === undefined ? this.center.distanceTo(this.currentUv) : r;
  }

  private currentAngleRad(): number {
    if (!this.center || !this.currentUv) return 0;
    return Math.atan2(this.currentUv.y - this.center.y, this.currentUv.x - this.center.x);
  }

  /// Mirrors `RotateTool.effectiveAngle`'s convention exactly (a plain typed
  /// number follows the current drag's sign, an explicit '-' overrides it),
  /// just staying in degrees since that's what the backend expects.
  private effectiveSweepDeg(): number {
    if (this.startAngleRad === null) return 0;
    const dragSweepDeg = THREE.MathUtils.radToDeg(this.currentAngleRad() - this.startAngleRad);
    if (this.numeric.isEmpty) return dragSweepDeg;
    const v = this.numeric.values()[0];
    if (v === undefined) return dragSweepDeg;
    if (this.numeric.display.includes("-")) return -Math.abs(v);
    return dragSweepDeg < 0 ? -Math.abs(v) : Math.abs(v);
  }

  private static segmentsForSweep(sweepDeg: number): number {
    return Math.max(2, Math.round((Math.abs(sweepDeg) / 360) * DrawArcTool.FULL_CIRCLE_SEGMENTS));
  }

  /// Locks in the radius (typed override or mouse distance) and the start
  /// angle (always mouse-derived), then moves on to the sweep stage.
  private commitRadiusStage(ctx: ToolContext) {
    const radius = this.effectiveRadius();
    if (!this.center || !this.currentUv || radius === null || radius < 1e-6) return;
    this.radius = radius;
    this.startAngleRad = this.currentAngleRad();
    this.numeric.clear();
    this.updatePreview(ctx);
  }

  private updatePreview(ctx: ToolContext) {
    const plane = this.activePlane;
    const center = this.center;
    if (!plane || !center) return;

    if (this.radius === null) {
      const radius = this.effectiveRadius();
      if (radius === null || !this.currentUv) return;
      const dir = this.currentUv.clone().sub(center);
      if (dir.lengthSq() < 1e-12) dir.set(1, 0);
      else dir.normalize();
      const endUv = center.clone().addScaledVector(dir, radius);
      this.preview.update(ctx.scene, [to3d(plane, center), to3d(plane, endUv)]);
      measurementHud.show("Arc radius", `${radius.toFixed(1)} mm`, this.numeric.isEmpty ? null : this.numeric.display);
      return;
    }

    if (this.startAngleRad === null) return;
    const sweepDeg = this.effectiveSweepDeg();
    const segments = DrawArcTool.segmentsForSweep(sweepDeg);
    const sweepRad = THREE.MathUtils.degToRad(sweepDeg);
    const radius = this.radius;
    const startAngleRad = this.startAngleRad;
    const points: THREE.Vector3[] = [];
    for (let i = 0; i <= segments; i++) {
      const angle = startAngleRad + sweepRad * (i / segments);
      points.push(to3d(plane, new THREE.Vector2(center.x + Math.cos(angle) * radius, center.y + Math.sin(angle) * radius)));
    }
    points.push(points[0].clone()); // close back to the arc's start via the chord
    this.preview.update(ctx.scene, points);
    measurementHud.show("Arc sweep", `${sweepDeg.toFixed(1)}°`, this.numeric.isEmpty ? null : this.numeric.display);
  }

  private commit(ctx: ToolContext) {
    const plane = this.activePlane;
    const center = this.center;
    const radius = this.radius;
    const startAngleRad = this.startAngleRad;
    const sweepDeg = this.effectiveSweepDeg();
    this.reset(ctx);
    if (!plane || !center || radius === null || startAngleRad === null) return;
    if (Math.abs(sweepDeg) < 1e-3 || Math.abs(sweepDeg) > 359.5) return;
    const segments = DrawArcTool.segmentsForSweep(sweepDeg);
    const startAngleDeg = THREE.MathUtils.radToDeg(startAngleRad);
    void documentStore.drawArc(
      [plane.origin.x, plane.origin.y, plane.origin.z],
      [plane.normal.x, plane.normal.y, plane.normal.z],
      [center.x, center.y],
      radius,
      startAngleDeg,
      sweepDeg,
      segments,
      plane.faceId ?? undefined,
    );
    this.onCommitted?.();
  }

  private reset(ctx: ToolContext) {
    this.activePlane = null;
    this.center = null;
    this.currentUv = null;
    this.radius = null;
    this.startAngleRad = null;
    this.numeric.clear();
    this.preview.clear(ctx.scene);
    this.snapIndicator.hide();
    measurementHud.hide();
  }
}

/// Click-per-vertex polygon tool (SketchUp's "Line" tool, but oriented
/// around building a closed face rather than one edge at a time): each
/// click adds a point on the locked sketch plane; clicking back near the
/// start point closes the loop and commits. Point click order (CW/CCW)
/// doesn't matter - resplit_plane's face_detect pass re-derives correct
/// orientation from the undirected edge graph either way.
///
/// A typed number while placing a point overrides that segment's length
/// (measured from the last placed point, along the current mouse
/// direction): Enter with a non-empty typed buffer places that point
/// without closing the loop; Enter with an empty buffer closes it (the
/// pre-existing shortcut), matching how a second click behaves either way.
export class DrawPolygonTool implements Tool {
  readonly name = "polygon";
  private static readonly CLOSE_DISTANCE_PX = 12;

  private raycaster = new THREE.Raycaster();
  private activePlane: SketchPlane | null = null;
  private pointsUv: THREE.Vector2[] = [];
  private currentUv: THREE.Vector2 | null = null;
  private firstClickPixels: { x: number; y: number } | null = null;
  private numeric = new NumericBuffer();
  private preview = new PreviewLine();
  private snapIndicator = new SnapIndicator();

  constructor(private onCommitted?: () => void) {}

  activate() {
    this.activePlane = null;
    this.pointsUv = [];
    this.currentUv = null;
    this.firstClickPixels = null;
    this.numeric.clear();
  }

  deactivate(ctx: ToolContext) {
    this.reset(ctx);
  }

  onPointerDown(e: PointerEvent, ctx: ToolContext) {
    if (e.button !== 0) return;

    if (this.pointsUv.length >= 3 && this.firstClickPixels && this.isNearFirstClick(e)) {
      this.commit(ctx);
      return;
    }

    if (!this.activePlane) {
      const target = resolveSketchTarget(e, ctx, this.raycaster);
      if (!target) return;
      const snap = findPlanarSnap(documentStore.getSnapshot(), target.point, target.plane, ctx.camera, ctx.domElement);
      const point = snap ? snap.point : target.point;
      this.activePlane = target.plane;
      this.firstClickPixels = { x: e.clientX, y: e.clientY };
      this.pointsUv.push(to2d(target.plane, point));
      this.currentUv = this.pointsUv[0].clone();
      return;
    }

    const hit = raycastPlaneSnapped(e, ctx, this.raycaster, this.activePlane);
    if (!hit) return;
    this.currentUv = to2d(this.activePlane, hit.point);
    const next = this.effectiveNextUv();
    if (next) this.pointsUv.push(next);
    this.numeric.clear();
    this.updatePreview(ctx);
  }

  onPointerMove(e: PointerEvent, ctx: ToolContext) {
    if (!this.activePlane) {
      const target = resolveSketchTarget(e, ctx, this.raycaster);
      if (!target) return;
      const snap = findPlanarSnap(documentStore.getSnapshot(), target.point, target.plane, ctx.camera, ctx.domElement);
      this.snapIndicator.update(ctx.scene, ctx.camera, snap);
      return;
    }

    const hit = raycastPlaneSnapped(e, ctx, this.raycaster, this.activePlane);
    if (!hit) return;
    this.snapIndicator.update(ctx.scene, ctx.camera, hit.snap);
    this.currentUv = to2d(this.activePlane, hit.point);
    this.updatePreview(ctx);
  }

  onKeyDown(e: KeyboardEvent, ctx: ToolContext) {
    if (this.activePlane && this.pointsUv.length > 0) {
      if (e.key === "Enter" && !this.numeric.isEmpty) {
        const next = this.effectiveNextUv();
        if (next) this.pointsUv.push(next);
        this.numeric.clear();
        this.updatePreview(ctx);
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

  private effectiveNextUv(): THREE.Vector2 | null {
    if (!this.currentUv || this.pointsUv.length === 0) return null;
    if (this.numeric.isEmpty) return this.currentUv;
    const dist = this.numeric.values()[0];
    if (dist === undefined) return this.currentUv;
    const last = this.pointsUv[this.pointsUv.length - 1];
    const dir = this.currentUv.clone().sub(last);
    if (dir.lengthSq() < 1e-12) return this.currentUv;
    dir.normalize();
    return last.clone().addScaledVector(dir, dist);
  }

  private updatePreview(ctx: ToolContext) {
    const plane = this.activePlane;
    if (!plane || this.pointsUv.length === 0) return;
    const next = this.effectiveNextUv();
    const allUv = next ? [...this.pointsUv, next] : this.pointsUv;
    this.preview.update(ctx.scene, allUv.map((p) => to3d(plane, p)));
    if (next) {
      const last = this.pointsUv[this.pointsUv.length - 1];
      const length = last.distanceTo(next);
      measurementHud.show("Segment", `${length.toFixed(1)} mm`, this.numeric.isEmpty ? null : this.numeric.display);
    }
  }

  private commit(ctx: ToolContext) {
    const plane = this.activePlane;
    const pointsUv = this.pointsUv;
    this.reset(ctx);
    if (!plane || pointsUv.length < 3) return;
    const points2d: [number, number][] = pointsUv.map((p) => [p.x, p.y]);
    void documentStore.drawPolygon(
      [plane.origin.x, plane.origin.y, plane.origin.z],
      [plane.normal.x, plane.normal.y, plane.normal.z],
      points2d,
      plane.faceId ?? undefined,
    );
    this.onCommitted?.();
  }

  private reset(ctx: ToolContext) {
    this.activePlane = null;
    this.pointsUv = [];
    this.currentUv = null;
    this.firstClickPixels = null;
    this.numeric.clear();
    this.preview.clear(ctx.scene);
    this.snapIndicator.hide();
    measurementHud.hide();
  }
}
