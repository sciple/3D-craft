import { type DocumentSnapshot, faceIdKey } from "../state/document-store";

/// Enclosed volume (in model units³, i.e. mm³) of each group in the snapshot,
/// keyed by `faceIdKey(group_id)`. Computed entirely from data the snapshot
/// already carries - vertices plus each face's triangle index-triples and
/// group id - so this adds nothing to the backend and cannot affect modeling
/// or STL export.
///
/// A closed solid's volume is the signed-tetrahedron sum `Σ a·(b×c)/6` over
/// its outward-wound triangles (the triangles come from the backend's
/// `triangulate_face`, wound consistent with each face's outward normal, so a
/// watertight solid sums to a positive volume). We `abs()` the per-group total
/// defensively. A group that isn't a closed solid (e.g. a lone flat sketch)
/// yields a meaningless number - acceptable for a mass *estimate*, and it can
/// never corrupt geometry.
///
/// Deliberately total and non-throwing: any malformed triangle/index is
/// skipped so a mass readout can never break the panel that renders it.
export function groupVolumes(snapshot: DocumentSnapshot): Map<string, number> {
  const volumes = new Map<string, number>();
  const verts = snapshot.vertices;
  if (!verts) return volumes;

  for (const face of snapshot.faces) {
    if (!face.group_id) continue;
    const key = faceIdKey(face.group_id);
    let acc = volumes.get(key) ?? 0;
    for (const tri of face.triangles) {
      const a = verts[tri[0]];
      const b = verts[tri[1]];
      const c = verts[tri[2]];
      if (!a || !b || !c) continue;
      // a · (b × c)
      const cx = b[1] * c[2] - b[2] * c[1];
      const cy = b[2] * c[0] - b[0] * c[2];
      const cz = b[0] * c[1] - b[1] * c[0];
      acc += (a[0] * cx + a[1] * cy + a[2] * cz) / 6;
    }
    volumes.set(key, acc);
  }

  for (const [k, v] of volumes) volumes.set(k, Math.abs(v));
  return volumes;
}
