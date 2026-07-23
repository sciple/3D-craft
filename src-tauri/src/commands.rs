use std::sync::Mutex;

use glam::{DVec2, DVec3};
use tauri::State;

use crate::geometry::mesh::FaceId;
use crate::geometry::plane::Plane;
use crate::geometry::pushpull;
use crate::io::project_file::ProjectFile;
use crate::io::stl_export;
use crate::scene::document::{Document, DocumentSnapshot, GroupId, MirrorAxis};

/// How many undo steps to retain. Documents at this app's scale (a
/// spaceship part, not a large assembly) are small enough that cloning the
/// whole `Document` per step is cheap, so a generous cap costs little.
const MAX_UNDO_STEPS: usize = 100;

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
