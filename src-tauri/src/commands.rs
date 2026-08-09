use std::sync::Mutex;

use glam::{DVec2, DVec3};
use tauri::State;

use crate::geometry::mesh::FaceId;
use crate::geometry::plane::Plane;
use crate::geometry::pushpull;
use crate::io::project_file::ProjectFile;
use crate::io::stl_export;
use crate::scene::document::{Document, DocumentSnapshot, GroupId, MirrorAxis, ModelReport};

/// How many undo steps to retain. Documents at this app's scale (a
/// spaceship part, not a large assembly) are small enough that cloning the
/// whole `Document` per step is cheap, so a generous cap costs little.
const MAX_UNDO_STEPS: usize = 100;

/// Upper bound on the cells an Array Copy may produce. A mistyped count
/// (300 x 200 instead of 3 x 2) would otherwise build tens of thousands of
/// solids and hang the app with no way back; this turns that into an error
/// message. Well above any plausible print-plate layout.
const MAX_ARRAY_CELLS: usize = 400;

/// Owns the live document plus linear undo/redo history. Every *modeling*
/// command snapshots the document before mutating it; pure selection
/// commands intentionally don't, so Ctrl+Z undoes geometry edits rather than
/// just reverting the selection.
#[derive(Default)]
pub struct History {
    document: Document,
    undo_stack: Vec<Document>,
    redo_stack: Vec<Document>,
}

impl History {
    /// Call before applying a modeling mutation: snapshots the current
    /// document onto the undo stack and clears redo (a fresh edit
    /// invalidates whatever was undone before it).
    fn record(&mut self) {
        self.undo_stack.push(self.document.clone());
        if self.undo_stack.len() > MAX_UNDO_STEPS {
            self.undo_stack.remove(0);
        }
        self.redo_stack.clear();
    }

    fn undo(&mut self) {
        if let Some(previous) = self.undo_stack.pop() {
            let current = std::mem::replace(&mut self.document, previous);
            self.redo_stack.push(current);
        }
    }

    fn redo(&mut self) {
        if let Some(next) = self.redo_stack.pop() {
            let current = std::mem::replace(&mut self.document, next);
            self.undo_stack.push(current);
        }
    }
}

pub struct AppState(pub Mutex<History>);

impl Default for AppState {
    fn default() -> Self {
        AppState(Mutex::new(History::default()))
    }
}

#[tauri::command]
pub fn get_document(state: State<AppState>) -> DocumentSnapshot {
    state.0.lock().unwrap().document.snapshot()
}

#[tauri::command]
pub fn undo(state: State<AppState>) -> DocumentSnapshot {
    let mut history = state.0.lock().unwrap();
    history.undo();
    history.document.snapshot()
}

#[tauri::command]
pub fn redo(state: State<AppState>) -> DocumentSnapshot {
    let mut history = state.0.lock().unwrap();
    history.redo();
    history.document.snapshot()
}

#[tauri::command]
pub fn draw_rectangle(
    state: State<AppState>,
    plane_origin: DVec3,
    plane_normal: DVec3,
    corner_a: DVec2,
    corner_b: DVec2,
    target_face_id: Option<FaceId>,
) -> DocumentSnapshot {
    let mut history = state.0.lock().unwrap();
    history.record();
    let plane = Plane::from_normal(plane_origin, plane_normal);
    history.document.draw_rectangle(&plane, corner_a, corner_b, target_face_id);
    history.document.snapshot()
}

#[tauri::command]
pub fn draw_circle(
    state: State<AppState>,
    plane_origin: DVec3,
    plane_normal: DVec3,
    center: DVec2,
    radius: f64,
    segments: u32,
    target_face_id: Option<FaceId>,
) -> DocumentSnapshot {
    let mut history = state.0.lock().unwrap();
    history.record();
    let plane = Plane::from_normal(plane_origin, plane_normal);
    history.document.draw_circle(&plane, center, radius, segments as usize, target_face_id);
    history.document.snapshot()
}

#[tauri::command]
pub fn draw_arc(
    state: State<AppState>,
    plane_origin: DVec3,
    plane_normal: DVec3,
    center: DVec2,
    radius: f64,
    start_angle_deg: f64,
    sweep_deg: f64,
    segments: u32,
    target_face_id: Option<FaceId>,
) -> DocumentSnapshot {
    let mut history = state.0.lock().unwrap();
    history.record();
    let plane = Plane::from_normal(plane_origin, plane_normal);
    history.document.draw_arc(&plane, center, radius, start_angle_deg, sweep_deg, segments as usize, target_face_id);
    history.document.snapshot()
}

