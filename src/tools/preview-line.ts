import * as THREE from "three";

/// A single reusable line object for draw-tool previews (rectangle/circle
/// outline while dragging), so each tool doesn't hand-roll its own
/// create/update/dispose bookkeeping.
export class PreviewLine {
  private line: THREE.Line | null = null;

  update(scene: THREE.Scene, points: THREE.Vector3[]) {
    const geometry = new THREE.BufferGeometry().setFromPoints(points);
    if (!this.line) {
      this.line = new THREE.Line(geometry, new THREE.LineBasicMaterial({ color: 0xffcc55 }));
      scene.add(this.line);
    } else {
      this.line.geometry.dispose();
      this.line.geometry = geometry;
    }
  }

  clear(scene: THREE.Scene) {
    if (!this.line) return;
    scene.remove(this.line);
    this.line.geometry.dispose();
    (this.line.material as THREE.Material).dispose();
    this.line = null;
  }
}
