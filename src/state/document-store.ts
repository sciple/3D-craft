import { invoke } from "@tauri-apps/api/core";

// Mirrors the slotmap key shape Rust's `serde` feature produces for
// FaceId/GroupId ({ idx, version }) - see src-tauri/src/geometry/mesh.rs
// and src-tauri/src/scene/document.rs. Treat these as opaque tokens: pass
// them back to commands as-is, compare via faceIdKey/groupIdKey.
export interface FaceId {
  idx: number;
  version: number;
}
export type GroupId = FaceId;

export interface FaceSnapshot {
  id: FaceId;
  group_id: GroupId | null;
  triangles: [number, number, number][];
  /// Outer boundary loop, as indices into DocumentSnapshot.vertices - the
  /// visible/snappable edges, as opposed to `triangles`, which also includes
  /// invisible ear-clipping diagonals.
  outer: number[];
  /// Hole boundary loops, same indexing.
  holes: number[][];
  normal: [number, number, number];
}

export interface GroupSnapshot {
  id: GroupId;
  name: string;
}

export interface DocumentSnapshot {
  vertices: [number, number, number][];
  faces: FaceSnapshot[];
  groups: GroupSnapshot[];
  selected_face_ids: FaceId[];
}

/// One offending edge from `checkModel`, in world coordinates rather than as
/// indices into `DocumentSnapshot.vertices` - see the Rust `ProblemEdge` for
/// why (snapshot vertex interning is per-call and order-dependent).
export interface ProblemEdge {
  a: [number, number, number];
  b: [number, number, number];
}

/// Result of the watertightness check STL export gates on - see
/// `Document::check_model` on the Rust side.
export interface ModelReport {
  /// Connected printable solids found in the document.
  part_count: number;
  /// How many of those have at least one issue.
  broken_part_count: number;
  open_edges: ProblemEdge[];
  duplicate_edges: ProblemEdge[];
  problem_face_ids: FaceId[];
}

export function reportHasProblems(report: ModelReport): boolean {
  return report.open_edges.length > 0 || report.duplicate_edges.length > 0;
}

export function faceIdKey(id: FaceId): string {
  return `${id.idx}:${id.version}`;
}

type Vec3 = [number, number, number];
type Vec2 = [number, number];
type Listener = (snapshot: DocumentSnapshot) => void;
type ReportListener = (report: ModelReport | null) => void;

const EMPTY_SNAPSHOT: DocumentSnapshot = { vertices: [], faces: [], groups: [], selected_face_ids: [] };

class DocumentStore {
  private snapshot: DocumentSnapshot = EMPTY_SNAPSHOT;
  private listeners = new Set<Listener>();
  // Tracks whether the document has changes not yet written to a project
  // file - drives the close-guard's Save/Discard/Cancel prompt (see
  // ui/close-guard.ts). Only commands that touch persisted content
  // (ProjectFile stores vertices/faces/groups, not selection) should mark
  // this true - see `applyEdit` vs. the plain `apply` used by
  // selectFaces/selectGroup/refresh.
  private dirty = false;
  // The watertightness problems currently being shown in the viewport (see
  // `showModelProblems`), or null when nothing is highlighted. Kept apart
  // from `snapshot` because it isn't produced by the mutating commands -
  // it's an on-demand diagnostic the user asks for, and it must survive the
  // selection-only snapshots that arrive while they click around the model.
  private report: ModelReport | null = null;
  private reportListeners = new Set<ReportListener>();
  // Every command is chained through this so they execute (and apply their
  // resulting snapshot) in strict call order, never overlapping. Without
  // this, two commands fired in quick succession (e.g. a draw click
  // immediately followed by a select click) could have their IPC responses
  // arrive out of order and let a stale snapshot clobber a newer one.
  private queue: Promise<unknown> = Promise.resolve();

  private enqueue<T>(task: () => Promise<T>): Promise<T> {
    const result = this.queue.then(task, task);
    this.queue = result.then(
      () => undefined,
      () => undefined,
    );
    return result;
  }

  getSnapshot(): DocumentSnapshot {
    return this.snapshot;
  }

  isDirty(): boolean {
    return this.dirty;
  }

  subscribe(listener: Listener): () => void {
    this.listeners.add(listener);
    return () => this.listeners.delete(listener);
  }

  private apply(snapshot: DocumentSnapshot) {
    this.snapshot = snapshot;
    for (const listener of this.listeners) listener(snapshot);
  }