#[tauri::command]
pub fn draw_ngon(
    state: State<AppState>,
    plane_origin: DVec3,
    plane_normal: DVec3,
    center: DVec2,
    radius: f64,
    sides: u32,
    start_angle_deg: f64,
    target_face_id: Option<FaceId>,
) -> DocumentSnapshot {
    let mut history = state.0.lock().unwrap();
    history.record();
    let plane = Plane::from_normal(plane_origin, plane_normal);
    history.document.draw_ngon(&plane, center, radius, sides as usize, start_angle_deg, target_face_id);
    history.document.snapshot()
}

#[tauri::command]
pub fn draw_polygon(
    state: State<AppState>,
    plane_origin: DVec3,
    plane_normal: DVec3,
    points: Vec<DVec2>,
    target_face_id: Option<FaceId>,
) -> DocumentSnapshot {
    let mut history = state.0.lock().unwrap();
    history.record();
    let plane = Plane::from_normal(plane_origin, plane_normal);
    history.document.draw_polygon(&plane, points, target_face_id);
    history.document.snapshot()
}

#[tauri::command]
pub fn push_pull_face(state: State<AppState>, face_id: FaceId, distance: f64) -> DocumentSnapshot {
    let mut history = state.0.lock().unwrap();
    history.record();
    history.document.push_pull(face_id, distance);
    history.document.snapshot()
}

#[tauri::command]
pub fn push_pull_faces(state: State<AppState>, face_ids: Vec<FaceId>, distance: f64) -> DocumentSnapshot {
    let mut history = state.0.lock().unwrap();
    history.record();
    history.document.push_pull_faces(&face_ids, distance);
    history.document.snapshot()
}

#[tauri::command]
pub fn inset_face(state: State<AppState>, face_id: FaceId, offset: f64) -> DocumentSnapshot {
    let mut history = state.0.lock().unwrap();
    history.record();
    history.document.inset_face(face_id, offset);
    history.document.snapshot()
}

#[tauri::command]
pub fn erase_face(state: State<AppState>, face_id: FaceId) -> DocumentSnapshot {
    let mut history = state.0.lock().unwrap();
    history.record();
    history.document.erase_face(face_id);
    history.document.snapshot()
}

#[tauri::command]
pub fn move_faces(state: State<AppState>, face_ids: Vec<FaceId>, delta: DVec3) -> DocumentSnapshot {
    let mut history = state.0.lock().unwrap();
    history.record();
    history.document.translate_faces(&face_ids, delta);
    history.document.snapshot()
}

#[tauri::command]
pub fn drop_to_plate(state: State<AppState>, face_ids: Vec<FaceId>) -> DocumentSnapshot {
    let mut history = state.0.lock().unwrap();
    history.record();
    history.document.drop_to_plate(&face_ids);
    history.document.snapshot()
}

#[tauri::command]
pub fn rotate_faces(
    state: State<AppState>,
    face_ids: Vec<FaceId>,
    pivot: DVec3,
    axis: DVec3,
    angle_radians: f64,
) -> DocumentSnapshot {
    let mut history = state.0.lock().unwrap();
    history.record();
    history.document.rotate_faces(&face_ids, pivot, axis, angle_radians);
    history.document.snapshot()
}

#[tauri::command]
pub fn scale_faces(state: State<AppState>, face_ids: Vec<FaceId>, pivot: DVec3, scale: DVec3) -> DocumentSnapshot {
    let mut history = state.0.lock().unwrap();
    history.record();
    history.document.scale_faces(&face_ids, pivot, scale);
    history.document.snapshot()
}

#[tauri::command]
pub fn duplicate_faces(state: State<AppState>, face_ids: Vec<FaceId>, delta: DVec3) -> DocumentSnapshot {
    let mut history = state.0.lock().unwrap();
    history.record();
    history.document.duplicate_faces(&face_ids, delta);
    history.document.snapshot()
}

/// Copies the selection into a `columns` x `rows` grid in one undo step -
/// the whole point of doing this in the backend rather than looping
/// `duplicate_faces` from the frontend, which would leave one undo entry per
/// copy. See `Document::array_faces` for the grid layout and why the counts
/// include the source. Guards run *before* `record()` so a rejected call
/// doesn't leave a no-op undo step behind.
#[tauri::command]
pub fn array_faces(
    state: State<AppState>,
    face_ids: Vec<FaceId>,
    columns: usize,
    rows: usize,
    pitch_x: f64,
    pitch_y: f64,
) -> Result<DocumentSnapshot, String> {
    if columns < 1 || rows < 1 {
        return Err("An array needs at least 1 column and 1 row.".to_string());
    }
    let cells = columns.checked_mul(rows).unwrap_or(usize::MAX);
    if cells > MAX_ARRAY_CELLS {
        return Err(format!(
            "{columns} x {rows} is {cells} copies - too many to build (limit {MAX_ARRAY_CELLS}). \
             Use smaller counts."
        ));
    }
    let mut history = state.0.lock().unwrap();
    history.record();
    history.document.array_faces(&face_ids, columns, rows, pitch_x, pitch_y);
    Ok(history.document.snapshot())
}

