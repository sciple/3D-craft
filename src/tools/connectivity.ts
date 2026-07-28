import type { DocumentSnapshot, FaceId } from "../state/document-store";
import { faceIdKey } from "../state/document-store";

function edgeKey(a: number, b: number): string {
  return a < b ? `${a}:${b}` : `${b}:${a}`;
}

function buildEdgeToFaces(snapshot: DocumentSnapshot): Map<string, number[]> {
  const edgeToFaces = new Map<string, number[]>();
  snapshot.faces.forEach((face, index) => {
    for (const loop of [face.outer, ...face.holes]) {
      for (let i = 0; i < loop.length; i++) {
        const key = edgeKey(loop[i], loop[(i + 1) % loop.length]);
        const list = edgeToFaces.get(key);
        if (list) list.push(index);
        else edgeToFaces.set(key, [index]);
      }
    }
  });
  return edgeToFaces;
}

/// Flood-fills from `startIndex` over shared edges, adding every reached
/// face index into `visited` (shared across calls, so re-touching an
/// already-visited object is a no-op).
function floodFillComponent(
  snapshot: DocumentSnapshot,
  edgeToFaces: Map<string, number[]>,
  startIndex: number,
  visited: Set<number>,
) {
  if (visited.has(startIndex)) return;
  visited.add(startIndex);
  const stack = [startIndex];
  while (stack.length > 0) {
    const current = stack.pop()!;
    const face = snapshot.faces[current];
    for (const loop of [face.outer, ...face.holes]) {
      for (let i = 0; i < loop.length; i++) {
        const key = edgeKey(loop[i], loop[(i + 1) % loop.length]);
        for (const neighbor of edgeToFaces.get(key) ?? []) {
          if (!visited.has(neighbor)) {
            visited.add(neighbor);
            stack.push(neighbor);
          }
        }
      }
    }
  }
}

function faceIndex(snapshot: DocumentSnapshot, id: FaceId): number {
  return snapshot.faces.findIndex((f) => faceIdKey(f.id) === faceIdKey(id));
}

/// All faces reachable from `startId` by walking shared edges (outer or
/// hole loops) - the "whole object" a double/triple-click grabs in
/// Blender/SketchUp, e.g. every face of a box or hull in one go. Pure mesh
/// connectivity, independent of groups - a solid the user hasn't grouped
/// yet is still selectable as a unit this way.
export function connectedFaceIds(snapshot: DocumentSnapshot, startId: FaceId): FaceId[] {
  const startIndex = faceIndex(snapshot, startId);
  if (startIndex === -1) return [startId];

  const visited = new Set<number>();
  floodFillComponent(snapshot, buildEdgeToFaces(snapshot), startIndex, visited);
  return [...visited].map((i) => snapshot.faces[i].id);
}

/// All faces belonging to any connected object touched by `faceIds` - the
/// union of `connectedFaceIds` over every id, deduped. Used by box-select:
/// touching any part of an object with the marquee brings its whole
/// connected object into the selection, not just the faces the rectangle
/// happened to cross.
export function expandToConnectedObjects(snapshot: DocumentSnapshot, faceIds: FaceId[]): FaceId[] {
  const edgeToFaces = buildEdgeToFaces(snapshot);
  const visited = new Set<number>();
  const result: FaceId[] = [];
  const resultKeys = new Set<string>();

  const addOnce = (id: FaceId) => {
    const key = faceIdKey(id);
    if (resultKeys.has(key)) return;
    resultKeys.add(key);
    result.push(id);
  };

  for (const id of faceIds) {
    const startIndex = faceIndex(snapshot, id);
    if (startIndex === -1) {
      addOnce(id); // unknown id (stale snapshot) - keep it as-is, matching connectedFaceIds' fallback
      continue;
    }
    if (visited.has(startIndex)) continue;

    const before = new Set(visited);
    floodFillComponent(snapshot, edgeToFaces, startIndex, visited);
    for (const i of visited) {
      if (!before.has(i)) addOnce(snapshot.faces[i].id);
    }
  }

  return result;
}
