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
  // export unit-conversion-free), so the grid can double as a true-scale
  // stand-in for the printer's build plate: 180x180 mm in 10 mm cells,
  // centered on the origin (-90..+90 in X and Y).
  const BED_SIZE = 180;
  const BED_CELLS = 18; // 10 mm per cell

  const grid = new THREE.GridHelper(BED_SIZE, BED_CELLS, 0x556677, 0x3a4552);
  grid.rotation.x = Math.PI / 2; // GridHelper defaults to the XZ plane; rotate into XY (our ground plane)
  scene.add(grid);

  // The plate perimeter, drawn brighter than the grid lines so the printable
  // boundary is unmistakable. WebGL ignores LineBasicMaterial.linewidth, so the
  // contrast has to come from colour, not thickness. Nudged a hair above z=0 to
  // avoid z-fighting with the grid's own outermost lines, which are coincident.
  const half = BED_SIZE / 2;
  const bedOutline = new THREE.LineLoop(
    new THREE.BufferGeometry().setFromPoints([
      new THREE.Vector3(-half, -half, 0.01),
      new THREE.Vector3(half, -half, 0.01),
      new THREE.Vector3(half, half, 0.01),
      new THREE.Vector3(-half, half, 0.01),
    ]),
    new THREE.LineBasicMaterial({ color: 0x9db8d8 }),
  );
  scene.add(bedOutline);
  scene.add(new THREE.AxesHelper(10)); // red=X, green=Y, blue=Z, matching SketchUp's axis colors

  window.addEventListener("resize", () => {
    camera.aspect = container.clientWidth / container.clientHeight;
    camera.updateProjectionMatrix();
    renderer.setSize(container.clientWidth, container.clientHeight);
  });

  return { scene, camera, renderer };
}