#[tauri::command]
pub fn mirror_faces(
    state: State<AppState>,
    face_ids: Vec<FaceId>,
    axis: MirrorAxis,
    pivot: DVec3,
) -> DocumentSnapshot {
    let mut history = state.0.lock().unwrap();
    history.record();
    history.document.mirror_faces(&face_ids, axis, pivot);
    history.document.snapshot()
}

#[tauri::command]
pub fn group_faces(state: State<AppState>, face_ids: Vec<FaceId>, name: String) -> DocumentSnapshot {
    let mut history = state.0.lock().unwrap();
    history.record();
    history.document.group_faces(&face_ids, name);
    history.document.snapshot()
}

#[tauri::command]
pub fn ungroup(state: State<AppState>, group_id: GroupId) -> DocumentSnapshot {
    let mut history = state.0.lock().unwrap();
    history.record();
    history.document.ungroup(group_id);
    history.document.snapshot()
}

#[tauri::command]
pub fn select_group(state: State<AppState>, group_id: GroupId) -> DocumentSnapshot {
    let mut history = state.0.lock().unwrap();
    history.document.select_group(group_id);
    history.document.snapshot()
}

#[tauri::command]
pub fn select_faces(state: State<AppState>, face_ids: Vec<FaceId>) -> DocumentSnapshot {
    let mut history = state.0.lock().unwrap();
    history.document.select(&face_ids);
    history.document.snapshot()
}

/// Records the segment the Measure tool just measured as a persistent guide.
/// One measurement = one guide = one undo step, so Ctrl+Z after a stray
/// measurement removes exactly that mark. Guards run *before* `record()`
/// (same reasoning as `array_faces`): a degenerate or non-finite segment
/// must not leave a no-op undo step behind, and a NaN would poison the
/// renderer's bounding sphere.
#[tauri::command]
pub fn add_guide(state: State<AppState>, a: DVec3, b: DVec3) -> DocumentSnapshot {
    let mut history = state.0.lock().unwrap();
    if !a.is_finite() || !b.is_finite() || a.distance_squared(b) < 1e-12 {
        return history.document.snapshot();
    }
    history.record();
    history.document.add_guide(a, b);
    history.document.snapshot()
}

/// Removes every guide in one undo step. No-op-safe: with no guides there is
/// nothing to record, and recording anyway would make Ctrl+Z silently step
/// over the user's *previous, real* edit - same reasoning as the selection
/// commands not recording.
#[tauri::command]
pub fn clear_guides(state: State<AppState>) -> DocumentSnapshot {
    let mut history = state.0.lock().unwrap();
    if history.document.guides.is_empty() {
        return history.document.snapshot();
    }
    history.record();
    history.document.clear_guides();
    history.document.snapshot()
}

/// Writes the current document to `path` as a JSON project file. The path
/// itself is chosen frontend-side via the dialog plugin's native save
/// picker - this command just needs a resolved path, no plugin required on
/// the Rust side for the actual file I/O.
#[tauri::command]
pub fn save_project(state: State<AppState>, path: String) -> Result<(), String> {
    let history = state.0.lock().unwrap();
    let project = history.document.to_project_file();
    let json = serde_json::to_string_pretty(&project).map_err(|e| e.to_string())?;
    std::fs::write(&path, json).map_err(|e| e.to_string())
}

/// Loads a JSON project file from `path`, replacing the current document.
/// Recorded like any other modeling command, so an accidental load doesn't
/// lose unsaved work beyond an undo.
#[tauri::command]
pub fn load_project(state: State<AppState>, path: String) -> Result<DocumentSnapshot, String> {
    let json = std::fs::read_to_string(&path).map_err(|e| e.to_string())?;
    let project: ProjectFile = serde_json::from_str(&json).map_err(|e| e.to_string())?;
    let mut history = state.0.lock().unwrap();
    history.record();
    history.document = Document::from_project_file(&project);
    Ok(history.document.snapshot())
}

