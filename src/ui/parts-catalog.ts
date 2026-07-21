import { documentStore, type DocumentSnapshot, type FaceId, type GroupId, faceIdKey } from "../state/document-store";
import { PARTS_CATALOG, TOTAL_CATALOG_PARTS, type CatalogPart } from "./parts-catalog-data";
import { groupVolumes } from "../physics/volume";
import {
  DEFAULT_MATERIAL_KEY,
  DEFAULT_MATERIAL_CATEGORIES,
  materialByKey,
  materialsInCategories,
  formatMass,
  type Material,
} from "./materials";

/// Materials the mass estimate offers for a part - its declared categories, or
/// the structural default (no fluids) when it doesn't declare any.
function allowedMaterials(part: CatalogPart): Material[] {
  return materialsInCategories(part.materialCategories ?? DEFAULT_MATERIAL_CATEGORIES);
}

/// A guided "build manifest" panel: a curated checklist of spacecraft
/// subsystems (see `parts-catalog-data.ts`) that lights up as you model them,
/// and estimates each modeled part's mass.
///
/// A part is counted as *modeled* when a document group of the exact same name
/// exists - so the link between "Reactor" in this list and the actual geometry
/// is just a group named "Reactor". This reuses the existing group machinery
/// wholesale (no backend changes): clicking a modeled part reselects its group
/// (like the outliner), and clicking an unmodeled part with a live selection
/// tags that selection under the part's name (`documentStore.groupFaces`).
///
/// Mass estimate: `enclosed_solid_volume(units³) × metersPerUnit³ × density`.
/// Volume is computed on the frontend from the snapshot (`physics/volume.ts`);
/// the material (density) and the global scale (`metersPerUnit`) are chosen
/// here and persisted in localStorage. None of this touches the backend, the
/// document model, or STL export.
///
/// Like the outliner, this re-renders its whole list from each snapshot; the
/// only state it keeps between renders is which categories/parts are expanded,
/// plus the persisted material/scale choices.
export function createPartsCatalog(container: HTMLElement) {
  const materialOverrides = loadMaterialOverrides();
  let metersPerUnit = loadScale();

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
  // Total dry mass gets its own prominent element so it's easy to read at a
  // glance (it's the number the user actually acts on).
  const massTotal = document.createElement("span");
  massTotal.className = "parts-catalog-total";
  header.append(caret, title, progress, massTotal);

  // Scale control lives outside the rebuilt list so typing in it never loses
  // focus to a re-render. `change` (not `input`) commits on blur/Enter.
  const controls = document.createElement("div");
  controls.className = "parts-catalog-controls";
  const scaleLabel = document.createElement("label");
  scaleLabel.className = "parts-catalog-scale";
  scaleLabel.append("1 unit = ");
  const scaleInput = document.createElement("input");
  scaleInput.type = "number";
  scaleInput.min = "0";
  scaleInput.step = "any";
  scaleInput.value = String(metersPerUnit);
  scaleInput.title = "Real metres represented by one model unit (mm). Scales mass by this factor cubed.";
  scaleInput.addEventListener("change", () => {
    const v = parseFloat(scaleInput.value);
    if (Number.isFinite(v) && v > 0) {
      metersPerUnit = v;
      saveScale(metersPerUnit);
    } else {
      scaleInput.value = String(metersPerUnit);
    }
    render(documentStore.getSnapshot());
  });
  scaleLabel.append(scaleInput, " m");
  controls.append(scaleLabel);

  const list = document.createElement("div");
  list.className = "parts-catalog-list";

  const applyOpenState = () => {
    caret.textContent = isOpen ? "▾" : "▸";
    controls.style.display = isOpen ? "flex" : "none";
    list.style.display = isOpen ? "flex" : "none";
    panel.classList.toggle("is-open", isOpen);
  };
  header.addEventListener("click", () => {
    isOpen = !isOpen;
    applyOpenState();
  });
  applyOpenState();

  panel.append(header, controls, list);
  container.appendChild(panel);

  // Category expand state; part detail (material/description) starts collapsed.
  const collapsedCategories = new Set<string>();
  const expandedParts = new Set<string>();

  // The chosen material, clamped to the part's allowed set (so a saved override
  // that no longer applies - e.g. a fluid on a now-structural part - falls back
  // to the first sensible option instead of a wrong density).
  const materialKeyFor = (part: CatalogPart): string => {
    const chosen = materialOverrides[part.name] ?? part.defaultMaterial ?? DEFAULT_MATERIAL_KEY;
    const allowed = allowedMaterials(part);
    return allowed.some((m) => m.key === chosen) ? chosen : allowed[0]?.key ?? DEFAULT_MATERIAL_KEY;
  };

  const partMassKg = (volumeUnits3: number, part: CatalogPart): number =>
    volumeUnits3 * metersPerUnit ** 3 * materialByKey(materialKeyFor(part)).density;

  function render(snapshot: DocumentSnapshot) {
    // name -> group id, for reselecting a modeled part's geometry. If two
    // groups somehow share a name, last one wins - fine for this checklist.
    const groupIdByName = new Map<string, GroupId>();
    for (const group of snapshot.groups) groupIdByName.set(group.name, group.id);
    const selectedFaceIds = snapshot.selected_face_ids;
    const hasSelection = selectedFaceIds.length > 0;
    const volumes = groupVolumes(snapshot);

    const volumeForPart = (gid: GroupId | null): number =>
      gid ? (volumes.get(faceIdKey(gid)) ?? 0) : 0;

    let modeledCount = 0;
    let totalKg = 0;
    for (const cat of PARTS_CATALOG) {
      for (const part of cat.parts) {
        const gid = groupIdByName.get(part.name);
        if (!gid) continue;
        modeledCount++;
        totalKg += partMassKg(volumeForPart(gid), part);
      }
    }
    progress.textContent = `${modeledCount}/${TOTAL_CATALOG_PARTS}`;
    progress.title = "Modeled subsystems";
    massTotal.textContent = formatMass(totalKg);
    massTotal.title = "Estimated total dry mass";

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
        const massKg = modeledGroupId ? partMassKg(volumeForPart(modeledGroupId), part) : null;
        list.appendChild(
          renderPartRow(part, modeledGroupId, massKg, hasSelection, selectedFaceIds, {
            expandedParts,
            rerender: render,
            materialKeyFor,
            onMaterialChange: (name, key) => {
              materialOverrides[name] = key;
              saveMaterialOverrides(materialOverrides);
              render(documentStore.getSnapshot());
            },
          }),
        );
      }
    }
  }

  documentStore.subscribe(render);
}

