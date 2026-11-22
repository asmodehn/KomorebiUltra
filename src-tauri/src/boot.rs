//! Per-launch state of the work the page does once after a launch.
//!
//! A page reload cannot tell a reload from a launch by itself: its own storage
//! survives both, and anything it wrote before the reload is still there. So
//! the state lives in the process, not on disk: a new process starts with every
//! task pending.
//!
//! A task is marked done only once its effect is stored, so a page that dies in
//! between leaves it pending and the next page of the same launch retries it.

use std::sync::atomic::{AtomicBool, Ordering};

static AVATAR_CORNER_DONE: AtomicBool = AtomicBool::new(false);

/// True while this launch still has to place the character in the corner.
#[tauri::command]
pub fn avatar_corner_pending() -> bool {
    !AVATAR_CORNER_DONE.load(Ordering::Relaxed)
}

/// Records that the character was placed and the offset stored for this
/// launch, so later pages of the same process keep the position she has now.
#[tauri::command]
pub fn avatar_corner_done() {
    AVATAR_CORNER_DONE.store(true, Ordering::Relaxed);
}
