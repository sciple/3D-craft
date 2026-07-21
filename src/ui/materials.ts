/// Material density presets for the mass estimate, plus mass formatting.
/// Densities are kg/m³ (real-world engineering values); a part's mass is
/// `real_volume_m³ × density`. Purely a frontend table - nothing here touches
/// the document model or STL export.

/// `category` groups materials by what kind of part they belong on, so the
/// catalog can offer only sensible choices per part (e.g. fluids on a tank,
/// never on landing gear). See `CatalogPart.materialCategories`.
export type MaterialCategory = "metal" | "composite" | "plastic" | "fluid";

export interface Material {
  key: string;
  name: string;
  density: number; // kg/m³
  category: MaterialCategory;
}

export const MATERIALS: Material[] = [
  { key: "aluminium", name: "Aluminium", density: 2700, category: "metal" },
  { key: "titanium", name: "Titanium", density: 4500, category: "metal" },
  { key: "steel", name: "Steel", density: 7850, category: "metal" },
  { key: "inconel", name: "Inconel", density: 8440, category: "metal" },
  { key: "cfrp", name: "CFRP composite", density: 1600, category: "composite" },
  { key: "pla", name: "PLA (print plastic)", density: 1240, category: "plastic" },
  { key: "water", name: "Water / propellant", density: 1000, category: "fluid" },
  { key: "lh2", name: "Liquid hydrogen", density: 71, category: "fluid" },
  { key: "lox", name: "Liquid oxygen", density: 1141, category: "fluid" },
];

export const DEFAULT_MATERIAL_KEY = "aluminium";

/// Material categories offered on a part when it doesn't specify its own -
/// the structural set (metals, composites, print plastic), excluding fluids.
export const DEFAULT_MATERIAL_CATEGORIES: MaterialCategory[] = ["metal", "composite", "plastic"];

/// Materials whose category is in `categories`, in table order.
export function materialsInCategories(categories: readonly MaterialCategory[]): Material[] {
  return MATERIALS.filter((m) => categories.includes(m.category));
}

export function materialByKey(key: string): Material {
  return MATERIALS.find((m) => m.key === key) ?? MATERIALS[0];
}

/// Human-friendly mass string with an appropriate unit. Returns "—" for a
/// non-positive/non-finite mass (e.g. an unmodeled or non-solid part).
export function formatMass(kg: number): string {
  if (!Number.isFinite(kg) || kg <= 0) return "—";
  if (kg < 1e-3) return `${(kg * 1e6).toFixed(1)} mg`;
  if (kg < 1) return `${(kg * 1e3).toFixed(1)} g`;
  if (kg < 1e3) return `${kg.toFixed(kg < 10 ? 2 : 1)} kg`;
  if (kg < 1e6) return `${(kg / 1e3).toFixed(kg < 1e4 ? 2 : 1)} t`;
  return `${(kg / 1e3).toExponential(2)} t`;
}