  /// Same as `apply`, but also marks the document dirty - use for every
  /// command that changes persisted content (geometry/groups), never for
  /// selection-only commands.
  private applyEdit(snapshot: DocumentSnapshot) {
    this.dirty = true;
    // Any geometry change can fix or move the problems that were
    // highlighted, so drop the overlay rather than leave stale red edges
    // floating where the geometry no longer is. Plain `apply` deliberately
    // doesn't, so the highlight survives selecting/orbiting.
    this.showModelProblems(null);
    this.apply(snapshot);
  }

  subscribeModelReport(listener: ReportListener): () => void {
    this.reportListeners.add(listener);
    return () => this.reportListeners.delete(listener);
  }

  getModelReport(): ModelReport | null {
    return this.report;
  }

  /// Shows (or, with null, clears) the problem-edge highlight in the
  /// viewport. Separate from `checkModel` so running the check doesn't
  /// force a highlight the user didn't ask for.
  showModelProblems(report: ModelReport | null) {
    if (this.report === null && report === null) return;
    this.report = report;
    for (const listener of this.reportListeners) listener(report);
  }

  /// Runs the watertightness diagnostic STL export gates on. Read-only: it
  /// neither mutates the document nor touches the undo history.
  checkModel(): Promise<ModelReport> {
    return this.enqueue(() => invoke<ModelReport>("check_model"));
  }

  refresh() {
    return this.enqueue(async () => {
      this.apply(await invoke<DocumentSnapshot>("get_document"));
    });
  }

  undo() {
    return this.enqueue(async () => {
      this.applyEdit(await invoke<DocumentSnapshot>("undo"));
    });
  }

  redo() {
    return this.enqueue(async () => {
      this.applyEdit(await invoke<DocumentSnapshot>("redo"));
    });
  }

  /// `targetFaceId`: when the sketch was drawn directly on top of an
  /// existing solid face (see `tools/plane.ts`'s `resolveSketchTarget`),
  /// the new loop splits just that face instead of triggering a
  /// document-wide coplanar resplit - see `Document::resplit` on the Rust
  /// side.
  drawRectangle(planeOrigin: Vec3, planeNormal: Vec3, cornerA: Vec2, cornerB: Vec2, targetFaceId?: FaceId) {
    return this.enqueue(async () => {
      this.applyEdit(
        await invoke<DocumentSnapshot>("draw_rectangle", {
          planeOrigin,
          planeNormal,
          cornerA,
          cornerB,
          targetFaceId: targetFaceId ?? null,
        }),
      );
    });
  }

  drawCircle(planeOrigin: Vec3, planeNormal: Vec3, center: Vec2, radius: number, segments: number, targetFaceId?: FaceId) {
    return this.enqueue(async () => {
      this.applyEdit(
        await invoke<DocumentSnapshot>("draw_circle", {
          planeOrigin,
          planeNormal,
          center,
          radius,
          segments,
          targetFaceId: targetFaceId ?? null,
        }),
      );
    });
  }

  /// `startAngleDeg`/`sweepDeg`: the arc runs from `startAngleDeg` through
  /// `startAngleDeg + sweepDeg` and is closed with a straight chord between
  /// its two endpoints (no center vertex) - see `add_arc` on the Rust side
  /// for why the chord closure was chosen over a center-connected pie/wedge.
  drawArc(
    planeOrigin: Vec3,
    planeNormal: Vec3,
    center: Vec2,
    radius: number,
    startAngleDeg: number,
    sweepDeg: number,
    segments: number,
    targetFaceId?: FaceId,
  ) {
    return this.enqueue(async () => {
      this.applyEdit(
        await invoke<DocumentSnapshot>("draw_arc", {
          planeOrigin,
          planeNormal,
          center,
          radius,
          startAngleDeg,
          sweepDeg,
          segments,
          targetFaceId: targetFaceId ?? null,
        }),
      );
    });
  }

  /// `startAngleDeg`: rotation of the polygon's first vertex, mirroring
  /// `drawArc`'s convention - the tool derives it from the click that sets
  /// the radius, so one vertex lands under the cursor.
  drawNgon(
    planeOrigin: Vec3,
    planeNormal: Vec3,
    center: Vec2,
    radius: number,
    sides: number,
    startAngleDeg: number,
    targetFaceId?: FaceId,
  ) {
    return this.enqueue(async () => {
      this.applyEdit(
        await invoke<DocumentSnapshot>("draw_ngon", {
          planeOrigin,
          planeNormal,
          center,
          radius,
          sides,
          startAngleDeg,
          targetFaceId: targetFaceId ?? null,
        }),
      );
    });
  }

