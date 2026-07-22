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

export function faceIdKey(id: FaceId): string {
  return `${id.idx}:${id.version}`;
}

type Vec3 = [number, number, number];
type Vec2 = [number, number];
type Listener = (snapshot: DocumentSnapshot) => void;

const EMPTY_SNAPSHOT: DocumentSnapshot = { vertices: [], faces: [], groups: [], selected_face_ids: [] };

class DocumentStore {
  private snapshot: DocumentSnapshot = EMPTY_SNAPSHOT;
  private listeners = new Set<Listener>();
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

  subscribe(listener: Listener): () => void {
    this.listeners.add(listener);
    return () => this.listeners.delete(listener);
  }

  private apply(snapshot: DocumentSnapshot) {
    this.snapshot = snapshot;
    for (const listener of this.listeners) listener(snapshot);
  }

  refresh() {
    return this.enqueue(async () => {
      this.apply(await invoke<DocumentSnapshot>("get_document"));
    });
  }

  undo() {
    return this.enqueue(async () => {
      this.apply(await invoke<DocumentSnapshot>("undo"));
    });
  }

  redo() {
    return this.enqueue(async () => {
      this.apply(await invoke<DocumentSnapshot>("redo"));
    });
  }

  /// `targetFaceId`: when the sketch was drawn directly on top of an
  /// existing solid face (see `tools/plane.ts`'s `resolveSketchTarget`),
  /// the new loop splits just that face instead of triggering a
  /// document-wide coplanar resplit - see `Document::resplit` on the Rust
  /// side.
  drawRectangle(planeOrigin: Vec3, planeNormal: Vec3, cornerA: Vec2, cornerB: Vec2, targetFaceId?: FaceId) {
    return this.enqueue(async () => {
      this.apply(
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
      this.apply(
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
      this.apply(
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

  drawPolygon(planeOrigin: Vec3, planeNormal: Vec3, points: Vec2[], targetFaceId?: FaceId) {
    return this.enqueue(async () => {
      this.apply(
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
      this.apply(await invoke<DocumentSnapshot>("push_pull_face", { faceId, distance }));
    });
  }

  pushPullFaces(faceIds: FaceId[], distance: number) {
    return this.enqueue(async () => {
      this.apply(await invoke<DocumentSnapshot>("push_pull_faces", { faceIds, distance }));
    });
  }

  insetFace(faceId: FaceId, offset: number) {
    return this.enqueue(async () => {
      this.apply(await invoke<DocumentSnapshot>("inset_face", { faceId, offset }));
    });
  }

  eraseFace(faceId: FaceId) {
    return this.enqueue(async () => {
      this.apply(await invoke<DocumentSnapshot>("erase_face", { faceId }));
    });
  }

  moveFaces(faceIds: FaceId[], delta: Vec3) {
    return this.enqueue(async () => {
      this.apply(await invoke<DocumentSnapshot>("move_faces", { faceIds, delta }));
    });
  }

  rotateFaces(faceIds: FaceId[], pivot: Vec3, axis: Vec3, angleRadians: number) {
    return this.enqueue(async () => {
      this.apply(await invoke<DocumentSnapshot>("rotate_faces", { faceIds, pivot, axis, angleRadians }));
    });
  }

  scaleFaces(faceIds: FaceId[], pivot: Vec3, scale: Vec3) {
    return this.enqueue(async () => {
      this.apply(await invoke<DocumentSnapshot>("scale_faces", { faceIds, pivot, scale }));
    });
  }

  duplicateFaces(faceIds: FaceId[], delta: Vec3) {
    return this.enqueue(async () => {
      this.apply(await invoke<DocumentSnapshot>("duplicate_faces", { faceIds, delta }));
    });
  }

  /// Mirrors a *copy* of `faceIds` across the world plane perpendicular to
  /// `axis` through `pivot` - the source geometry is left untouched. See
  /// `Document::mirror_faces` for why a copy (matches SketchUp's Mirror).
  mirrorFaces(faceIds: FaceId[], axis: "x" | "y" | "z", pivot: Vec3) {
    return this.enqueue(async () => {
      this.apply(await invoke<DocumentSnapshot>("mirror_faces", { faceIds, axis, pivot }));
    });
  }

  groupFaces(faceIds: FaceId[], name: string) {
    return this.enqueue(async () => {
      this.apply(await invoke<DocumentSnapshot>("group_faces", { faceIds, name }));
    });
  }

  ungroup(groupId: GroupId) {
    return this.enqueue(async () => {
      this.apply(await invoke<DocumentSnapshot>("ungroup", { groupId }));
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
    return this.enqueue(() => invoke<void>("save_project", { path }));
  }

  loadProject(path: string) {
    return this.enqueue(async () => {
      this.apply(await invoke<DocumentSnapshot>("load_project", { path }));
    });
  }

  exportStl(path: string) {
    return this.enqueue(() => invoke<void>("export_stl", { path }));
  }
}

export const documentStore = new DocumentStore();
