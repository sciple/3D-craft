import { save, open } from "@tauri-apps/plugin-dialog";
import { documentStore } from "../state/document-store";
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
  saveButton.addEventListener("click", () => void handleSave());

  const openButton = document.createElement("button");
  openButton.title = "Open Project";
  openButton.innerHTML = icons.open;
  openButton.addEventListener("click", () => void handleOpen());

  const exportButton = document.createElement("button");
  exportButton.title = "Export STL";
  exportButton.innerHTML = icons.exportStl;
  exportButton.addEventListener("click", () => void handleExportStl());

  bar.append(saveButton, openButton, exportButton);
  container.appendChild(bar);
}

async function handleSave() {
  const path = await save({
    title: "Save Project",
    defaultPath: "project.json",
    filters: [{ name: "3D Craft Project", extensions: ["json"] }],
  });
  if (!path) return;
  try {
    await documentStore.saveProject(path);
  } catch (err) {
    alert(`Couldn't save the project: ${err}`);
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
    alert(`Couldn't export STL: ${err}`);
  }
}
