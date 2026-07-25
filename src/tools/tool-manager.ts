import type { Tool, ToolContext } from "./types";

export class ToolManager {
  private activeTool: Tool | null = null;
  private listeners = new Set<(tool: Tool) => void>();

  constructor(private ctx: ToolContext) {
    ctx.domElement.addEventListener("pointerdown", (e) => this.activeTool?.onPointerDown?.(e, this.ctx));
    ctx.domElement.addEventListener("pointermove", (e) => this.activeTool?.onPointerMove?.(e, this.ctx));
    ctx.domElement.addEventListener("pointerup", (e) => this.activeTool?.onPointerUp?.(e, this.ctx));
    window.addEventListener("keydown", (e) => {
      // Typing into a form field (e.g. the outliner's group-name input)
      // must never be read as a tool shortcut - Backspace/Delete in
      // particular would otherwise erase the current selection instead of
      // editing the text.
      if (e.target instanceof HTMLInputElement || e.target instanceof HTMLTextAreaElement) return;
      this.activeTool?.onKeyDown?.(e, this.ctx);
    });
    window.addEventListener("keyup", (e) => {
      if (e.target instanceof HTMLInputElement || e.target instanceof HTMLTextAreaElement) return;
      this.activeTool?.onKeyUp?.(e, this.ctx);
    });
  }

  setTool(tool: Tool) {
    if (this.activeTool === tool) return;
    this.activeTool?.deactivate?.(this.ctx);
    this.activeTool = tool;
    tool.activate?.(this.ctx);
    for (const listener of this.listeners) listener(tool);
  }

  getTool(): Tool | null {
    return this.activeTool;
  }

  onToolChanged(listener: (tool: Tool) => void): () => void {
    this.listeners.add(listener);
    return () => this.listeners.delete(listener);
  }
}
