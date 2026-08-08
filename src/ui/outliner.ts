import { message } from "@tauri-apps/plugin-dialog";
import { documentStore } from "../state/document-store";
import { expandToConnectedObjects } from "../tools/connectivity";

/// A minimal groups panel: name the current face selection and group it,
/// then click a group's name later to reselect all its faces, or ungroup
/// it. No nested groups or drag-and-drop reparenting - v1 groups are flat,
/// one-off collections (hull, wing, ...), matching the document model.
/// Also hosts one-shot whole-selection actions (Mirror, Drop to Plate, Array
/// Copy) that don't fit the modal tool toolbar.
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

  // Drops each disconnected selected object independently onto the build
  // plate (Z = 0) - each object's own lowest point rests on the plate,
  // relative offsets between separately-selected objects are not preserved.
  // See `Document::drop_to_plate`.
  const dropToPlateButton = document.createElement("button");
  dropToPlateButton.textContent = "Drop to Plate";
  dropToPlateButton.title = "Move each selected object down so it rests on the build plate (Z = 0)";
  dropToPlateButton.addEventListener("click", () => {
    const selected = documentStore.getSnapshot().selected_face_ids;
    if (selected.length === 0) return;
    void documentStore.dropToPlate(selected);
  });

  actionsRow.append(...mirrorButtons, dropToPlateButton);

  // Array Copy: lays the selection out as a columns x rows grid on the build
  // plate in one undoable step. See `Document::array_faces` for why the
  // counts include the original and why the pitch is center-to-center.
  const arrayRow = document.createElement("div");
  arrayRow.className = "outliner-array-row";

  /// A labelled number field. `isCount` fields must hold a whole number >= 1
  /// (a column/row count); the others are free-form distances, negative
  /// included - a negative pitch grows the grid backwards along that axis,
  /// which is occasionally what you want, so no `min` is set on them.
  const numberField = (label: string, value: number, isCount: boolean, title: string) => {
    const wrapper = document.createElement("label");
    wrapper.append(label);
    const input = document.createElement("input");
    input.type = "number";
    input.step = isCount ? "1" : "any";
    if (isCount) input.min = "1";
    input.value = String(value);
    input.title = title;
    wrapper.appendChild(input);
    return { wrapper, input, isCount, last: value };
  };

  const columns = numberField("Cols", 3, true, "Number of copies across X, counting the original");
  const rows = numberField("Rows", 2, true, "Number of copies along Y, counting the original");
  const pitchX = numberField("X", 30, false, "Center-to-center distance between columns (mm)");
  const pitchY = numberField("Y", 30, false, "Center-to-center distance between rows (mm)");

  /// Reads a field, reverting it to its last accepted value when it doesn't
  /// hold a usable number rather than sending garbage to the backend - the
  /// same approach as the parts catalog's scale field, and it keeps the last
  /// good value visible.
  const readField = (field: ReturnType<typeof numberField>): number | null => {
    const v = parseFloat(field.input.value);
    const ok = field.isCount ? Number.isInteger(v) && v >= 1 : Number.isFinite(v);
    if (!ok) {
      field.input.value = String(field.last);
      return null;
    }
    field.last = v;
    return v;
  };

  const arrayButton = document.createElement("button");
  arrayButton.textContent = "Array";
  arrayButton.title = "Copy the selection into a grid of columns x rows, spaced center-to-center";
  arrayButton.addEventListener("click", () => {
    const cols = readField(columns);
    const rowCount = readField(rows);
    const dx = readField(pitchX);
    const dy = readField(pitchY);
    if (cols === null || rowCount === null || dx === null || dy === null) return;

    const snapshot = documentStore.getSnapshot();
    if (snapshot.selected_face_ids.length === 0) return;
    // Expand to whole connected objects first: clicking one face of a solid
    // and hitting Array should tile the whole solid, not leave a grid of
    // floating quads. (Mirror deliberately doesn't do this - it's usually
    // aimed at a half-model that's already fully selected.)
    const selected = expandToConnectedObjects(snapshot, snapshot.selected_face_ids);
    void documentStore.arrayFaces(selected, cols, rowCount, dx, dy).catch((err) => {
      void message(`Couldn't build the array: ${err}`, { title: "Array", kind: "warning" });
    });
  });

  const countsLine = document.createElement("div");
  countsLine.className = "outliner-array-line";
  countsLine.append(columns.wrapper, rows.wrapper);
  const pitchLine = document.createElement("div");
  pitchLine.className = "outliner-array-line";
  pitchLine.append("Pitch", pitchX.wrapper, pitchY.wrapper, arrayButton);
  arrayRow.append(countsLine, pitchLine);

  panel.append(createRow, actionsRow, arrayRow, list);
  container.appendChild(panel);

  documentStore.subscribe((snapshot) => {
    const hasSelection = snapshot.selected_face_ids.length > 0;
    for (const button of mirrorButtons) button.disabled = !hasSelection;
    dropToPlateButton.disabled = !hasSelection;
    arrayButton.disabled = !hasSelection;

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
