import type { ToolManager } from "../tools/tool-manager";
import type { Tool } from "../tools/types";

export interface ToolbarEntry {
  tool: Tool;
  label: string;
  shortcut: string;
}

export function createToolbar(container: HTMLElement, toolManager: ToolManager, entries: ToolbarEntry[]) {
  const bar = document.createElement("div");
  bar.className = "toolbar";

  const buttons = new Map<Tool, HTMLButtonElement>();
  for (const entry of entries) {
    const button = document.createElement("button");
    button.textContent = `${entry.label} (${entry.shortcut.toUpperCase()})`;
    button.addEventListener("click", () => toolManager.setTool(entry.tool));
    bar.appendChild(button);
    buttons.set(entry.tool, button);
  }

  const setActive = (active: Tool) => {
    for (const [tool, button] of buttons) {
      button.classList.toggle("active", tool === active);
    }
  };
  toolManager.onToolChanged(setActive);
  container.appendChild(bar);

  window.addEventListener("keydown", (e) => {
    if (e.target instanceof HTMLInputElement || e.target instanceof HTMLTextAreaElement) return;
    const match = entries.find((entry) => entry.shortcut.toLowerCase() === e.key.toLowerCase());
    if (match) toolManager.setTool(match.tool);
  });

  return bar;
}