interface RowContext {
  expandedParts: Set<string>;
  rerender: (snapshot: DocumentSnapshot) => void;
  materialKeyFor: (part: CatalogPart) => string;
  onMaterialChange: (partName: string, materialKey: string) => void;
}

function renderPartRow(
  part: CatalogPart,
  modeledGroupId: GroupId | null,
  massKg: number | null,
  hasSelection: boolean,
  selectedFaceIds: FaceId[],
  ctx: RowContext,
): HTMLElement {
  const wrapper = document.createElement("div");
  wrapper.className = "parts-catalog-part";

  const row = document.createElement("div");
  row.className = "parts-catalog-row";

  const dot = document.createElement("span");
  dot.className = `parts-catalog-dot${modeledGroupId ? " is-modeled" : ""}`;

  // The name toggles the detail (material + description + print tip) open/closed.
  const nameButton = document.createElement("button");
  nameButton.className = "parts-catalog-name";
  nameButton.textContent = part.name;
  nameButton.title = part.description;
  const expanded = ctx.expandedParts.has(part.name);
  nameButton.addEventListener("click", () => {
    if (expanded) ctx.expandedParts.delete(part.name);
    else ctx.expandedParts.add(part.name);
    ctx.rerender(documentStore.getSnapshot());
  });

  // Mass estimate for a modeled part; "—" placeholder otherwise.
  const mass = document.createElement("span");
  mass.className = "parts-catalog-mass";
  mass.textContent = massKg !== null ? formatMass(massKg) : "—";

  row.append(dot, nameButton, mass);

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

    // Material picker (drives the density used for the mass estimate).
    const matRow = document.createElement("label");
    matRow.className = "parts-catalog-material";
    matRow.append("Material ");
    const select = document.createElement("select");
    for (const m of allowedMaterials(part)) {
      const opt = document.createElement("option");
      opt.value = m.key;
      opt.textContent = `${m.name} — ${m.density} kg/m³`;
      select.appendChild(opt);
    }
    select.value = ctx.materialKeyFor(part);
    select.addEventListener("change", () => ctx.onMaterialChange(part.name, select.value));
    matRow.appendChild(select);
    detail.appendChild(matRow);

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

// --- localStorage persistence (app-global, not per-project; see file header) ---

const MATERIALS_KEY = "partsCatalog.materialOverrides";
const SCALE_KEY = "partsCatalog.metersPerUnit";
const DEFAULT_METERS_PER_UNIT = 0.001; // 1 unit = 1 mm modeled true-size

function loadMaterialOverrides(): Record<string, string> {
  try {
    const raw = localStorage.getItem(MATERIALS_KEY);
    if (!raw) return {};
    const parsed = JSON.parse(raw);
    return parsed && typeof parsed === "object" ? (parsed as Record<string, string>) : {};
  } catch {
    return {};
  }
}

function saveMaterialOverrides(overrides: Record<string, string>) {
  try {
    localStorage.setItem(MATERIALS_KEY, JSON.stringify(overrides));
  } catch {
    // Persistence is best-effort; a storage failure must not break the panel.
  }
}

function loadScale(): number {
  try {
    const raw = localStorage.getItem(SCALE_KEY);
    const v = raw ? parseFloat(raw) : NaN;
    return Number.isFinite(v) && v > 0 ? v : DEFAULT_METERS_PER_UNIT;
  } catch {
    return DEFAULT_METERS_PER_UNIT;
  }
}

function saveScale(metersPerUnit: number) {
  try {
    localStorage.setItem(SCALE_KEY, String(metersPerUnit));
  } catch {
    // Best-effort (see saveMaterialOverrides).
  }
}