  drawPolygon(planeOrigin: Vec3, planeNormal: Vec3, points: Vec2[], targetFaceId?: FaceId) {
    return this.enqueue(async () => {
      this.applyEdit(
        await invoke<DocumentSnapshot>("draw_polygon", {
          planeOrigin,
          planeNormal,
          points,
          targetFaceId: targetFaceId ?? null,
        }),
      );
    });
  }

  pushPullFace(faceId: FaceId, distance: number) {
    return this.enqueue(async () => {
      this.applyEdit(await invoke<DocumentSnapshot>("push_pull_face", { faceId, distance }));
    });
  }

  pushPullFaces(faceIds: FaceId[], distance: number) {
    return this.enqueue(async () => {
      this.applyEdit(await invoke<DocumentSnapshot>("push_pull_faces", { faceIds, distance }));
    });
  }

  insetFace(faceId: FaceId, offset: number) {
    return this.enqueue(async () => {
      this.applyEdit(await invoke<DocumentSnapshot>("inset_face", { faceId, offset }));
    });
  }

  eraseFace(faceId: FaceId) {
    return this.enqueue(async () => {
      this.applyEdit(await invoke<DocumentSnapshot>("erase_face", { faceId }));
    });
  }

  moveFaces(faceIds: FaceId[], delta: Vec3) {
    return this.enqueue(async () => {
      this.applyEdit(await invoke<DocumentSnapshot>("move_faces", { faceIds, delta }));
    });
  }

  rotateFaces(faceIds: FaceId[], pivot: Vec3, axis: Vec3, angleRadians: number) {
    return this.enqueue(async () => {
      this.applyEdit(await invoke<DocumentSnapshot>("rotate_faces", { faceIds, pivot, axis, angleRadians }));
    });
  }

  scaleFaces(faceIds: FaceId[], pivot: Vec3, scale: Vec3) {
    return this.enqueue(async () => {
      this.applyEdit(await invoke<DocumentSnapshot>("scale_faces", { faceIds, pivot, scale }));
    });
  }

  duplicateFaces(faceIds: FaceId[], delta: Vec3) {
    return this.enqueue(async () => {
      this.applyEdit(await invoke<DocumentSnapshot>("duplicate_faces", { faceIds, delta }));
    });
  }

  /// Mirrors a *copy* of `faceIds` across the world plane perpendicular to
  /// `axis` through `pivot` - the source geometry is left untouched. See
  /// `Document::mirror_faces` for why a copy (matches SketchUp's Mirror).
  mirrorFaces(faceIds: FaceId[], axis: "x" | "y" | "z", pivot: Vec3) {
    return this.enqueue(async () => {
      this.applyEdit(await invoke<DocumentSnapshot>("mirror_faces", { faceIds, axis, pivot }));
    });
  }

  groupFaces(faceIds: FaceId[], name: string) {
    return this.enqueue(async () => {
      this.applyEdit(await invoke<DocumentSnapshot>("group_faces", { faceIds, name }));
    });
  }

  ungroup(groupId: GroupId) {
    return this.enqueue(async () => {
      this.applyEdit(await invoke<DocumentSnapshot>("ungroup", { groupId }));
    });
  }

  selectGroup(groupId: GroupId) {
    return this.enqueue(async () => {
      this.apply(await invoke<DocumentSnapshot>("select_group", { groupId }));
    });
  }

  selectFaces(faceIds: FaceId[]) {
    return this.enqueue(async () => {
      this.apply(await invoke<DocumentSnapshot>("select_faces", { faceIds }));
    });
  }

  saveProject(path: string) {
    return this.enqueue(async () => {
      await invoke<void>("save_project", { path });
      this.dirty = false;
    });
  }

  loadProject(path: string) {
    return this.enqueue(async () => {
      this.apply(await invoke<DocumentSnapshot>("load_project", { path }));
      this.dirty = false;
      // Uses `apply`, not `applyEdit`, so clear the highlight explicitly -
      // the incoming document has nothing to do with whatever was flagged
      // in the one being replaced.
      this.showModelProblems(null);
    });
  }

  exportStl(path: string) {
    return this.enqueue(() => invoke<void>("export_stl", { path }));
  }

  /// Moves every disconnected printable solid onto a floor-aligned,
  /// non-overlapping grid - see `Document::arrange_for_print` on the Rust
  /// side. Rejects (like `exportStl`) if there's nothing printable yet.
  arrangeForPrint() {
    return this.enqueue(async () => {
      this.applyEdit(await invoke<DocumentSnapshot>("arrange_for_print"));
    });
  }
}

export const documentStore = new DocumentStore();
