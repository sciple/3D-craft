import * as THREE from "three";

/// Z-up throughout (matches both SketchUp's axis convention and the STL/3D
/// printing convention where Z is the vertical print axis), so no axis
/// remapping is ever needed on export.
export function initViewport(container: HTMLElement) {
  const scene = new THREE.Scene();
  scene.background = new THREE.Color(0x2f2f2f);

  const camera = new THREE.PerspectiveCamera(
    50,
    container.clientWidth / container.clientHeight,
    0.01,
    5000,
  );
  camera.up.set(0, 0, 1);
  camera.position.set(6, -6, 4.5);
  camera.lookAt(0, 0, 0);

  const renderer = new THREE.WebGLRenderer({ antialias: true });
  renderer.setPixelRatio(window.devicePixelRatio);
  renderer.setSize(container.clientWidth, container.clientHeight);
  container.appendChild(renderer.domElement);

  scene.add(new THREE.HemisphereLight(0xffffff, 0x444444, 3));
  const dirLight = new THREE.DirectionalLight(0xffffff, 2);
  dirLight.position.set(5, -8, 10);
  scene.add(dirLight);

  // 1 unit = 1 mm throughout this app (STL/slicers assume mm, and it keeps
  // export unit-conversion-free) - a 100x100mm grid in 10mm cells reads as a
  // sensible default print-bed-scale reference for small spacecraft parts.
  const grid = new THREE.GridHelper(100, 10, 0x556677, 0x3a4552);
  grid.rotation.x = Math.PI / 2; // GridHelper defaults to the XZ plane; rotate into XY (our ground plane)
  scene.add(grid);
  scene.add(new THREE.AxesHelper(10)); // red=X, green=Y, blue=Z, matching SketchUp's axis colors

  window.addEventListener("resize", () => {
    camera.aspect = container.clientWidth / container.clientHeight;
    camera.updateProjectionMatrix();
    renderer.setSize(container.clientWidth, container.clientHeight);
  });

  return { scene, camera, renderer };
}