/// Moves every disconnected printable solid onto a floor-aligned,
/// non-overlapping grid (see `Document::arrange_for_print`) - a one-click
/// "prepare for print" step. Errors the same way `export_stl` does when
/// there's nothing printable yet, rather than silently doing nothing.
#[tauri::command]
pub fn arrange_for_print(state: State<AppState>) -> Result<DocumentSnapshot, String> {
    let mut history = state.0.lock().unwrap();
    if history.document.solid_boundary_face_ids().is_empty() {
        return Err(
            "Nothing printable to arrange yet - flat sketches have no thickness. Use Push/Pull to \
             turn them into solids first."
                .to_string(),
        );
    }
    history.record();
    history.document.arrange_for_print();
    Ok(history.document.snapshot())
}

/// Exports the document's solids to a binary STL file at `path`. Flat
/// sketch faces are skipped (zero thickness - unprintable by definition),
/// so a leftover construction sketch never blocks exporting the finished
/// solids. Refuses to write a non-manifold model rather than silently
/// producing an STL a slicer will reject or (worse) silently misinterpret -
/// see `pushpull::is_manifold`'s doc comment for what that check covers.
#[tauri::command]
pub fn export_stl(state: State<AppState>, path: String) -> Result<(), String> {
    let history = state.0.lock().unwrap();
    let document = &history.document;
    let solid_ids = document.solid_boundary_face_ids();
    if solid_ids.is_empty() {
        return Err(if document.mesh.faces.is_empty() {
            "Nothing to export - the document is empty.".to_string()
        } else {
            "Nothing printable to export yet - flat sketches have no thickness. Use Push/Pull to \
             turn them into solids first."
                .to_string()
        });
    }
    if !pushpull::is_manifold(&document.mesh, &solid_ids) {
        return Err(
            "This model isn't watertight (a solid has an open or missing face, e.g. from erasing \
             part of it), so it won't slice correctly. Undo or rebuild the open solid before \
             exporting."
                .to_string(),
        );
    }
    let bytes = stl_export::write_binary_stl(&document.mesh, &solid_ids);
    std::fs::write(&path, bytes).map_err(|e| e.to_string())
}

/// Read-only watertightness diagnostic: the same check `export_stl` gates
/// on, but returning *where* the model is open so the frontend can highlight
/// it (see `Document::check_model`). Deliberately does not call
/// `history.record()` - same reasoning as the selection commands: it changes
/// nothing, so Ctrl+Z must not step over it.
#[tauri::command]
pub fn check_model(state: State<AppState>) -> ModelReport {
    state.0.lock().unwrap().document.check_model()
}

#[cfg(test)]
mod tests {
    use super::*;
    use glam::DVec2;

    #[test]
    fn undo_reverts_the_last_recorded_mutation() {
        let mut history = History::default();
        let plane = Plane::from_normal(DVec3::ZERO, DVec3::Z);

        history.record();
        history.document.draw_rectangle(&plane, DVec2::ZERO, DVec2::new(1.0, 1.0), None);
        assert_eq!(history.document.mesh.faces.len(), 1);

        history.undo();
        assert_eq!(history.document.mesh.faces.len(), 0);
    }

    #[test]
    fn redo_reapplies_an_undone_mutation() {
        let mut history = History::default();
        let plane = Plane::from_normal(DVec3::ZERO, DVec3::Z);

        history.record();
        history.document.draw_rectangle(&plane, DVec2::ZERO, DVec2::new(1.0, 1.0), None);
        history.undo();
        assert_eq!(history.document.mesh.faces.len(), 0);

        history.redo();
        assert_eq!(history.document.mesh.faces.len(), 1);
    }

    #[test]
    fn a_new_edit_after_undo_clears_the_redo_stack() {
        let mut history = History::default();
        let plane = Plane::from_normal(DVec3::ZERO, DVec3::Z);

        history.record();
        history.document.draw_rectangle(&plane, DVec2::ZERO, DVec2::new(1.0, 1.0), None);
        history.undo();

        history.record();
        history.document.draw_circle(&plane, DVec2::new(5.0, 5.0), 1.0, 8, None);
        assert!(history.redo_stack.is_empty());
        assert_eq!(history.document.mesh.faces.len(), 1);
    }

