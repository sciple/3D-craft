import { documentStore } from "../state/document-store";

/// A minimal groups panel: name the current face selection and group it,
/// then click a group's name later to reselect all its faces, or ungroup
/// it. No nested groups or drag-and-drop reparenting - v1 groups are flat,
/// one-off collections (hull, wing, ...), matching the document model.
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

  panel.append(createRow, list);
  container.appendChild(panel);

  documentStore.subscribe((snapshot) => {
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
