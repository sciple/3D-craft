import type { DocumentSnapshot, FaceId } from "../state/document-store";
import { faceIdKey } from "../state/document-store";

function edgeKey(a: number, b: number): string {
  return a < b ? `${a}:${b}` : `${b}:${a}`;
}

/// All faces reachable from `startId` by walking shared edges (outer or
/// hole loops) - the "whole object" a double/triple-click grabs in
/// Blender/SketchUp, e.g. every face of a box or hull in one go. Pure mesh
/// connectivity, independent of groups - a solid the user hasn't grouped
/// yet is still selectable as a unit this way.
export function connectedFaceIds(snapshot: DocumentSnapshot, startId: FaceId): FaceId[] {
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

  const startIndex = snapshot.faces.findIndex((f) => faceIdKey(f.id) === faceIdKey(startId));
  if (startIndex === -1) return [startId];

  const visited = new Set<number>([startIndex]);
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

  return [...visited].map((i) => snapshot.faces[i].id);
}
