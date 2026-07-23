import { getCurrentWindow } from "@tauri-apps/api/window";
import { message } from "@tauri-apps/plugin-dialog";
import { documentStore } from "../state/document-store";
import { promptSaveProject } from "./file-menu";

/// Intercepts the native window-close request (titlebar X, Alt+F4, taskbar
/// close) so unsaved modeling work is never silently discarded - closing a
/// Tauri window otherwise just quits the whole app with no prompt at all.
/// Mirrors the Save/Discard/Cancel convention of every other document-based
/// desktop app.
export function installCloseGuard() {
  const appWindow = getCurrentWindow();

  void appWindow.onCloseRequested(async (event) => {
    if (!documentStore.isDirty()) return; // nothing unsaved - let the close proceed

    event.preventDefault();
    const choice = await message("You have unsaved changes. Save before closing?", {
      title: "Unsaved Changes",
      kind: "warning",
      buttons: { yes: "Save", no: "Discard", cancel: "Cancel" },
    });

    if (choice === "Cancel") return; // stay open
    if (choice === "Save") {
      const saved = await promptSaveProject();
      if (!saved) return; // save dialog cancelled, or the save failed - stay open either way
    }
    // .destroy() (not .close()) bypasses onCloseRequested, avoiding
    // re-triggering this same handler.
    await appWindow.destroy();
  });
}
