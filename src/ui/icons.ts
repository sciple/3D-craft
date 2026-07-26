/// Small inline-SVG line icons for the toolbar/file menu, all drawn on a
/// 24x24 grid with `stroke="currentColor"` so they inherit the button's
/// text color for free (active/hover states need no icon-specific CSS).
/// Kept as plain strings (not a bundled icon font/library) to avoid adding
/// a dependency for ~15 simple glyphs.
const svg = (body: string) =>
  `<svg viewBox="0 0 24 24" width="20" height="20" fill="none" stroke="currentColor" stroke-width="1.6" stroke-linecap="round" stroke-linejoin="round">${body}</svg>`;

export const icons = {
  select: svg(`<path d="M5 3l15 6.5-6.2 2.3-2.3 6.2z" fill="currentColor" stroke-width="1"/>`),
  rectangle: svg(`<rect x="4" y="6" width="16" height="12" rx="1"/>`),
  circle: svg(`<circle cx="12" cy="12" r="8"/>`),
  polygon: svg(`<path d="M12 3l8 6-3 10H7L4 9z"/>`),
  ngon: svg(`<path d="M12 3l7.8 4.5v9L12 21l-7.8-4.5v-9z"/>`),
  arc: svg(`<path d="M12 12V4a8 8 0 0 1 8 8z"/>`),
  pushPull: svg(`<path d="M12 2.5v9.5M12 2.5l-3 3M12 2.5l3 3"/><rect x="5" y="14" width="14" height="7.5"/>`),
  inset: svg(`<rect x="4" y="4" width="16" height="16" rx="1"/><rect x="9.5" y="9.5" width="5" height="5"/>`),
  scale: svg(
    `<rect x="6.5" y="6.5" width="11" height="11"/><path d="M2.5 2.5l5 5M2.5 2.5v5M2.5 2.5h5"/><path d="M21.5 21.5l-5-5M21.5 21.5v-5M21.5 21.5h-5"/>`,
  ),
  move: svg(
    `<path d="M12 2.5v19M2.5 12h19M12 2.5l-2.5 2.5M12 2.5l2.5 2.5M12 21.5l-2.5-2.5M12 21.5l2.5-2.5M2.5 12l2.5-2.5M2.5 12l2.5 2.5M21.5 12l-2.5-2.5M21.5 12l-2.5 2.5"/>`,
  ),
  rotate: svg(`<path d="M20 12a8 8 0 1 1-2.34-5.66"/><path d="M20 4v5h-5"/>`),
  measure: svg(`<rect x="2" y="8" width="20" height="8" rx="1"/><path d="M6 8v3M10 8v4M14 8v3M18 8v4"/>`),
  save: svg(`<path d="M5 4h11l3 3v13H5z"/><path d="M8 4v5h7V4"/><rect x="8" y="14" width="8" height="6"/>`),
  open: svg(`<path d="M3 7a2 2 0 0 1 2-2h4l2 2h8a2 2 0 0 1 2 2v8a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2z"/>`),
  exportStl: svg(`<path d="M12 3v12M12 15l-4-4M12 15l4-4"/><path d="M4 17v3a1 1 0 0 0 1 1h14a1 1 0 0 0 1-1v-3"/>`),
  arrangeForPrint: svg(
    `<rect x="3" y="3" width="7" height="7" rx="1"/><rect x="14" y="3" width="7" height="7" rx="1"/><rect x="3" y="14" width="7" height="7" rx="1"/><rect x="14" y="14" width="7" height="7" rx="1"/>`,
  ),
  // A solid with a tick - "is this printable?"
  checkModel: svg(
    `<path d="M12 2.5l8 4.5v9l-8 4.5-8-4.5v-9z"/><path d="M8 11.5l3 3 5.5-5.5"/>`,
  ),
};
