use serde::{Deserialize, Serialize};

/// The on-disk project format: a flat, serde-friendly mirror of a
/// `Document`'s geometry - deliberately not a direct serialization of
/// `Document` itself, since its vertex/face ids are slotmap keys that are
/// meaningless (and can even collide) across separate program runs. Faces
/// and groups here reference vertices/faces by plain array index instead,
/// and get re-interned into fresh ids on load (see
/// `Document::to_project_file` / `Document::from_project_file`).
#[derive(Debug, Serialize, Deserialize)]
pub struct ProjectFile {
    pub vertices: Vec<[f64; 3]>,
    pub faces: Vec<ProjectFace>,
    pub groups: Vec<ProjectGroup>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct ProjectFace {
    pub outer: Vec<u32>,
    pub holes: Vec<Vec<u32>>,
    /// Was this face created by push/pull (a cap or side wall of an
    /// already-built solid)? Preserved so `resplit_plane`'s solid-exclusion
    /// logic (see `Document::solid_face_ids`) still holds after a
    /// save/load round-trip.
    pub solid: bool,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct ProjectGroup {
    pub name: String,
    pub face_indices: Vec<u32>,
}
