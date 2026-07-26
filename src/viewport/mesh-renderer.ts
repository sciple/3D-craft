import * as THREE from "three";
import type { DocumentSnapshot, FaceId, ModelReport } from "../state/document-store";
import { faceIdKey } from "../state/document-store";

/// Renders the document's triangulated geometry as one mesh, plus a second
/// overlay mesh built from just the selected faces' triangles. Painting
/// selection via a separate overlay (rather than per-vertex colors on the
/// main mesh) avoids color bleeding onto neighboring unselected faces that
/// happen to share a vertex (e.g. a selected cap and an unselected side wall
/// of the same solid).
export class MeshRenderer {
  readonly mesh: THREE.Mesh;
  readonly highlightMesh: THREE.Mesh;
  readonly edges: THREE.LineSegments;
  readonly problemEdges: THREE.LineSegments;
  private triangleFaceIds: FaceId[] = [];

  constructor(scene: THREE.Scene) {
    const material = new THREE.MeshStandardMaterial({
      color: 0x8fb8ff,
      side: THREE.DoubleSide,
      metalness: 0.1,
      roughness: 0.7,
      // Faces from the same solid legitimately share vertices at their
      // shared edges (that's what makes push/pull watertight), but a hard
      // edge - e.g. a flat cap meeting a wall at a hole's rim - should never
      // shade smoothly across it. flatShading computes each triangle's
      // normal independently instead of averaging normals at shared
      // vertices, which also matches SketchUp's own faceted look.
      flatShading: true,
      // Pushes filled faces back slightly in depth so the coincident edge
      // lines drawn on top don't z-fight/flicker against them.
      polygonOffset: true,
      polygonOffsetFactor: 1,
      polygonOffsetUnits: 1,
    });
    this.mesh = new THREE.Mesh(new THREE.BufferGeometry(), material);
    scene.add(this.mesh);

    const highlightMaterial = new THREE.MeshBasicMaterial({
      color: 0xff8c1a,
      side: THREE.DoubleSide,
      transparent: true,
      opacity: 0.55,
      depthTest: true,
      polygonOffset: true,
      polygonOffsetFactor: 1,
      polygonOffsetUnits: 1,
    });
    this.highlightMesh = new THREE.Mesh(new THREE.BufferGeometry(), highlightMaterial);
    this.highlightMesh.renderOrder = 1;
    scene.add(this.highlightMesh);

    // Face boundary edges - the visible black outlines SketchUp-style tools
    // draw on every face, and the visual reference this app needs for
    // precise modeling (and, in a later phase, for snapping).
    const edgeMaterial = new THREE.LineBasicMaterial({ color: 0x1a1a1a });
    this.edges = new THREE.LineSegments(new THREE.BufferGeometry(), edgeMaterial);
    this.edges.renderOrder = 2;
    scene.add(this.edges);

    // Watertightness problems (see `showProblems`). `depthTest: false` is
    // the point of this overlay: an open edge on the far side of a solid
    // has to be findable without orbiting to hunt for it, so these draw on
    // top of everything. It also has to carry the "this is the broken bit"
    // signal through color alone - LineBasicMaterial's `linewidth` is
    // ignored by ANGLE (the WebGL backend on Windows), so the lines can't
    // be made thicker.
    const problemMaterial = new THREE.LineBasicMaterial({ color: 0xff2020, depthTest: false });
    this.problemEdges = new THREE.LineSegments(new THREE.BufferGeometry(), problemMaterial);
    this.problemEdges.renderOrder = 4;
    this.problemEdges.visible = false;
    scene.add(this.problemEdges);
  }

  /// Draws a model-check report's offending edges in red, or clears the
  /// overlay when given null. Unlike the other three objects this one owns
  /// its own position buffer (the report carries world coordinates, not
  /// indices into `snapshot.vertices`), which is what lets it survive the
  /// selection-only snapshots `update` handles in between.
  showProblems(report: ModelReport | null) {
    const edges = report ? [...report.open_edges, ...report.duplicate_edges] : [];
    if (edges.length === 0) {
      this.problemEdges.visible = false;
      return;
    }
    const positions = new Float32Array(edges.length * 6);
    edges.forEach((edge, i) => {
      positions.set(edge.a, i * 6);
      positions.set(edge.b, i * 6 + 3);
    });
    this.problemEdges.geometry.setAttribute("position", new THREE.BufferAttribute(positions, 3));
    this.problemEdges.geometry.computeBoundingSphere();
    this.problemEdges.visible = true;
  }

  update(snapshot: DocumentSnapshot) {
    const positions = new Float32Array(snapshot.vertices.length * 3);
    snapshot.vertices.forEach((v, i) => {
      positions[i * 3] = v[0];
      positions[i * 3 + 1] = v[1];
      positions[i * 3 + 2] = v[2];
    });

    const selected = new Set(snapshot.selected_face_ids.map(faceIdKey));
    const indices: number[] = [];
    const highlightIndices: number[] = [];
    const edgeIndices: number[] = [];
    this.triangleFaceIds = [];

    for (const face of snapshot.faces) {
      const isSelected = selected.has(faceIdKey(face.id));
      for (const tri of face.triangles) {
        indices.push(tri[0], tri[1], tri[2]);
        this.triangleFaceIds.push(face.id);
        if (isSelected) {
          highlightIndices.push(tri[0], tri[1], tri[2]);
        }
      }
      for (const loop of [face.outer, ...face.holes]) {
        pushLoopSegments(edgeIndices, loop);
      }
    }

    setGeometry(this.mesh.geometry, positions, indices);
    setGeometry(this.highlightMesh.geometry, positions, highlightIndices);

    this.edges.geometry.setAttribute("position", new THREE.BufferAttribute(positions, 3));
    this.edges.geometry.setIndex(edgeIndices);
    this.edges.geometry.computeBoundingSphere();
  }

  /// Maps a three.js Raycaster intersection's `faceIndex` (a triangle index
  /// into this mesh's index buffer) back to the document FaceId it came from.
  faceIdForTriangle(triangleIndex: number): FaceId | undefined {
    return this.triangleFaceIds[triangleIndex];
  }
}

function pushLoopSegments(out: number[], loop: number[]) {
  for (let i = 0; i < loop.length; i++) {
    out.push(loop[i], loop[(i + 1) % loop.length]);
  }
}

function setGeometry(geometry: THREE.BufferGeometry, positions: Float32Array, indices: number[]) {
  geometry.setAttribute("position", new THREE.BufferAttribute(positions, 3));
  geometry.setIndex(indices);
  geometry.computeVertexNormals();
  geometry.computeBoundingSphere();
}
