use std::collections::HashSet;

use crate::geometry::mesh::FaceId;

/// The document's current selection. v1 selects at face granularity;
/// selecting a group (see `Document::select_group`) expands to all of that
/// group's faces here.
#[derive(Debug, Clone, Default)]
pub struct Selection {
    pub faces: HashSet<FaceId>,
}

impl Selection {
    pub fn clear(&mut self) {
        self.faces.clear();
    }
}
