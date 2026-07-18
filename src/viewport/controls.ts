import * as THREE from "three";

/// The six standard CAD/Blender-style axis-aligned views, bound to the
/// numeric keypad: Numpad7/Numpad1/Numpad3 for Top/Front/Right, Ctrl+ the
/// same key for the opposite view (Bottom/Back/Left).
export type StandardView = "top" | "bottom" | "front" | "back" | "left" | "right";

const STANDARD_VIEW_KEYS: Record<string, { primary: StandardView; opposite: StandardView }> = {
  Numpad7: { primary: "top", opposite: "bottom" },
  Numpad1: { primary: "front", opposite: "back" },
  Numpad3: { primary: "right", opposite: "left" },
};

/// SketchUp-style camera navigation: middle-drag orbits, shift+middle-drag
/// or plain right-drag pans, scroll zooms. Right-drag is a second binding
/// for pan (not just shift+middle) since middle-click-while-holding-shift
/// is awkward or unavailable on many trackpads/mice; left mouse is
/// deliberately left untouched so tools (draw/select/push-pull) can use it.
///
/// Uses physics-convention spherical coordinates with Z as the polar axis
/// (rather than three.js's built-in Spherical/OrbitControls, which assume
/// Y-up) to match this app's Z-up world.
export class CadCameraControls {
  private target = new THREE.Vector3(0, 0, 0);
  private radius: number;
  private theta: number; // azimuth around +Z, radians
  private phi: number; // polar angle from +Z, radians

  private mode: "orbit" | "pan" | null = null;
  private lastX = 0;
  private lastY = 0;

  constructor(
    private camera: THREE.PerspectiveCamera,
    private domElement: HTMLElement,
  ) {
    const offset = camera.position.clone().sub(this.target);
    this.radius = offset.length();
    this.theta = Math.atan2(offset.y, offset.x);
    this.phi = Math.acos(THREE.MathUtils.clamp(offset.z / this.radius, -1, 1));

    domElement.addEventListener("contextmenu", (e) => e.preventDefault());
    domElement.addEventListener("pointerdown", this.onPointerDown);
    domElement.addEventListener("wheel", this.onWheel, { passive: false });
    window.addEventListener("keydown", this.onKeyDown);
  }

  private onKeyDown = (e: KeyboardEvent) => {
    if (e.target instanceof HTMLInputElement || e.target instanceof HTMLTextAreaElement) return;
    const keys = STANDARD_VIEW_KEYS[e.code];
    if (!keys) return;
    e.preventDefault();
    this.setStandardView(e.ctrlKey ? keys.opposite : keys.primary);
  };

  /// Snaps the camera to one of the six standard axis-aligned views,
  /// keeping the current target and zoom (radius) - only the viewing angle
  /// changes, matching how SketchUp/Blender's standard views behave.
  setStandardView(view: StandardView) {
    switch (view) {
      case "top":
        this.phi = 0;
        break;
      case "bottom":
        this.phi = Math.PI;
        break;
      case "front":
        this.theta = -Math.PI / 2;
        this.phi = Math.PI / 2;
        break;
      case "back":
        this.theta = Math.PI / 2;
        this.phi = Math.PI / 2;
        break;
      case "right":
        this.theta = 0;
        this.phi = Math.PI / 2;
        break;
      case "left":
        this.theta = Math.PI;
        this.phi = Math.PI / 2;
        break;
    }
    this.updateCamera();
  }

  private onPointerDown = (e: PointerEvent) => {
    if (e.button === 1) {
      this.mode = e.shiftKey ? "pan" : "orbit";
    } else if (e.button === 2) {
      this.mode = "pan";
    } else {
      return;
    }
    e.preventDefault();
    this.domElement.setPointerCapture(e.pointerId);
    this.lastX = e.clientX;
    this.lastY = e.clientY;
    window.addEventListener("pointermove", this.onPointerMove);
    window.addEventListener("pointerup", this.onPointerUp);
  };

  private onPointerMove = (e: PointerEvent) => {
    const dx = e.clientX - this.lastX;
    const dy = e.clientY - this.lastY;
    this.lastX = e.clientX;
    this.lastY = e.clientY;

    if (this.mode === "orbit") {
      this.theta -= dx * 0.006;
      this.phi = THREE.MathUtils.clamp(this.phi - dy * 0.006, 0.02, Math.PI - 0.02);
      this.updateCamera();
    } else if (this.mode === "pan") {
      const panSpeed = this.radius * 0.0015;
      const right = new THREE.Vector3(1, 0, 0).applyQuaternion(this.camera.quaternion);
      const up = new THREE.Vector3(0, 1, 0).applyQuaternion(this.camera.quaternion);
      this.target.addScaledVector(right, -dx * panSpeed).addScaledVector(up, dy * panSpeed);
      this.updateCamera();
    }
  };

  private onPointerUp = () => {
    this.mode = null;
    window.removeEventListener("pointermove", this.onPointerMove);
    window.removeEventListener("pointerup", this.onPointerUp);
  };

  private onWheel = (e: WheelEvent) => {
    e.preventDefault();
    const scale = Math.exp(e.deltaY * 0.001);
    this.radius = THREE.MathUtils.clamp(this.radius * scale, 0.2, 2000);
    this.updateCamera();
  };

  private updateCamera() {
    const sinPhi = Math.sin(this.phi);
    const offset = new THREE.Vector3(
      this.radius * sinPhi * Math.cos(this.theta),
      this.radius * sinPhi * Math.sin(this.theta),
      this.radius * Math.cos(this.phi),
    );
    this.camera.position.copy(this.target).add(offset);
    // World +Z (this app's up axis) is parallel to the view direction right
    // at the poles (looking straight down for Top, straight up for Bottom),
    // which makes camera orientation undefined - use world +Y/-Y as "up on
    // screen" there instead, matching the Top/Bottom standard-view
    // convention. Free orbiting never reaches the poles (phi is clamped
    // away from them below), so this only ever applies right after a
    // Top/Bottom standard-view snap.
    if (this.phi < 0.05) {
      this.camera.up.set(0, 1, 0);
    } else if (this.phi > Math.PI - 0.05) {
      this.camera.up.set(0, -1, 0);
    } else {
      this.camera.up.set(0, 0, 1);
    }
    this.camera.lookAt(this.target);
  }
}
