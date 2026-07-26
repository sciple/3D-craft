import { save, open, ask, message } from "@tauri-apps/plugin-dialog";
import { documentStore, reportHasProblems } from "../state/document-store";
import type { ModelReport } from "../state/document-store";
import { icons } from "./icons";

/// Save/Open/Export STL - each backed by the OS's native file picker (the
/// dialog plugin) so the user gets a normal Windows save/open dialog rather
/// than anything custom-built. The chosen path is handed straight to the
/// matching Rust command, which does the actual file I/O.
export function createFileMenu(container: HTMLElement) {
  const bar = document.createElement("div");
  bar.className = "file-menu";

  const saveButton = document.createElement("button");
  saveButton.title = "Save Project";
  saveButton.innerHTML = icons.save;
  saveButton.addEventListener("click", () => void promptSaveProject());

  const openButton = document.createElement("button");
  openButton.title = "Open Project";
  openButton.innerHTML = icons.open;
  openButton.addEventListener("click", () => void handleOpen());

  const exportButton = document.createElement("button");
  exportButton.title = "Export STL";
  exportButton.innerHTML = icons.exportStl;
  exportButton.addEventListener("click", () => void handleExportStl());

  const arrangeButton = document.createElement("button");
  arrangeButton.title = "Arrange for Print";
  arrangeButton.innerHTML = icons.arrangeForPrint;
  arrangeButton.addEventListener("click", () => void handleArrangeForPrint());

  const checkButton = document.createElement("button");
  checkButton.title = "Check Model";
  checkButton.innerHTML = icons.checkModel;
  checkButton.addEventListener("click", () => void handleCheckModel());

  bar.append(saveButton, openButton, exportButton, arrangeButton, checkButton);
  container.appendChild(bar);
}

/// Exported so `ui/close-guard.ts` can reuse the exact same save flow (and
/// know whether it actually succeeded) when the user picks "Save" from the
/// unsaved-changes prompt on window close.
export async function promptSaveProject(): Promise<boolean> {
  const path = await save({
    title: "Save Project",
    defaultPath: "project.json",
    filters: [{ name: "3D Craft Project", extensions: ["json"] }],
  });
  if (!path) return false;
  try {
    await documentStore.saveProject(path);
    return true;
  } catch (err) {
    alert(`Couldn't save the project: ${err}`);
    return false;
  }
}

async function handleOpen() {
  const path = await open({
    title: "Open Project",
    multiple: false,
    filters: [{ name: "3D Craft Project", extensions: ["json"] }],
  });
  if (!path || Array.isArray(path)) return;
  try {
    await documentStore.loadProject(path);
  } catch (err) {
    alert(`Couldn't open that project: ${err}`);
  }
}

async function handleExportStl() {
  const path = await save({
    title: "Export STL",
    defaultPath: "model.stl",
    filters: [{ name: "STL", extensions: ["stl"] }],
  });
  if (!path) return;
  try {
    await documentStore.exportStl(path);
  } catch (err) {
    // A refused export used to be a dead end: the user was told the model
    // isn't watertight but not *where*. Re-run the same check in its
    // detail-returning form and offer to draw the offending edges in the
    // viewport. Errors with nothing to point at (empty document, sketches
    // only) still just report themselves.
    const report = await documentStore.checkModel();
    if (!reportHasProblems(report)) {
      alert(`Couldn't export STL: ${err}`);
      return;
    }
    const show = await ask(`${err}\n\n${problemSummary(report)}`, {
      title: "Export STL",
      kind: "warning",
      okLabel: "Show Me",
      cancelLabel: "Close",
    });
    if (show) documentStore.showModelProblems(report);
  }
}

/// The Check Model button: the same diagnostic as above, available at any
/// time instead of only when an export attempt has already been refused.
/// Highlights straight away when there's something to show - the user asked
/// for the check, so a second "show me?" prompt would just be in the way.
async function handleCheckModel() {
  const report = await documentStore.checkModel();
  if (!reportHasProblems(report)) {
    documentStore.showModelProblems(null);
    await message(
      report.part_count === 0
        ? "Nothing printable yet - use Push/Pull to turn a sketch into a solid."
        : `Model is watertight: ${report.part_count} part(s) ready to export.`,
      { title: "Check Model", kind: "info" },
    );
    return;
  }
  documentStore.showModelProblems(report);
  await message(`${problemSummary(report)}\n\nThe problem edges are highlighted in red.`, {
    title: "Check Model",
    kind: "warning",
  });
}

function problemSummary(report: ModelReport): string {
  const counts: string[] = [];
  if (report.open_edges.length > 0) counts.push(`${report.open_edges.length} open edge(s)`);
  if (report.duplicate_edges.length > 0) {
    counts.push(`${report.duplicate_edges.length} duplicated edge(s)`);
  }
  return `${report.broken_part_count} of ${report.part_count} part(s) affected: ${counts.join(", ")}.`;
}

async function handleArrangeForPrint() {
  try {
    await documentStore.arrangeForPrint();
  } catch (err) {
    alert(`Couldn't arrange parts: ${err}`);
  }
}