    /// The whole reason `array_faces` is a backend command instead of a
    /// frontend loop over `duplicate_faces`: the entire grid must collapse
    /// into a single undo step.
    #[test]
    fn an_array_of_copies_undoes_in_one_step() {
        let mut history = History::default();
        let plane = Plane::from_normal(DVec3::ZERO, DVec3::Z);
        history.record();
        let sketch_id = history.document.draw_rectangle(&plane, DVec2::ZERO, DVec2::new(1.0, 1.0), None)[0];
        history.record();
        let box_faces = history.document.push_pull(sketch_id, 1.0);
        let faces_before = history.document.mesh.faces.len();

        history.record();
        history.document.array_faces(&box_faces, 3, 2, 30.0, 30.0);
        assert_eq!(history.document.mesh.faces.len(), faces_before + 5 * 6);

        history.undo();
        assert_eq!(history.document.mesh.faces.len(), faces_before, "one undo must remove every copy in the grid, not just the last one");
    }

    #[test]
    fn undoing_a_corner_stud_sketch_restores_the_neighboring_walls_split_edge() {
        // `Document::propagate_boundary_split_to_solid_siblings` is the only
        // operation here that edits an *already existing, otherwise
        // untouched* face's loop in place, rather than creating new faces or
        // replacing a face's own loop wholesale. `record()` snapshots the
        // whole document, so undo covers it - but nothing else in the
        // codebase relies on that for a mutation of this shape, so pin it.
        let mut history = History::default();
        let plane = Plane::from_normal(DVec3::ZERO, DVec3::Z);
        history.record();
        let sketch_id = history.document.draw_rectangle(&plane, DVec2::ZERO, DVec2::new(10.0, 10.0), None)[0];
        history.record();
        let box_faces = history.document.push_pull(sketch_id, 4.0);

        let loop_sizes = |doc: &crate::scene::document::Document| -> Vec<usize> {
            let mut sizes: Vec<usize> = doc.mesh.faces.values().map(|f| f.outer.len()).collect();
            sizes.sort_unstable();
            sizes
        };
        let before = loop_sizes(&history.document);

        let top_id = box_faces.iter().copied().find(|&fid| history.document.mesh.faces[fid].normal.z > 0.9).unwrap();
        let top = history.document.mesh.faces[top_id].clone();
        let top_plane = Plane::from_normal(history.document.mesh.position(top.outer[0]), top.normal);
        let corner = top_plane.to_2d(history.document.mesh.position(top.outer[0]));

        history.record();
        history.document.draw_rectangle(&top_plane, corner, corner + DVec2::new(5.0, 5.0), Some(top_id));
        let after = loop_sizes(&history.document);
        assert_ne!(after, before, "the sketch must have split the cap and grown two neighboring walls");
        assert_eq!(
            after.iter().filter(|&&n| n == 5).count(),
            2,
            "both walls sharing the split edge gain exactly one T-junction vertex"
        );

        history.undo();
        assert_eq!(
            loop_sizes(&history.document),
            before,
            "undo must restore the in-place edits made to the neighboring walls, not just the new faces"
        );
    }

    #[test]
    fn clearing_guides_undoes_in_one_step() {
        let mut history = History::default();
        for i in 0..3 {
            history.record();
            history.document.add_guide(DVec3::new(i as f64, 0.0, 0.0), DVec3::new(i as f64, 1.0, 0.0));
        }
        assert_eq!(history.document.guides.len(), 3);

        history.record();
        history.document.clear_guides();
        assert!(history.document.guides.is_empty());

        history.undo();
        assert_eq!(history.document.guides.len(), 3, "one undo must restore every cleared guide, not just the last one");
    }

    #[test]
    fn undo_after_a_measurement_removes_just_that_guide() {
        let mut history = History::default();
        history.record();
        history.document.add_guide(DVec3::new(0.0, 0.0, 0.0), DVec3::new(1.0, 0.0, 0.0));
        history.record();
        history.document.add_guide(DVec3::new(0.0, 0.0, 0.0), DVec3::new(0.0, 1.0, 0.0));
        assert_eq!(history.document.guides.len(), 2);

        history.undo();
        assert_eq!(history.document.guides.len(), 1, "undo after a measurement should remove only that guide");
    }

    #[test]
    fn undo_and_redo_on_empty_stacks_are_no_ops() {
        let mut history = History::default();
        history.undo();
        history.redo();
        assert_eq!(history.document.mesh.faces.len(), 0);
    }

    #[test]
    fn undo_stack_is_capped_at_max_undo_steps() {
        let mut history = History::default();
        let plane = Plane::from_normal(DVec3::ZERO, DVec3::Z);
        for _ in 0..(MAX_UNDO_STEPS + 10) {
            history.record();
            history.document.draw_circle(&plane, DVec2::ZERO, 1.0, 8, None);
        }
        assert_eq!(history.undo_stack.len(), MAX_UNDO_STEPS);
    }
}
