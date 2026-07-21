/// The curated, hand-editable catalog of spacecraft subsystems that drives
/// the Parts Catalog panel (`parts-catalog.ts`). This is deliberately just a
/// static data table, not an in-app editor: to add/rename/reorganize parts,
/// edit this file. Each part's `name` is also its *link key* - the catalog
/// counts a part as "modeled" when a document group of the exact same name
/// exists (see `parts-catalog.ts`), so keep names unique and stable (renaming
/// one here orphans any group already tagged under the old name).
///
/// Scope is one interstellar spacecraft's subsystems, matching this tool's
/// purpose (modeling small parts to 3D-print). `printTip` is an optional
/// print-orientation/geometry hint shown when a part row is expanded.

export interface CatalogPart {
  name: string;
  description: string;
  printTip?: string;
}

export interface CatalogCategory {
  category: string;
  parts: CatalogPart[];
}

export const PARTS_CATALOG: CatalogCategory[] = [
  {
    category: "Propulsion",
    parts: [
      {
        name: "Main drive",
        description: "Primary engine - fusion torch, antimatter, or ion grid.",
        printTip: "Print bell/nozzle-up so the flare needs no internal supports.",
      },
      {
        name: "RCS thrusters",
        description: "Reaction-control thruster quads for attitude/translation.",
        printTip: "Model one quad, then Ctrl+D duplicate around the hull.",
      },
      {
        name: "Propellant tank",
        description: "Fuel/reaction-mass tankage - usually the largest volume.",
        printTip: "A capsule/cylinder; print upright to keep the walls even.",
      },
      {
        name: "Nozzle / mag-nozzle",
        description: "Expansion bell or magnetic-nozzle skirt behind the drive.",
        printTip: "Print mouth-down as a hollow cone to avoid support scars inside.",
      },
      {
        name: "Fuel scoop / ramscoop",
        description: "Forward Bussard scoop gathering interstellar hydrogen as fuel.",
        printTip: "A wide flared funnel/ring; print mouth-down as a thin cone.",
      },
    ],
  },
  {
    category: "Power & thermal",
    parts: [
      {
        name: "Reactor",
        description: "Fission/fusion core supplying the ship's power.",
        printTip: "A short heavy cylinder; add end-cap detail with Inset + Push/Pull.",
      },
      {
        name: "RTG",
        description: "Radioisotope generator - the classic finned canister.",
        printTip: "Fins print cleanest standing vertical (along the print Z).",
      },
      {
        name: "Radiators",
        description: "Heat-rejection panels dumping waste heat to space.",
        printTip: "Print flat on the bed - thin large panels, minimal supports.",
      },
      {
        name: "Sunshade / heat shield",
        description: "Forward shade or ablative shield facing the heat source.",
        printTip: "Model as a thin dish; print concave-side up.",
      },
      {
        name: "Solar array",
        description: "Deployable photovoltaic panels for inner-system power.",
        printTip: "Print panels flat; model the hinge/boom as a separate part.",
      },
    ],
  },
  {
    category: "Propellant-less propulsion",
    parts: [
      {
        name: "Solar sail boom",
        description: "Deployable spar structure the sail membrane stretches over.",
        printTip: "Long thin trusses; print flat and orient the long axis on the bed.",
      },
      {
        name: "Magsail loop",
        description: "Superconducting magnetic-sail current loop / support ring.",
        printTip: "A large thin torus; print flat.",
      },
      {
        name: "Beamed-light collector",
        description: "Reflector/collector for beamed-power light-sail propulsion.",
        printTip: "Shallow dish; print concave-side up as a thin shell.",
      },
    ],
  },
  {
    category: "Structure & mechanisms",
    parts: [
      {
        name: "Primary truss / spine",
        description: "The structural backbone everything else mounts to.",
        printTip: "Print the long axis flat along the bed for strength and no supports.",
      },
      {
        name: "Hull plating / Whipple shield",
        description: "Outer plating plus a Whipple bumper for micrometeoroid and relativistic-dust hits (the forward face takes the worst of it near light speed).",
        printTip: "Flat/curved panels; keep wall thickness >= 2 nozzle widths.",
      },
      {
        name: "Docking port / adapter",
        description: "Docking collar or inter-module adapter ring.",
        printTip: "Print ring-flat; use Inset for the collar's recessed lip.",
      },
      {
        name: "Staging decoupler",
        description: "Separation ring / decoupler between jettisonable stages.",
        printTip: "A thin ring; print flat and add the clamp band as surface detail.",
      },
      {
        name: "Landing gear / legs",
        description: "Deployable legs and footpads for a lander element.",
        printTip: "Print legs flat along their length; assemble at the hinge.",
      },
      {
        name: "Greeble panels",
        description: "Surface detail - hatches, panel lines, raised/recessed greebles.",
        printTip: "Sketch on the hull face, then Push/Pull in or out a fraction of a mm.",
      },
    ],
  },
  {
    category: "Avionics & control",
    parts: [
      {
        name: "Reaction wheel / CMG",
        description: "Momentum wheels or control-moment gyros - attitude control without spending propellant.",
        printTip: "A short heavy cylinder; print upright and add a mounting bracket.",
      },
      {
        name: "Flight computer / avionics bay",
        description: "Command & data-handling - the ship's flight computers and wiring runs.",
        printTip: "A greebled housing; detail the connectors with shallow insets.",
      },
    ],
  },
  {
    category: "Crew & life support",
    parts: [
      {
        name: "Crew cabin / hab",
        description: "Pressurized habitat / crew module.",
        printTip: "Hollow with a thin wall; print upright to keep the dome clean.",
      },
      {
        name: "Cryo pods",
        description: "Hibernation / cold-sleep pods for the long transit.",
        printTip: "Model one pod, duplicate into a bank.",
      },
      {
        name: "Airlock",
        description: "Pressure-lock vestibule between hab and vacuum.",
        printTip: "A short cylinder with a recessed hatch (Inset + Push/Pull).",
      },
      {
        name: "ECLSS bay",
        description: "Air/water recycling and life-support machinery bay.",
        printTip: "A greebled box; detail with panel insets.",
      },
      {
        name: "Centrifuge ring",
        description: "Rotating ring providing spin (artificial) gravity.",
        printTip: "Print flat as a torus; the ring's the strongest that way.",
      },
    ],
  },
  {
    category: "Sensors, comms & payload",
    parts: [
      {
        name: "High-gain antenna",
        description: "Main communications dish back toward home.",
        printTip: "Print the dish concave-side up as a thin shell.",
      },
      {
        name: "Star tracker / sensor array",
        description: "Navigation star trackers and sensor cluster.",
        printTip: "Small box + lens barrels; print barrels standing up.",
      },
      {
        name: "Instrument bay",
        description: "Science instrument housing / payload bay.",
        printTip: "Hollow box; leave a wall for mounting detail.",
      },
      {
        name: "Cargo / lander bay",
        description: "Cargo hold or probe/lander stowage bay.",
        printTip: "Model the bay as a recess with Inset, then Push/Pull inward.",
      },
    ],
  },
];

/// Total number of catalog parts across all categories - the denominator for
/// the panel's "N / total subsystems modeled" progress counter.
export const TOTAL_CATALOG_PARTS = PARTS_CATALOG.reduce((sum, c) => sum + c.parts.length, 0);
