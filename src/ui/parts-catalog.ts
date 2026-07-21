import { documentStore, type DocumentSnapshot, type FaceId, type GroupId } from "../state/document-store";
import { PARTS_CATALOG, TOTAL_CATALOG_PARTS, type CatalogPart } from "./parts-catalog-data";

/// A guided "build manifest" panel: a curated checklist of spacecraft
/// subsystems (see `parts-catalog-data.ts`) that lights up as you model them.
///
/// A part is counted as *modeled* when a document group of the exact same name
/// exists - so the link between "Reactor" in this list and the actual geometry
/// is just a group named "Reactor". This reuses the existing group machinery
/// wholesale (no backend changes): clicking a modeled part reselects its group
/// (like the outliner), and clicking an unmodeled part with a live selection
/// tags that selection under the part's name (`documentStore.groupFaces`).
///
/// Like the outliner, this re-renders its whole list from each snapshot; the
/// only state it keeps between renders is which categories/parts are expanded.
/// Toggling that expand state re-renders locally off the last snapshot (no
/// backend round-trip) - only real group edits flow through the store.
export function createPartsCatalog(container: HTMLElement) {
  const panel = document.createElement("div");
  panel.className = "parts-catalog";

  // The whole catalog is a dropdown: collapsed to just this header bar by
  // default so it never covers the 3D scene, expanded on click. Starts closed.
  let isOpen = false;
  const header = document.createElement("button");
  header.className = "parts-catalog-header";
  const caret = document.createElement("span");
  caret.className = "parts-catalog-caret";
  const title = document.createElement("span");
  title.className = "parts-catalog-title";
  title.textContent = "Parts Catalog";
  const progress = document.createElement("span");
  progress.className = "parts-catalog-progress";
  header.append(caret, title, progress);

  const list = document.createElement("div");
  list.className = "parts-catalog-list";

  const applyOpenState = () => {
    caret.textContent = isOpen ? "▾" : "▸";
    list.style.display = isOpen ? "flex" : "none";
    panel.classList.toggle("is-open", isOpen);
  };
  header.addEventListener("click", () => {
    isOpen = !isOpen;
    applyOpenState();
  });
  applyOpenState();

  panel.append(header, list);
  container.appendChild(panel);

  // Persisted purely on the client across re-renders. Categories start
  // expanded (the manifest is meant to be seen at a glance); part detail
  // (description + print tip) starts collapsed to keep the list compact.
  const collapsedCategories = new Set<string>();
  const expandedParts = new Set<string>();

  function render(snapshot: DocumentSnapshot) {
    // name -> group id, for reselecting a modeled part's geometry. If two
    // groups somehow share a name, last one wins - fine for this checklist.
    const groupIdByName = new Map<string, GroupId>();
    for (const group of snapshot.groups) groupIdByName.set(group.name, group.id);
    const selectedFaceIds = snapshot.selected_face_ids;
    const hasSelection = selectedFaceIds.length > 0;

    const modeledCount = PARTS_CATALOG.reduce(
      (sum, cat) => sum + cat.parts.filter((p) => groupIdByName.has(p.name)).length,
      0,
    );
    progress.textContent = `${modeledCount} / ${TOTAL_CATALOG_PARTS} modeled`;

    list.replaceChildren();
    for (const cat of PARTS_CATALOG) {
      const collapsed = collapsedCategories.has(cat.category);
      const modeledInCat = cat.parts.filter((p) => groupIdByName.has(p.name)).length;

      const catHeader = document.createElement("button");
      catHeader.className = "parts-catalog-cat";
      catHeader.textContent = `${collapsed ? "▸" : "▾"} ${cat.category}  (${modeledInCat}/${cat.parts.length})`;
      catHeader.addEventListener("click", () => {
        if (collapsed) collapsedCategories.delete(cat.category);
        else collapsedCategories.add(cat.category);
        render(documentStore.getSnapshot());
      });
      list.appendChild(catHeader);

      if (collapsed) continue;

      for (const part of cat.parts) {
        const modeledGroupId = groupIdByName.get(part.name) ?? null;
        list.appendChild(renderPartRow(part, modeledGroupId, hasSelection, selectedFaceIds, expandedParts, render));
      }
    }
  }

  documentStore.subscribe(render);
}

function renderPartRow(
  part: CatalogPart,
  modeledGroupId: GroupId | null,
  hasSelection: boolean,
  selectedFaceIds: FaceId[],
  expandedParts: Set<string>,
  rerender: (snapshot: DocumentSnapshot) => void,
): HTMLElement {
  const wrapper = document.createElement("div");
  wrapper.className = "parts-catalog-part";

  const row = document.createElement("div");
  row.className = "parts-catalog-row";

  const dot = document.createElement("span");
  dot.className = `parts-catalog-dot${modeledGroupId ? " is-modeled" : ""}`;

  // The name toggles the detail (description + print tip) open/closed.
  const nameButton = document.createElement("button");
  nameButton.className = "parts-catalog-name";
  nameButton.textContent = part.name;
  nameButton.title = part.description;
  const expanded = expandedParts.has(part.name);
  nameButton.addEventListener("click", () => {
    if (expanded) expandedParts.delete(part.name);
    else expandedParts.add(part.name);
    rerender(documentStore.getSnapshot());
  });

  row.append(dot, nameButton);

  // Trailing action: reselect a modeled part's group, or tag the current
  // selection as this part. Absent when there's nothing to do (unmodeled and
  // nothing selected) - the row is then just an informational checklist item.
  if (modeledGroupId) {
    const selectButton = document.createElement("button");
    selectButton.className = "parts-catalog-action";
    selectButton.textContent = "Select";
    selectButton.title = `Reselect the ${part.name} geometry`;
    selectButton.addEventListener("click", () => void documentStore.selectGroup(modeledGroupId));
    row.appendChild(selectButton);
  } else if (hasSelection) {
    const tagButton = document.createElement("button");
    tagButton.className = "parts-catalog-action";
    tagButton.textContent = "Tag";
    tagButton.title = `Tag the current selection as "${part.name}"`;
    tagButton.addEventListener("click", () => void documentStore.groupFaces(selectedFaceIds, part.name));
    row.appendChild(tagButton);
  }

  wrapper.appendChild(row);

  if (expanded) {
    const detail = document.createElement("div");
    detail.className = "parts-catalog-detail";
    const desc = document.createElement("div");
    desc.textContent = part.description;
    detail.appendChild(desc);
    if (part.printTip) {
      const tip = document.createElement("div");
      tip.className = "parts-catalog-tip";
      tip.textContent = `🖨 ${part.printTip}`;
      detail.appendChild(tip);
    }
    wrapper.appendChild(detail);
  }

  return wrapper;
}
