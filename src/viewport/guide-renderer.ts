import * as THREE from "three";
import type { DocumentSnapshot } from "../state/document-store";

/// Shared by the drawn guides here and by `SnapIndicator` in tools/snapping.ts,
/// so the live snap dot visually matches the guide it's snapping to.
export const GUIDE_COLOR = 0xff5ecb;

/// Renders the Measure tool's persistent guides: a dashed line per segment
/// plus a small square mark at each endpoint and midpoint. Deliberately a
/// separate object from `MeshRenderer` - guides aren't document geometry (they
/// never enter `Mesh`), carry their own world coordinates rather than indices
/// into `snapshot.vertices`, and must visually read as "not geometry" so they
/// can't be confused with model edges. Dashing is the only available signal
/// for that: `LineBasicMaterial.linewidth` is ignored by ANGLE on Windows
/// (see MeshRenderer's problemEdges), so thickness isn't an option.
export class GuideRenderer {
  readonly lines: THREE.LineSegments;
  readonly marks: THREE.Points;

  constructor(scene: THREE.Scene) {
    const lineMaterial = new THREE.LineDashedMaterial({
      color: GUIDE_COLOR,
      dashSize: 1.5,
      gapSize: 1.5,
      transparent: true,
      opacity: 0.9,
      depthTest: true,
    });
    this.lines = new THREE.LineSegments(new THREE.BufferGeometry(), lineMaterial);
    this.lines.renderOrder = 5;
    this.lines.visible = false;
    scene.add(this.lines);

    // depthTest: false, unlike the lines: the mark is the interactive
    // snap target and the snapping functions aren't occlusion-aware, so a
    // mark hidden behind a solid would be reachable but invisible - worse
    // than a visible one that's merely floating in front of geometry.
    const marksMaterial = new THREE.PointsMaterial({
      color: GUIDE_COLOR,
      size: 6,
      sizeAttenuation: false,
      depthTest: false,
    });
    this.marks = new THREE.Points(new THREE.BufferGeometry(), marksMaterial);
    this.marks.renderOrder = 6;
    this.marks.visible = false;
    scene.add(this.marks);
  }

  update(snapshot: DocumentSnapshot) {
    if (snapshot.guides.length === 0) {
      this.lines.visible = false;
      this.marks.visible = false;
      return;
    }

    const linePositions = new Float32Array(snapshot.guides.length * 6);
    // Three marks per guide: both endpoints plus the midpoint - drawing the
    // midpoint is what makes that snap target discoverable at all.
    const markPositions = new Float32Array(snapshot.guides.length * 9);
    snapshot.guides.forEach((guide, i) => {
      linePositions.set(guide.a, i * 6);
      linePositions.set(guide.b, i * 6 + 3);

      const mid: [number, number, number] = [
        (guide.a[0] + guide.b[0]) / 2,
        (guide.a[1] + guide.b[1]) / 2,
        (guide.a[2] + guide.b[2]) / 2,
      ];
      markPositions.set(guide.a, i * 9);
      markPositions.set(guide.b, i * 9 + 3);
      markPositions.set(mid, i * 9 + 6);
    });

    this.lines.geometry.setAttribute("position", new THREE.BufferAttribute(linePositions, 3));
    // Must run after every position update, not just once - a dashed
    // material with stale line-distance data renders solid, i.e.
    // indistinguishable from a model edge. (A method on Line/LineSegments
    // itself, not on BufferGeometry.)
    this.lines.computeLineDistances();
    this.lines.geometry.computeBoundingSphere();
    this.lines.visible = true;

    this.marks.geometry.setAttribute("position", new THREE.BufferAttribute(markPositions, 3));
    this.marks.geometry.computeBoundingSphere();
    this.marks.visible = true;
  }
}
