import { documentStore } from "../state/document-store";

/// A minimal groups panel: name the current face selection and group it,
/// then click a group's name later to reselect all its faces, or ungroup
/// it. No nested groups or drag-and-drop reparenting - v1 groups are flat,
/// one-off collections (hull, wing, ...), matching the document model.
/// Also hosts one-shot whole-selection actions (Mirror) that don't fit the
/// modal tool toolbar.
export function createOutliner(container: HTMLElement) {
  const panel = document.createElement("div");
  panel.className = "outliner";

  const createRow = document.createElement("div");
  createRow.className = "outliner-create-row";
  const nameInput = document.createElement("input");
  nameInput.type = "text";
  nameInput.placeholder = "part name";
  nameInput.value = "part";
  const groupButton = document.createElement("button");
  groupButton.textContent = "Group Selected";
  groupButton.addEventListener("click", () => {
    const selected = documentStore.getSnapshot().selected_face_ids;
    if (selected.length === 0) return;
    void documentStore.groupFaces(selected, nameInput.value.trim() || "part");
  });
  createRow.append(nameInput, groupButton);

  const list = document.createElement("div");
  list.className = "outliner-list";

  // Mirrors a *copy* of the current selection across the world origin
  // plane perpendicular to the given axis - the common case for a
  // symmetric hull/wing modeled as one half. See `Document::mirror_faces`.
  const actionsRow = document.createElement("div");
  actionsRow.className = "outliner-actions-row";
  const mirrorButtons = (["x", "y", "z"] as const).map((axis) => {
    const button = document.createElement("button");
    // Color just the axis letter to match the viewport axes (red=X, green=Y,
    // blue=Z, same as the scene's AxesHelper) for quick visual matching.
    button.append("Mirror ");
    const letter = document.createElement("span");
    letter.className = `axis-label axis-${axis}`;
    letter.textContent = axis.toUpperCase();
    button.appendChild(letter);
    button.title = `Mirror selection across ${axis.toUpperCase()} = 0`;
    button.addEventListener("click", () => {
      const selected = documentStore.getSnapshot().selected_face_ids;
      if (selected.length === 0) return;
      void documentStore.mirrorFaces(selected, axis, [0, 0, 0]);
    });
    return button;
  });
  actionsRow.append(...mirrorButtons);

  panel.append(createRow, actionsRow, list);
  container.appendChild(panel);

  documentStore.subscribe((snapshot) => {
    const hasSelection = snapshot.selected_face_ids.length > 0;
    for (const button of mirrorButtons) button.disabled = !hasSelection;

    list.replaceChildren();
    for (const group of snapshot.groups) {
      const row = document.createElement("div");
      row.className = "outliner-row";

      const nameButton = document.createElement("button");
      nameButton.className = "outliner-row-name";
      nameButton.textContent = group.name;
      nameButton.addEventListener("click", () => void documentStore.selectGroup(group.id));

      const ungroupButton = document.createElement("button");
      ungroupButton.className = "outliner-row-ungroup";
      ungroupButton.textContent = "Ungroup";
      ungroupButton.addEventListener("click", () => void documentStore.ungroup(group.id));

      row.append(nameButton, ungroupButton);
      list.appendChild(row);
    }
  });
}
