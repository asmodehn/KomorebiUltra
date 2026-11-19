//! Avatar layout (Live2D model URL, zoom, on-screen offsets).
//!
//! `avatar_zoom` is a multiplier applied on top of the auto-fit scale
//! that makes the model fill its allotted box. 1.0 = current behaviour;
//! 0.5 = half size; 2.0 = double size (will overflow the canvas, which
//! is the whole point of letting the user keep the model big inside a
//! smaller window).
//!
//! `avatar_offset_x` / `avatar_offset_y` are *fractions* of half the avatar
//! box (-2.0..=2.0) rather than absolute pixels, so they survive
//! window resizes without re-anchoring weirdly. Positive Y nudges the
//! model down. ±1 puts the model's centre on the box edge, so half of it is
//! outside and the window clips it; ±2 puts the centre half a box past the
//! edge, far enough that the character is drawn clear off the window side at
//! ordinary sizes. Past ±2 there is nothing left to see, and the drag would
//! only accumulate distance to travel back.

use super::store::{get_f64, read_string, write_optional_f64, write_optional_string};
use super::Result;
use tauri::{AppHandle, Runtime, Wry};

const KEY_LIVE2D_MODEL_URL: &str = "live2d_model_url";
const KEY_AVATAR_ZOOM: &str = "avatar_zoom";
const KEY_AVATAR_OFFSET_X: &str = "avatar_offset_x";
const KEY_AVATAR_OFFSET_Y: &str = "avatar_offset_y";

pub fn read_live2d_model_url(app: &AppHandle<Wry>) -> Option<String> {
    read_string(app, KEY_LIVE2D_MODEL_URL)
}

pub fn set_live2d_model_url<R: Runtime>(app: &AppHandle<R>, url: &str) -> Result<()> {
    write_optional_string(app, KEY_LIVE2D_MODEL_URL, url)
}

pub fn read_avatar_zoom(app: &AppHandle<Wry>) -> f64 {
    get_f64(app, KEY_AVATAR_ZOOM).unwrap_or(1.0)
}

pub fn read_avatar_offset_x(app: &AppHandle<Wry>) -> f64 {
    get_f64(app, KEY_AVATAR_OFFSET_X).unwrap_or(0.0)
}

pub fn read_avatar_offset_y(app: &AppHandle<Wry>) -> f64 {
    get_f64(app, KEY_AVATAR_OFFSET_Y).unwrap_or(0.0)
}

pub fn set_avatar_zoom<R: Runtime>(app: &AppHandle<R>, value: f64) -> Result<()> {
    let clamped = value.clamp(0.2, 3.0);
    write_optional_f64(app, KEY_AVATAR_ZOOM, Some(clamped))
}

pub fn set_avatar_offset_x<R: Runtime>(app: &AppHandle<R>, value: f64) -> Result<()> {
    let clamped = value.clamp(-2.0, 2.0);
    write_optional_f64(app, KEY_AVATAR_OFFSET_X, Some(clamped))
}

pub fn set_avatar_offset_y<R: Runtime>(app: &AppHandle<R>, value: f64) -> Result<()> {
    let clamped = value.clamp(-2.0, 2.0);
    write_optional_f64(app, KEY_AVATAR_OFFSET_Y, Some(clamped))
}
