import { initViewport } from "./viewport/scene";
import { CadCameraControls } from "./viewport/controls";
import { MeshRenderer } from "./viewport/mesh-renderer";
import { documentStore } from "./state/document-store";
import { ToolManager } from "./tools/tool-manager";
import type { ToolContext } from "./tools/types";
import { SelectTool } from "./tools/select-tool";
import { DrawRectangleTool, DrawCircleTool, DrawPolygonTool } from "./tools/draw-tool";
import { PushPullTool } from "./tools/pushpull-tool";
import { InsetTool } from "./tools/inset-tool";
import { ScaleTool } from "./tools/scale-tool";
import { MoveTool } from "./tools/move-tool";
import { RotateTool } from "./tools/rotate-tool";
import { createToolbar } from "./ui/toolbar";
import { createOutliner } from "./ui/outliner";
import { createFileMenu } from "./ui/file-menu";

window.addEventListener("DOMContentLoaded", async () => {
  const viewportEl = document.querySelector<HTMLElement>("#viewport");
  const uiRootEl = document.querySelector<HTMLElement>("#ui-root");
  if (!viewportEl || !uiRootEl) return;

  const { scene, camera, renderer } = initViewport(viewportEl);
  new CadCameraControls(camera, renderer.domElement);
  const meshRenderer = new MeshRenderer(scene);

  const ctx: ToolContext = {
    scene,
    camera,
    domElement: renderer.domElement,
    meshRenderer,
  };
  const toolManager = new ToolManager(ctx);

  const selectTool = new SelectTool();
  const returnToSelect = () => toolManager.setTool(selectTool);
  const rectangleTool = new DrawRectangleTool(returnToSelect);
  const circleTool = new DrawCircleTool(returnToSelect);
  const polygonTool = new DrawPolygonTool(returnToSelect);
  const pushPullTool = new PushPullTool();
  const insetTool = new InsetTool();
  const scaleTool = new ScaleTool();
  const moveTool = new MoveTool();
  const rotateTool = new RotateTool();

  createToolbar(uiRootEl, toolManager, [
    { tool: selectTool, label: "Select", shortcut: "s" },
    { tool: rectangleTool, label: "Rectangle", shortcut: "r" },
    { tool: circleTool, label: "Circle", shortcut: "c" },
    { tool: polygonTool, label: "Polygon", shortcut: "l" },
    { tool: pushPullTool, label: "Push/Pull", shortcut: "p" },
    { tool: insetTool, label: "Inset", shortcut: "i" },
    { tool: scaleTool, label: "Scale", shortcut: "g" },
    { tool: moveTool, label: "Move", shortcut: "m" },
    { tool: rotateTool, label: "Rotate", shortcut: "t" },
  ]);
  toolManager.setTool(selectTool);
  createOutliner(uiRootEl);
  createFileMenu(uiRootEl);

  documentStore.subscribe((snapshot) => meshRenderer.update(snapshot));
  await documentStore.refresh();

  // Ctrl+Z / Ctrl+Y (and the common Ctrl+Shift+Z alternative) drive
  // undo/redo globally regardless of which tool is active - matches every
  // other desktop modeling app's convention. Only committed modeling
  // commands are recorded (see history.record() in commands.rs); an
  // in-progress drag preview is purely client-side, so undoing mid-drag
  // reverts the last *committed* edit without touching that preview.
  window.addEventListener("keydown", (e) => {
    if (e.target instanceof HTMLInputElement || e.target instanceof HTMLTextAreaElement) return;
    if (!(e.ctrlKey || e.metaKey)) return;
    if (e.key.toLowerCase() === "z" && !e.shiftKey) {
      e.preventDefault();
      void documentStore.undo();
    } else if (e.key.toLowerCase() === "y" || (e.key.toLowerCase() === "z" && e.shiftKey)) {
      e.preventDefault();
      void documentStore.redo();
    }
  });

  function animate() {
    requestAnimationFrame(animate);
    renderer.render(scene, camera);
  }
  animate();
});
