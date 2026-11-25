//! Dev-only agent harness: token-friendly frames plus a file command channel.
//!
//! Enabled by `KOMOREBI_AGENT_DIR=<dir>` (debug builds only). Every
//! `POLL_MS` it:
//!
//! * executes the newline-separated commands in `<dir>/cmd`, then deletes it,
//! * writes `<dir>/state.json` when it changes,
//! * captures `<dir>/frame-<n>.png` on demand (`frame`, `watch <ms>`).
//!
//! Commands:
//!
//! ```text
//! frame                 capture one frame now
//! watch <ms>            capture every <ms> ms (0 = off)
//! eval <js>             run JS in the webview (drives the real UI)
//! ```
//!
//! The frame is the window area only: a PNG of the webview (see
//! `crate::window::frame`), downscaled to at most `MAX_W` px wide and
//! flattened over a checkerboard so transparent (click-through) areas are
//! visible. `state.json` carries the same frame as text: window geometry,
//! the selected surface backend, OS input rects and the frontend's UI snapshot.

use tauri::AppHandle;

/// Stores the latest UI snapshot pushed by the frontend (dev harness only).
pub fn report(json: String) {
    #[cfg(debug_assertions)]
    imp::report(json);
    #[cfg(not(debug_assertions))]
    let _ = json;
}

pub fn spawn(app: AppHandle) {
    #[cfg(debug_assertions)]
    imp::spawn(app);
    #[cfg(not(debug_assertions))]
    let _ = app;
}

#[cfg(debug_assertions)]
mod imp {
    use std::path::{Path, PathBuf};
    use std::sync::{mpsc, Mutex, OnceLock};
    use std::time::{Duration, Instant};

    use tauri::{AppHandle, Manager, WebviewWindow};

    use crate::window::capture_frame;

    const POLL_MS: u64 = 250;
    /// Global cursor sampling interval (~30 Hz). Observations are also gated
    /// by `EMIT_GATE_MS`, so this bounds emission to at most 30 Hz.
    #[cfg(target_os = "linux")]
    const CURSOR_POLL_MS: u64 = 34;
    /// A change must be this old before it becomes a new observation.
    const EMIT_GATE_MS: u64 = 34;
    /// Newest-last observation ring, bounded so a long session cannot grow it.
    const MAX_OBSERVATIONS: usize = 32;
    /// Exact source name reported in capability, records and log lines.
    const X11_SOURCE: &str = "x11-xquerypointer";
    /// Wait for a repaint before an on-demand capture. A snapshot taken in the
    /// same tick as an `eval` can still show the old frame, which reads as
    /// "the click did nothing".
    const SETTLE_MS: u64 = 300;
    /// Frames are tokenized by area; keep them small by default.
    const MAX_W: u32 = 480;
    const CHECKER: u32 = 8;

    static UI_SNAPSHOT: Mutex<Option<String>> = Mutex::new(None);
    static STARTED: OnceLock<Instant> = OnceLock::new();

    /// One sample from a real cursor source. Timing is captured at sample
    /// time, never when the harness thread happens to publish state.json.
    struct CursorSample {
        x: i32,
        y: i32,
        at_ms: u64,
    }

    /// The selected read-only source and its capability. No source is a first-
    /// class outcome: unsupported platforms report a reason, never a position.
    struct CursorSelection {
        capability: CursorCapability,
        samples: Option<mpsc::Receiver<CursorSample>>,
    }

    impl CursorSelection {
        fn none() -> Self {
            const REASON: &str = "no passive global pointer source selected";
            tracing::info!("agent frame: global cursor unavailable reason={REASON}");
            Self {
                capability: CursorCapability::unavailable(REASON),
                samples: None,
            }
        }
    }

    /// Opt-in only: the host has DISPLAY=:1 and passive polling of the
    /// operator's display is not wanted. X11 is never auto-selected.
    #[cfg(target_os = "linux")]
    fn select_cursor_source() -> CursorSelection {
        if std::env::var("KOMOREBI_CURSOR_SOURCE").as_deref() != Ok("x11") {
            return CursorSelection::none();
        }
        let display = std::env::var("KOMOREBI_CURSOR_DISPLAY")
            .or_else(|_| std::env::var("DISPLAY"))
            .unwrap_or_default();
        match x11::X11Cursor::open(&display) {
            Ok(cursor) => {
                tracing::info!("cursor source selected source={X11_SOURCE}");
                let (tx, rx) = mpsc::channel();
                std::thread::spawn(move || poll_cursor(cursor, tx));
                CursorSelection {
                    capability: CursorCapability::available(X11_SOURCE),
                    samples: Some(rx),
                }
            }
            Err(reason) => {
                tracing::info!("cursor source unavailable source={X11_SOURCE} reason={reason}");
                CursorSelection {
                    capability: CursorCapability::unavailable(&reason),
                    samples: None,
                }
            }
        }
    }

    #[cfg(not(target_os = "linux"))]
    fn select_cursor_source() -> CursorSelection {
        CursorSelection::none()
    }

    /// Absolute CLOCK_MONOTONIC milliseconds, the same clock the harness X
    /// probe prints in its MARK lines so records can be bound to an oracle.
    #[cfg(target_os = "linux")]
    fn monotonic_ms() -> u64 {
        let mut ts = libc::timespec {
            tv_sec: 0,
            tv_nsec: 0,
        };
        unsafe { libc::clock_gettime(libc::CLOCK_MONOTONIC, &mut ts) };
        (ts.tv_sec as u64).saturating_mul(1000) + (ts.tv_nsec as u64) / 1_000_000
    }

    #[cfg(target_os = "linux")]
    fn poll_cursor(cursor: x11::X11Cursor, tx: mpsc::Sender<CursorSample>) {
        loop {
            if let Some((x, y)) = cursor.query() {
                let at_ms = monotonic_ms();
                if tx.send(CursorSample { x, y, at_ms }).is_err() {
                    break;
                }
            }
            std::thread::sleep(Duration::from_millis(CURSOR_POLL_MS));
        }
    }

    /// Minimal read-only X11 adapter. `x11-dl` resolves libX11 at runtime;
    /// nothing here injects input or changes the window's input shape.
    #[cfg(target_os = "linux")]
    mod x11 {
        use std::ffi::CString;
        use std::os::raw::{c_int, c_uint, c_ulong};

        use x11_dl::xlib::{Display, Xlib};

        pub(super) struct X11Cursor {
            xlib: Xlib,
            display: DisplayHandle,
            root: c_ulong,
        }

        // The connection is used only by the polling thread that owns it.
        struct DisplayHandle(*mut Display);
        unsafe impl Send for DisplayHandle {}

        impl X11Cursor {
            pub(super) fn open(name: &str) -> Result<Self, String> {
                let xlib = Xlib::open().map_err(|_| "X11 library unavailable".to_string())?;
                let c_name = CString::new(name).map_err(|_| format!("invalid DISPLAY={name}"))?;
                let display = unsafe { (xlib.XOpenDisplay)(c_name.as_ptr()) };
                if display.is_null() {
                    return Err(format!("XOpenDisplay failed for DISPLAY={name}"));
                }
                let root = unsafe { (xlib.XDefaultRootWindow)(display) };
                Ok(Self {
                    xlib,
                    display: DisplayHandle(display),
                    root,
                })
            }

            /// Root-relative pointer position, matching `XQueryPointer` on the
            /// X server. `None` means this instant could not be observed.
            pub(super) fn query(&self) -> Option<(i32, i32)> {
                let mut root_ret: c_ulong = 0;
                let mut child_ret: c_ulong = 0;
                let mut root_x: c_int = 0;
                let mut root_y: c_int = 0;
                let mut win_x: c_int = 0;
                let mut win_y: c_int = 0;
                let mut mask: c_uint = 0;
                let ok = unsafe {
                    (self.xlib.XQueryPointer)(
                        self.display.0,
                        self.root,
                        &mut root_ret,
                        &mut child_ret,
                        &mut root_x,
                        &mut root_y,
                        &mut win_x,
                        &mut win_y,
                        &mut mask,
                    )
                };
                if ok == 0 {
                    None
                } else {
                    Some((root_x, root_y))
                }
            }
        }

        impl Drop for X11Cursor {
            fn drop(&mut self) {
                unsafe { (self.xlib.XCloseDisplay)(self.display.0) };
            }
        }
    }

    pub(super) fn report(json: String) {
        if let Ok(mut slot) = UI_SNAPSHOT.lock() {
            *slot = Some(json);
        }
    }

    pub(super) fn spawn(app: AppHandle) {
        let Some(dir) = std::env::var_os("KOMOREBI_AGENT_DIR").map(PathBuf::from) else {
            return;
        };
        if let Err(e) = std::fs::create_dir_all(&dir) {
            tracing::warn!(?e, ?dir, "agent frame: cannot create dir");
            return;
        }
        STARTED.get_or_init(Instant::now);
        tracing::info!(?dir, "agent frame harness enabled");
        std::thread::spawn(move || run(app, dir));
    }

    fn run(app: AppHandle, dir: PathBuf) {
        let mut seq = 0u32;
        let mut watch_ms = 0u64;
        let mut last_watch = Instant::now();
        let mut last_state = String::new();
        let mut harness: Option<AgentFrameState> = None;
        let mut samples: Option<mpsc::Receiver<CursorSample>> = None;

        loop {
            std::thread::sleep(Duration::from_millis(POLL_MS));
            let Some(win) = app.get_webview_window("main") else {
                continue;
            };

            let mut snap = snapshot(&win);
            if harness.is_none() {
                let selection = select_cursor_source();
                samples = selection.samples;
                tracing::info!(
                    "agent frame: window context unavailable reason={}",
                    crate::sysctx::NO_WINDOW_SOURCE_REASON
                );
                harness = Some(AgentFrameState::new(snap, selection.capability));
                snap = snapshot(&win);
            }
            let state = harness.as_mut().expect("harness initialized");
            state.refresh(snap);

            // Real-source samples are queued by the X11 poll thread and folded
            // into the same change-only, rate-limited observation path as fake
            // commands. Publishing stays at POLL_MS; `at` keeps sample truth.
            if let Some(rx) = &samples {
                while let Ok(sample) = rx.try_recv() {
                    if state
                        .observe(sample.x, sample.y, X11_SOURCE, sample.at_ms)
                        .is_some()
                    {
                        tracing::info!(
                            "cursor observation kind=cursor:global source={} at={} x={} y={}",
                            X11_SOURCE,
                            sample.at_ms,
                            sample.x,
                            sample.y
                        );
                    }
                }
            }

            if let Ok(text) = std::fs::read_to_string(dir.join("cmd")) {
                let _ = std::fs::remove_file(dir.join("cmd"));
                let _events = state.consume(&text);
                if let Some(window) = state.last_window().cloned() {
                    crate::sysctx::set_window_context(window);
                }
                for command in parse_commands(&text) {
                    exec(&win, &dir, command, &mut seq, &mut watch_ms);
                }
            }

            if watch_ms > 0 && last_watch.elapsed() >= Duration::from_millis(watch_ms) {
                last_watch = Instant::now();
                shoot(&win, &dir, &mut seq, 0);
            }

            let state = state.publish().to_string();
            if state != last_state {
                let _ = std::fs::write(dir.join("state.json"), &state);
                last_state = state;
            }
        }
    }

    #[derive(Debug, PartialEq)]
    enum Command<'a> {
        Eval(&'a str),
        Watch(u64),
        Frame,
        Hide,
        Focus,
        Gtk,
        Show,
        /// A read-only global pointer observation injected by the harness.
        Cursor {
            x: i32,
            y: i32,
            source: String,
            at_ms: u64,
        },
        /// A read-only window-context observation injected by the harness.
        /// Only the low-risk fields are parsed; trailing tokens are ignored.
        Window(crate::sysctx::WindowContext),
        /// A `window` row whose app is not a minimal identity. No observation.
        InvalidApp,
        Unknown(&'a str),
    }

    fn parse_commands(text: &str) -> Vec<Command<'_>> {
        text.lines()
            .map(str::trim)
            .filter(|line| !line.is_empty())
            .map(parse_command)
            .collect()
    }

    fn parse_command(line: &str) -> Command<'_> {
        if let Some(js) = line.strip_prefix("eval ") {
            return Command::Eval(js);
        }
        if let Some(ms) = line.strip_prefix("watch ") {
            return Command::Watch(ms.trim().parse().unwrap_or(0));
        }
        if let Some(rest) = line.strip_prefix("cursor ") {
            let mut parts = rest.split_whitespace();
            if let (Some(x), Some(y), Some(source), Some(at), None) = (
                parts.next(),
                parts.next(),
                parts.next(),
                parts.next(),
                parts.next(),
            ) {
                if let (Ok(x), Ok(y), Ok(at_ms)) = (x.parse(), y.parse(), at.parse()) {
                    return Command::Cursor {
                        x,
                        y,
                        source: source.to_string(),
                        at_ms,
                    };
                }
            }
        }
        if let Some(rest) = line.strip_prefix("window ") {
            // Refuse the identity before parsing anything else: a malformed
            // row must not fall through to the unknown-command log with the
            // rejected token still in it.
            let mut app_parts = rest.split_whitespace();
            let _ = app_parts.next(); // identity
            if app_parts
                .next()
                .is_some_and(|app| !crate::sysctx::minimal_app(app))
            {
                return Command::InvalidApp;
            }
            let mut parts = rest.split_whitespace();
            let fields = (
                parts.next(),
                parts.next(),
                parts.next(),
                parts.next(),
                parts.next(),
                parts.next(),
                parts.next(),
                parts.next(),
                parts.next(),
                parts.next(),
                parts.next(),
            );
            if let (
                Some(id),
                Some(app),
                Some(x),
                Some(y),
                Some(w),
                Some(h),
                Some(output),
                Some(workspace),
                Some(state),
                Some(source),
                Some(at),
            ) = fields
            {
                if let (Ok(x), Ok(y), Ok(w), Ok(h), Ok(at_ms)) =
                    (x.parse(), y.parse(), w.parse(), h.parse(), at.parse())
                {
                    return Command::Window(crate::sysctx::WindowContext {
                        id: id.to_string(),
                        app: app.to_string(),
                        x,
                        y,
                        w,
                        h,
                        output: output.to_string(),
                        workspace: workspace.to_string(),
                        state: state.to_string(),
                        source: source.to_string(),
                        at_ms,
                    });
                }
            }
        }
        match line {
            "frame" => Command::Frame,
            "hide" => Command::Hide,
            "focus" => Command::Focus,
            "gtk" => Command::Gtk,
            "show" => Command::Show,
            other => Command::Unknown(other),
        }
    }

    fn exec(
        win: &WebviewWindow,
        dir: &Path,
        command: Command<'_>,
        seq: &mut u32,
        watch_ms: &mut u64,
    ) {
        match command {
            Command::Eval(js) => {
                if let Err(e) = win.eval(js) {
                    tracing::warn!(?e, "agent frame: eval failed");
                }
            }
            Command::Watch(ms) => {
                *watch_ms = ms;
                tracing::info!(watch_ms, "agent frame: watch interval");
            }
            Command::Frame => shoot(win, dir, seq, SETTLE_MS),
            Command::Hide => {
                let _ = win.hide();
                tracing::info!("agent frame: window hidden");
            }
            Command::Focus => {
                // `focus: false` in tauri.conf.json makes tao create the
                // window with accept-focus off; restore it and activate.
                let _ = win.set_focusable(true);
                match win.set_focus() {
                    Ok(()) => tracing::info!("agent frame: window focused"),
                    Err(e) => tracing::warn!(?e, "agent frame: focus failed"),
                }
            }
            Command::Gtk => gtk_probe(win),
            Command::Show => {
                let _ = win.show();
                tracing::info!("agent frame: window shown");
                shoot(win, dir, seq, SETTLE_MS);
            }
            // Observation commands are consumed by `AgentFrameState::consume`;
            // they never reach a window or OS input operation.
            Command::Cursor { .. } => {}
            Command::Window(_) => {}
            Command::InvalidApp => {}
            Command::Unknown(other) => tracing::warn!(cmd = other, "agent frame: unknown command"),
        }
    }

    /// Dump GTK focus state: whether keys can reach the webview at all.
    fn gtk_probe(win: &WebviewWindow) {
        let gtk_win = match win.gtk_window() {
            Ok(w) => w,
            Err(e) => {
                tracing::warn!(?e, "agent frame: no gtk window");
                return;
            }
        };
        use gtk::prelude::*;
        tracing::info!(
            active = gtk_win.is_active(),
            toplevel_focus = gtk_win.has_toplevel_focus(),
            accept_focus = gtk_win.accepts_focus(),
            focused = win.is_focused().unwrap_or(false),
            "agent frame: gtk window state"
        );
        let _ = win.with_webview(|platform| {
            let view = platform.inner();
            tracing::info!(
                widget_focus = view.has_focus(),
                can_focus = view.can_focus(),
                is_focus = view.is_focus(),
                "agent frame: webview focus state"
            );
        });
    }

    fn shoot(win: &WebviewWindow, dir: &Path, seq: &mut u32, settle_ms: u64) {
        if settle_ms > 0 {
            std::thread::sleep(Duration::from_millis(settle_ms));
        }
        let png = match capture_frame(win) {
            Ok(png) => png,
            Err(e) => {
                tracing::warn!(%e, "agent frame: capture failed");
                return;
            }
        };
        let small = match shrink(&png) {
            Ok(png) => png,
            Err(e) => {
                tracing::warn!(%e, "agent frame: shrink failed");
                return;
            }
        };
        *seq += 1;
        let path = dir.join(format!("frame-{seq:04}.png"));
        if let Err(e) = std::fs::write(&path, small) {
            tracing::warn!(?e, ?path, "agent frame: write failed");
        } else {
            tracing::info!(?path, "agent frame: captured");
        }
    }

    /// Downscale to `MAX_W` and flatten over a checkerboard so transparent
    /// areas are recognizable instead of silently black.
    fn shrink(png: &[u8]) -> Result<Vec<u8>, String> {
        let img = image::load_from_memory(png)
            .map_err(|e| e.to_string())?
            .to_rgba8();
        let (w, h) = img.dimensions();
        if w == 0 || h == 0 {
            return Err("empty frame".into());
        }
        let scale = (MAX_W as f32 / w as f32).min(1.0);
        let nw = ((w as f32 * scale).round() as u32).max(1);
        let nh = ((h as f32 * scale).round() as u32).max(1);
        let small = image::imageops::resize(&img, nw, nh, image::imageops::FilterType::Triangle);

        let mut out = image::RgbaImage::new(nw, nh);
        for (x, y, p) in out.enumerate_pixels_mut() {
            let dark = ((x / CHECKER) + (y / CHECKER)).is_multiple_of(2);
            *p = if dark {
                image::Rgba([32, 32, 40, 255])
            } else {
                image::Rgba([44, 44, 56, 255])
            };
        }
        image::imageops::overlay(&mut out, &small, 0, 0);

        let mut buf = Vec::new();
        out.write_to(&mut std::io::Cursor::new(&mut buf), image::ImageFormat::Png)
            .map_err(|e| e.to_string())?;
        Ok(buf)
    }

    pub(super) struct StateSnapshot<'a> {
        t_ms: u64,
        backend: &'a str,
        label: &'a str,
        visible: bool,
        focused: bool,
        size: Option<(u32, u32)>,
        scale: f64,
        rects: Vec<(i32, i32, i32, i32)>,
        ui: Option<serde_json::Value>,
    }

    fn snapshot(win: &WebviewWindow) -> StateSnapshot<'_> {
        let started = STARTED.get_or_init(Instant::now);
        StateSnapshot {
            t_ms: started.elapsed().as_millis() as u64,
            backend: crate::platform::surface().name(),
            label: win.label(),
            visible: win.is_visible().unwrap_or(false),
            focused: win.is_focused().unwrap_or(false),
            size: win.inner_size().ok().map(|s| (s.width, s.height)),
            scale: win.scale_factor().unwrap_or(1.0),
            rects: crate::commands::overlay::last_rects(),
            ui: UI_SNAPSHOT
                .lock()
                .ok()
                .and_then(|s| s.clone())
                .and_then(|s| serde_json::from_str::<serde_json::Value>(&s).ok()),
        }
    }

    /// What a global cursor source can report, and why it cannot when absent.
    /// Strings (not statics) let the no-display reason name the actual DISPLAY.
    pub(super) struct CursorCapability {
        available: bool,
        source: Option<String>,
        reason: Option<String>,
    }

    impl CursorCapability {
        pub(super) fn unavailable(reason: &str) -> Self {
            Self {
                available: false,
                source: None,
                reason: Some(reason.to_string()),
            }
        }

        pub(super) fn available(source: &str) -> Self {
            Self {
                available: true,
                source: Some(source.to_string()),
                reason: None,
            }
        }

        fn value(&self) -> serde_json::Value {
            serde_json::json!({
                "available": self.available,
                "source": self.source,
                "reason": self.reason,
            })
        }
    }

    /// Windows reports are owned here so observation state lives with the
    /// harness, not in a process-global sink shared between tests.
    struct OwnedSnapshot {
        t_ms: u64,
        backend: String,
        label: String,
        visible: bool,
        focused: bool,
        size: Option<(u32, u32)>,
        scale: f64,
        rects: Vec<(i32, i32, i32, i32)>,
        ui: Option<serde_json::Value>,
    }

    impl From<StateSnapshot<'_>> for OwnedSnapshot {
        fn from(s: StateSnapshot<'_>) -> Self {
            Self {
                t_ms: s.t_ms,
                backend: s.backend.to_string(),
                label: s.label.to_string(),
                visible: s.visible,
                focused: s.focused,
                size: s.size,
                scale: s.scale,
                rects: s.rects,
                ui: s.ui,
            }
        }
    }

    /// One low-risk window record, shared by the command event and the JSON
    /// written to `state.json`. The forbidden tokens in a command never reach
    /// this function, so they cannot appear in either output.
    fn window_event(window: &crate::sysctx::WindowContext) -> serde_json::Value {
        serde_json::json!({
            "kind": "window:context",
            "source": window.source,
            "at": window.at_ms,
            "payload": {
                "id": window.id,
                "app": window.app,
                "x": window.x,
                "y": window.y,
                "w": window.w,
                "h": window.h,
                "output": window.output,
                "workspace": window.workspace,
                "state": window.state,
            },
        })
    }

    /// The harness consumes command text and publishes the value written to
    /// `state.json`. Cursor observations are read-only: they change no window
    /// state and attempt no OS input.
    pub(super) struct AgentFrameState {
        snapshot: OwnedSnapshot,
        cursor: CursorCapability,
        last: Option<serde_json::Value>,
        last_window: Option<crate::sysctx::WindowContext>,
        observations: Vec<serde_json::Value>,
        last_at: Option<u64>,
        last_pos: Option<(i32, i32)>,
        last_rejection: Option<&'static str>,
        input_calls: Vec<String>,
    }

    impl AgentFrameState {
        pub(super) fn new(snapshot: StateSnapshot<'_>, cursor: CursorCapability) -> Self {
            Self {
                snapshot: snapshot.into(),
                cursor,
                last: None,
                last_window: None,
                observations: Vec::new(),
                last_at: None,
                last_pos: None,
                last_rejection: None,
                input_calls: Vec::new(),
            }
        }

        pub(super) fn refresh(&mut self, snapshot: StateSnapshot<'_>) {
            self.snapshot = snapshot.into();
        }

        pub(super) fn consume(&mut self, text: &str) -> Vec<serde_json::Value> {
            let mut events = Vec::new();
            for command in parse_commands(text) {
                match command {
                    Command::Cursor {
                        x,
                        y,
                        source,
                        at_ms,
                    } => {
                        self.last_rejection = None;
                        if let Some(event) = self.observe(x, y, &source, at_ms) {
                            events.push(event);
                        }
                    }
                    Command::Window(window) => {
                        self.last_rejection = None;
                        events.push(self.observe_window(window));
                    }
                    Command::InvalidApp => {
                        self.last_rejection = Some(crate::sysctx::INVALID_APP_REASON);
                        events.push(serde_json::json!({
                            "kind": "window:context:rejected",
                            "reason": crate::sysctx::INVALID_APP_REASON,
                        }));
                    }
                    // The only harness command that changes what receives input.
                    Command::Focus => self.input_calls.push("focus".into()),
                    _ => {}
                }
            }
            events
        }

        fn push_observation(&mut self, event: serde_json::Value) {
            self.observations.push(event);
            if self.observations.len() > MAX_OBSERVATIONS {
                let excess = self.observations.len() - MAX_OBSERVATIONS;
                self.observations.drain(..excess);
            }
        }

        fn observe(
            &mut self,
            x: i32,
            y: i32,
            source: &str,
            at_ms: u64,
        ) -> Option<serde_json::Value> {
            let changed = self.last_pos != Some((x, y));
            let settled = self
                .last_at
                .is_none_or(|last| at_ms.saturating_sub(last) >= EMIT_GATE_MS);
            if !changed || !settled {
                return None;
            }
            let event = serde_json::json!({
                "kind": "cursor:global",
                "source": source,
                "at": at_ms,
                "payload": { "x": x, "y": y },
            });
            self.last = Some(event.clone());
            self.push_observation(event.clone());
            self.last_pos = Some((x, y));
            self.last_at = Some(at_ms);
            Some(event)
        }

        fn observe_window(&mut self, window: crate::sysctx::WindowContext) -> serde_json::Value {
            let event = window_event(&window);
            self.last_window = Some(window);
            self.push_observation(event.clone());
            event
        }

        pub(super) fn last_window(&self) -> Option<&crate::sysctx::WindowContext> {
            self.last_window.as_ref()
        }

        pub(super) fn publish(&self) -> serde_json::Value {
            let s = &self.snapshot;
            serde_json::json!({
                "t_ms": s.t_ms,
                "mode": "overlay",
                "backend": s.backend,
                "window": {
                    "label": s.label,
                    "visible": s.visible,
                    "focused": s.focused,
                    "size": s.size.map(|(w, h)| serde_json::json!({ "w": w, "h": h })),
                    "scale": s.scale,
                },
                "input_rects": s.rects,
                "ui": s.ui,
                "acp_context": {
                    "window_line": crate::sysctx::window_context_line(self.last_window.as_ref()),
                    "rejection": self.last_rejection,
                },
                "capabilities": {
                    "cursor:global": self.cursor.value(),
                    "window:context": {
                        "available": false,
                        "source": serde_json::Value::Null,
                        "reason": crate::sysctx::NO_WINDOW_SOURCE_REASON,
                    },
                },
                "last_observations": {
                    "cursor:global": self.last,
                    "window:context": self.last_window.as_ref().map(window_event),
                },
                "observations": self.observations,
            })
        }

        #[cfg(test)]
        pub(super) fn input_calls(&self) -> &[String] {
            &self.input_calls
        }
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        // Shared fixtures for the folded + new tests.
        const NO_SOURCE: &str = "no passive global pointer source";

        fn fixture_state(cursor: CursorCapability) -> AgentFrameState {
            AgentFrameState::new(
                StateSnapshot {
                    t_ms: 1234,
                    backend: "plain-window",
                    label: "main",
                    visible: true,
                    focused: false,
                    size: Some((640, 400)),
                    scale: 2.0,
                    rects: vec![(10, 20, 30, 40)],
                    ui: Some(serde_json::json!({"keys": []})),
                },
                cursor,
            )
        }

        // ---- T1: folded into the existing full-object state test ---------------
        /// Given a debug-build dev harness with no OS window source selected, no window
        ///   observation and a per-turn ACP context that reports the window as unavailable
        /// When the harness publishes its dev state record
        /// Then the record contains a dev-readable string exactly equal to
        ///   `- window context: unavailable reason=no passive global window source selected`
        /// And the existing published fields for capabilities, last observations, window
        ///   geometry, input rects, UI and the observation array stay as they are today
        ///
        /// Existing full-object equality: every retained state field is pinned here.
        #[test]
        fn state_describes_one_overlay_window() {
            let ui = serde_json::json!({
                "vw": 640,
                "vh": 400,
                "nodes": [{"tag": "button", "label": "Reset conversation", "pe": "auto"}],
                "focus": {"tag": "input", "value": "draft"},
                "keys": [],
            });
            let harness = AgentFrameState::new(
                StateSnapshot {
                    t_ms: 1234,
                    backend: "plain-window",
                    label: "main",
                    visible: true,
                    focused: false,
                    size: Some((640, 400)),
                    scale: 2.0,
                    rects: vec![(10, 20, 30, 40)],
                    ui: Some(ui.clone()),
                },
                CursorCapability::unavailable(NO_SOURCE),
            );
            let published = harness.publish();
            let expected_retained = serde_json::json!({
                "t_ms": 1234,
                "mode": "overlay",
                "backend": "plain-window",
                "window": {
                    "label": "main",
                    "visible": true,
                    "focused": false,
                    "size": {"w": 640, "h": 400},
                    "scale": 2.0,
                },
                "input_rects": [[10, 20, 30, 40]],
                "ui": ui,
                "capabilities": {
                    "cursor:global": {
                        "available": false,
                        "source": null,
                        "reason": NO_SOURCE,
                    },
                    "window:context": {
                        "available": false,
                        "source": null,
                        "reason": "no passive global window source selected",
                    }
                },
                "last_observations": {"cursor:global": null, "window:context": null},
                "observations": [],
            });
            let obj = published.as_object().expect("object");
            for (key, value) in expected_retained.as_object().unwrap() {
                assert_eq!(
                    obj.get(key),
                    Some(value),
                    "retained state field changed: {key}"
                );
            }
            let text = serde_json::to_string(&published).unwrap();
            const ACP_WINDOW_LINE: &str =
                "- window context: unavailable reason=no passive global window source selected";
            assert!(
                text.contains(&format!("\"{ACP_WINDOW_LINE}\"")),
                "published record must carry the exact ACP window line as a string"
            );
            assert!(!text.contains("\"- window context: available"));
        }

        // ---- T2: folded into the existing missing-UI state test ----------------
        /// Given a harness with no OS window source selected, no frontend snapshot and no window row
        /// When the harness publishes its state
        /// Then window:context is unavailable with a reason and has no last record
        /// And `acp_context.window_line` is exactly
        ///   `- window context: unavailable reason=no passive global window source selected`
        /// And no window record is invented from process/window fallback data
        /// And missing window/UI values stay null and the cursor report stays unavailable
        ///
        /// Existing missing-UI assertions: retained nulls and cursor capability stay pinned.
        #[test]
        fn state_without_frontend_snapshot_has_no_invented_ui() {
            let harness = AgentFrameState::new(
                StateSnapshot {
                    t_ms: 0,
                    backend: "wayland-layer-shell",
                    label: "main",
                    visible: false,
                    focused: false,
                    size: None,
                    scale: 1.0,
                    rects: vec![],
                    ui: None,
                },
                CursorCapability::unavailable(NO_SOURCE),
            );
            let state = harness.publish();
            assert_eq!(state["mode"], "overlay");
            assert_eq!(state["backend"], "wayland-layer-shell");
            assert!(state["ui"].is_null());
            assert!(state["window"]["size"].is_null());
            assert_eq!(state["input_rects"], serde_json::json!([]));
            assert_eq!(
                state["capabilities"]["cursor:global"],
                serde_json::json!({"available": false, "source": null, "reason": NO_SOURCE})
            );
            assert!(state["last_observations"]["cursor:global"].is_null());
            assert_eq!(
                state["capabilities"]["window:context"],
                serde_json::json!({"available":false,"source":null,"reason":"no passive global window source selected"})
            );
            assert!(state["last_observations"]["window:context"].is_null());
            assert_eq!(
                state["acp_context"]["window_line"],
                serde_json::json!(
                    "- window context: unavailable reason=no passive global window source selected"
                )
            );
            assert_eq!(state["observations"], serde_json::json!([]));
        }

        // ---- T3: folded into the existing command-parser test ------------------
        /// Given the harness command file with the existing commands, one cursor line (S2), one normal
        ///   eleven-field `window` row for `org.example.Editor`, and one `window` row whose app is a
        ///   rejected UUID and whose remaining fields are malformed (`nope` in the numeric x position,
        ///   the other ten fields present)
        /// When the harness parses its text
        /// Then each nonblank line becomes the command it names, including the normal `Window` and the
        ///   refused `InvalidApp`, and the refused raw line is never a `Command::Unknown`
        /// And unknown or differently cased lines are still not treated as commands
        ///
        /// Given the harness command file with the existing commands and one cursor line (S2)
        /// When the harness parses its text
        /// Then each nonblank line becomes the command it names, including cursor:global
        /// And unknown or differently cased lines are still not treated as commands
        #[test]
        fn file_commands_are_newline_separated_and_exact() {
            let commands = parse_commands(
                "frame\n watch 2000 \n\neval document.title\ncursor 412 388 harness 1700\nunknown\nFrame\n watch 0 \nwindow 0x2a org.example.Editor 100 200 800 600 DP-1 ws-3 active harness 1800\nwindow 0x2c 123e4567-e89b-12d3-a456-426614174000 nope 200 800 600 DP-1 ws-3 active harness 2000\n",
            );
            assert_eq!(
                commands,
                vec![
                    Command::Frame,
                    Command::Watch(2000),
                    Command::Eval("document.title"),
                    Command::Cursor {
                        x: 412,
                        y: 388,
                        source: "harness".to_string(),
                        at_ms: 1700,
                    },
                    Command::Unknown("unknown"),
                    Command::Unknown("Frame"),
                    Command::Watch(0),
                    Command::Window(crate::sysctx::WindowContext {
                        id: "0x2a".into(),
                        app: "org.example.Editor".into(),
                        x: 100,
                        y: 200,
                        w: 800,
                        h: 600,
                        output: "DP-1".into(),
                        workspace: "ws-3".into(),
                        state: "active".into(),
                        source: "harness".into(),
                        at_ms: 1800,
                    }),
                    Command::InvalidApp,
                ]
            );
            assert!(commands.iter().all(|c| {
                !matches!(c, Command::Unknown(s)
                    if s.contains("123e4567-e89b-12d3-a456-426614174000"))
            }));
        }

        // ---- T4: new, command text in -> published state out ------------------
        /// Given a debug-build dev harness with no OS window source selected, an existing last window
        ///   observation (`org.example.Viewer`), and rows for all eleven rejected app shapes, each shape
        ///   with the eleven well-formed low-risk fields, plus two rows with a rejected app and malformed
        ///   remaining fields: a UUID row with no remaining fields at all (the field-tuple match fails)
        ///   and an AKIA row with `nope` in the numeric x position (the field tuple matches, x.parse fails)
        /// When the harness consumes each row and publishes its dev state record
        /// Then the returned event is exactly `{"kind":"window:context:rejected","reason":"INVALID_APP"}`
        ///   and `acp_context.rejection` is exactly `INVALID_APP`, with `acp_context` holding exactly
        ///   `rejection` and `window_line`, and the rejected token appears in neither event nor record
        /// And no `window:context` observation is returned; the previous last window observation, the
        ///   observation array, the window capability and `acp_context.window_line` stay exactly as before
        /// And no input or focus operation is attempted
        /// And a later normal window row or an emitted cursor observation makes `acp_context.rejection`
        ///   null again, while a fresh harness starts null and goes to `INVALID_APP` after a refused row
        ///
        /// Given a debug-build dev harness with no OS window source selected, one normal `window`
        ///   row for `org.example.Editor`, and further `window` rows whose app token is a UUID
        ///   (bare and with an allowed suffix), AKIA, ASIA, a 32-hex run, a 40-alphanumeric run
        ///   (bare and `==`-padded), a path, a URL, and a labelled title or pixel value, each with
        ///   the same eleven low-risk fields
        /// When the harness consumes each row and publishes its dev state record
        /// Then each rejected row is refused with `INVALID_APP` observable in the returned events
        ///   or the published record, and no `window:context` observation is returned
        /// And the previous last window observation (the `org.example.Viewer` row, or null on a
        ///   fresh harness) and the existing observation array stay unchanged
        /// And the rejected token appears in neither the published record nor its ACP context line
        /// And the normal `org.example.Editor` row with the same eleven fields is still accepted
        ///   and published as before, with the exact ACP line already pinned below
        /// And the window capability stays unavailable with `no passive global window source selected`,
        ///   cursor/backend/window/input_rects/UI stay unchanged, and no input or focus call is attempted
        ///
        /// Given the same dev harness with no OS window source selected and one `window`
        ///   command carrying identity `0x2a`, app `org.example.Editor`, x=100, y=200,
        ///   w=800, h=600, output `DP-1`, workspace `ws-3`, state `active`, source `harness`,
        ///   timestamp 1800 and trailing forbidden title/pixel/path/URL/credential tokens
        /// When the harness consumes that one command and publishes its dev state record
        /// Then `acp_context.window_line` is exactly the per-turn ACP context line
        ///   `- window context: available source=harness at=1800 app=org.example.Editor x=100 y=200 w=800 h=600 state=active`
        /// And that line equals `window_context_line` for the newest consumed row and carries no
        ///   `id=`, `output=` or `workspace=` token
        /// And none of the five forbidden tokens appears anywhere in the published record
        /// And the newest consumed window record is still the published last window observation
        ///   while the window capability still reports that no OS source is selected
        /// And no input or focus operation is attempted
        ///
        /// Given a harness with no OS window source selected and no mutating access granted,
        ///   one cursor row and two competing window-context rows whose source and timestamp differ,
        ///   each row carrying five forbidden input tokens (title, pixel data, path, URL and
        ///   credential-like text) with no stated reason
        /// When the harness consumes the rows and publishes its state
        /// Then the newest window-context record is the second row, with its source and timestamp,
        ///   while the first record is still present in the observation array
        /// And each record holds only window identity, owning app, position and size, output,
        ///   workspace identity and state; none of the five forbidden tokens appears anywhere
        ///   in published state or in either event
        /// And the cursor observation, backend, window geometry, input rects and UI snapshot are unchanged
        /// And no OS input or focus change is attempted
        ///
        /// Existing cursor-row assertions: retained cursor and no-input behaviour stay pinned.
        #[test]
        fn harness_cursor_command_is_published_without_touching_input() {
            let mut harness = fixture_state(CursorCapability::unavailable(NO_SOURCE));
            let before = harness.publish();
            // Guard against null-equals-null false greens on the before side.
            assert_eq!(before["window"]["label"], "main");
            assert_eq!(before["input_rects"], serde_json::json!([[10, 20, 30, 40]]));
            assert_eq!(before["ui"], serde_json::json!({"keys": []}));

            let events = harness.consume("cursor 412 388 harness 1700\n\n");
            let after = harness.publish();

            let expected = serde_json::json!({
                "kind": "cursor:global",
                "source": "harness",
                "at": 1700,
                "payload": {"x": 412, "y": 388},
            });
            assert_eq!(events.len(), 1);
            assert_eq!(events[0], expected);
            assert_eq!(after["last_observations"]["cursor:global"], expected);
            assert_eq!(after["window"], before["window"]);
            assert_eq!(after["input_rects"], before["input_rects"]);
            assert_eq!(after["ui"], before["ui"]);
            assert_eq!(after["backend"], before["backend"]);
            assert_eq!(after["mode"], before["mode"]);
            assert_eq!(after["capabilities"]["cursor:global"]["available"], false);
            assert!(
                harness.input_calls().is_empty(),
                "a read-only observation must not attempt OS input or focus changes"
            );

            const FORBIDDEN_TITLE: &str = "TITLE_DO_NOT_EMIT_ACME_SALARY_42";
            const FORBIDDEN_PIXEL: &str = "PIXEL_DO_NOT_EMIT_AAABBBCCC";
            const FORBIDDEN_PATH: &str = "PATH_DO_NOT_EMIT_home_alexv_private";
            const FORBIDDEN_URL: &str = "URL_DO_NOT_EMIT_https_example_com_secret";
            const FORBIDDEN_CRED: &str = "CRED_DO_NOT_EMIT_token_abcd1234";

            let row_a = format!("window 0x2a org.example.Editor 100 200 800 600 DP-1 ws-3 active harness 1800 {FORBIDDEN_TITLE} {FORBIDDEN_PIXEL} {FORBIDDEN_PATH} {FORBIDDEN_URL} {FORBIDDEN_CRED}\n");
            let row_b = format!("window 0x2b org.example.Viewer 200 300 640 480 HDMI-1 ws-4 background harness-2 1900 {FORBIDDEN_TITLE} {FORBIDDEN_PIXEL} {FORBIDDEN_PATH} {FORBIDDEN_URL} {FORBIDDEN_CRED}\n");

            let ev_a = harness.consume(&row_a);
            let a = serde_json::json!({
              "kind":"window:context","source":"harness","at":1800,
              "payload":{"id":"0x2a","app":"org.example.Editor","x":100,"y":200,
                         "w":800,"h":600,"output":"DP-1","workspace":"ws-3","state":"active"}
            });
            assert_eq!(ev_a, vec![a.clone()]);
            let after_a = harness.publish();
            const ACP_WINDOW_LINE_A: &str = "- window context: available source=harness at=1800 app=org.example.Editor x=100 y=200 w=800 h=600 state=active";
            let line_a = after_a["acp_context"]["window_line"]
                .as_str()
                .expect("published ACP window line");
            assert_eq!(
                after_a["acp_context"]["window_line"],
                serde_json::json!(ACP_WINDOW_LINE_A)
            );
            assert_eq!(line_a, ACP_WINDOW_LINE_A);
            assert_eq!(
                line_a,
                crate::sysctx::window_context_line(Some(harness.last_window().unwrap()))
            );
            assert!(!line_a.contains("id="));
            assert!(!line_a.contains("output="));
            assert!(!line_a.contains("workspace="));
            let after_a_text = serde_json::to_string(&after_a).unwrap();
            assert_eq!(after_a["last_observations"]["window:context"], a);
            assert_eq!(
                after_a["capabilities"]["window:context"],
                serde_json::json!({"available":false,"source":null,"reason":"no passive global window source selected"})
            );
            assert!(harness.input_calls().is_empty());

            let ev_b = harness.consume(&row_b);
            let after = harness.publish();
            let b = serde_json::json!({
              "kind":"window:context","source":"harness-2","at":1900,
              "payload":{"id":"0x2b","app":"org.example.Viewer","x":200,"y":300,
                         "w":640,"h":480,"output":"HDMI-1","workspace":"ws-4","state":"background"}
            });
            assert_eq!(ev_b, vec![b.clone()]);

            assert_eq!(after["last_observations"]["window:context"], b);
            assert_eq!(
                after["last_observations"]["window:context"]["source"],
                serde_json::json!("harness-2")
            );
            assert_eq!(
                after["last_observations"]["window:context"]["at"],
                serde_json::json!(1900)
            );
            let obs = after["observations"].as_array().unwrap();
            assert!(obs.contains(&a));
            assert_eq!(obs.last().unwrap(), &b);

            let rejected_apps = [
                (
                    "uuid",
                    "123e4567-e89b-12d3-a456-426614174000",
                    "100 200 800 600 DP-1 ws-3 active harness 2000",
                ),
                (
                    "uuid-suffix",
                    "123e4567-e89b-12d3-a456-426614174000.x",
                    "100 200 800 600 DP-1 ws-3 active harness 2000",
                ),
                (
                    "akia",
                    "AKIAEXAMPLE1234567890",
                    "100 200 800 600 DP-1 ws-3 active harness 2000",
                ),
                (
                    "asia",
                    "ASIAEXAMPLE1234567890",
                    "100 200 800 600 DP-1 ws-3 active harness 2000",
                ),
                (
                    "hex32",
                    "9f3ad41c7b2e4a5180cd6f27ba39e412",
                    "100 200 800 600 DP-1 ws-3 active harness 2000",
                ),
                (
                    "alnum40",
                    "Ab12Cd34Ef56Gh78Ij90Kl12Mn34Op56Qr78St90",
                    "100 200 800 600 DP-1 ws-3 active harness 2000",
                ),
                (
                    "alnum40-padded",
                    "Ab12Cd34Ef56Gh78Ij90Kl12Mn34Op56Qr78St90==",
                    "100 200 800 600 DP-1 ws-3 active harness 2000",
                ),
                (
                    "path",
                    "/home/alexv/.config/com.example.App/secrets.ini",
                    "100 200 800 600 DP-1 ws-3 active harness 2000",
                ),
                (
                    "url",
                    "https://user:token9f3a@portal.invalid/app",
                    "100 200 800 600 DP-1 ws-3 active harness 2000",
                ),
                (
                    "title",
                    "title=ACME_SALARY_42",
                    "100 200 800 600 DP-1 ws-3 active harness 2000",
                ),
                (
                    "pixel",
                    "pixel=AAABBBCCC",
                    "100 200 800 600 DP-1 ws-3 active harness 2000",
                ),
                // Field-tuple match fails, and the field tuple matches but x does not parse.
                (
                    "uuid/missing-fields",
                    "123e4567-e89b-12d3-a456-426614174000",
                    "",
                ),
                (
                    "akia/bad-x",
                    "AKIAEXAMPLE1234567890",
                    "nope 200 800 600 DP-1 ws-3 active harness 2000",
                ),
            ];
            for (name, app, tail) in rejected_apps {
                let before = harness.publish();
                let row = format!("window 0x2c {app} {tail}\n");
                let events = harness.consume(&row);
                let after = harness.publish();
                let events_text = serde_json::to_string(&events).unwrap();
                let after_text = serde_json::to_string(&after).unwrap();
                let rejection = serde_json::json!({
                    "kind": "window:context:rejected",
                    "reason": "INVALID_APP"
                });

                assert!(
                    !events_text.contains("\"kind\":\"window:context\""),
                    "{name}: rejected row published a window observation"
                );
                assert_eq!(events, vec![rejection], "{name}: exact refusal event");
                assert_eq!(
                    after["acp_context"]["rejection"],
                    serde_json::json!("INVALID_APP"),
                    "{name}: named rejection absent from dev record"
                );
                let ctx: std::collections::BTreeSet<&str> = after["acp_context"]
                    .as_object()
                    .unwrap()
                    .keys()
                    .map(String::as_str)
                    .collect();
                assert_eq!(
                    ctx,
                    ["rejection", "window_line"].into_iter().collect(),
                    "{name}: acp_context keys changed"
                );
                assert!(
                    !events_text.contains(app) && !after_text.contains(app),
                    "{name}: rejected token leaked"
                );
                assert_eq!(
                    after["last_observations"]["window:context"],
                    before["last_observations"]["window:context"],
                    "{name}: previous last window observation changed"
                );
                assert_eq!(
                    after["observations"], before["observations"],
                    "{name}: observation array changed"
                );
                assert_eq!(
                    after["capabilities"]["window:context"],
                    before["capabilities"]["window:context"],
                    "{name}: window capability changed"
                );
                let line = after["acp_context"]["window_line"].as_str().unwrap();
                let previous = before["acp_context"]["window_line"].as_str().unwrap();
                assert!(!line.contains(app), "{name}: token leaked into ACP line");
                assert_eq!(line, previous, "{name}: ACP window line changed");
                assert!(harness.input_calls().is_empty(), "{name}: input attempted");
            }

            let mut fresh = fixture_state(CursorCapability::unavailable(NO_SOURCE));
            let before = fresh.publish();
            assert!(before["last_observations"]["window:context"].is_null());
            assert_eq!(before["observations"], serde_json::json!([]));
            let fresh_ctx: std::collections::BTreeSet<&str> = before["acp_context"]
                .as_object()
                .unwrap()
                .keys()
                .map(String::as_str)
                .collect();
            assert_eq!(
                fresh_ctx,
                ["rejection", "window_line"].into_iter().collect(),
                "fresh acp_context keys changed"
            );
            assert!(before["acp_context"]["rejection"].is_null());
            let fresh_events = fresh.consume(&format!(
                "window 0x2c {} 100 200 800 600 DP-1 ws-3 active harness 2000\n",
                rejected_apps[0].1
            ));
            let fresh_after = fresh.publish();
            assert!(
                !serde_json::to_string(&fresh_events)
                    .unwrap()
                    .contains("\"kind\":\"window:context\""),
                "null-before: rejected row published a window observation"
            );
            assert!(fresh_after["last_observations"]["window:context"].is_null());
            assert_eq!(fresh_after["observations"], serde_json::json!([]));
            assert_eq!(
                fresh_after["acp_context"]["rejection"],
                serde_json::json!("INVALID_APP")
            );
            assert!(
                !serde_json::to_string(&fresh_after)
                    .unwrap()
                    .contains(rejected_apps[0].1),
                "null-before: rejected token leaked"
            );

            let normal_events = harness.consume(&row_a);
            let normal_after = harness.publish();
            assert_eq!(normal_events, vec![a.clone()]);
            assert_eq!(
                normal_after["acp_context"]["rejection"],
                serde_json::Value::Null
            );
            assert!(normal_after["acp_context"]
                .as_object()
                .unwrap()
                .contains_key("rejection"));
            assert_eq!(
                normal_after["acp_context"]["window_line"],
                serde_json::json!(ACP_WINDOW_LINE_A)
            );

            let mut after_cursor = fixture_state(CursorCapability::unavailable(NO_SOURCE));
            after_cursor.consume(&format!(
                "window 0x2c {} 100 200 800 600 DP-1 ws-3 active harness 2000\n",
                rejected_apps[0].1
            ));
            assert_eq!(
                after_cursor.publish()["acp_context"]["rejection"],
                serde_json::json!("INVALID_APP")
            );
            after_cursor.consume("cursor 500 500 harness 3000\n");
            let cursor_after = after_cursor.publish();
            assert!(cursor_after["acp_context"]
                .as_object()
                .unwrap()
                .contains_key("rejection"));
            assert!(cursor_after["acp_context"]["rejection"].is_null());
            assert_eq!(
                cursor_after["last_observations"]["cursor:global"]["at"],
                serde_json::json!(3000)
            );
            assert!(cursor_after["last_observations"]["window:context"].is_null());
            assert!(after_cursor.input_calls().is_empty());

            assert_eq!(after["last_observations"]["cursor:global"], expected);
            assert_eq!(
                after["capabilities"]["window:context"],
                serde_json::json!({"available":false,"source":null,"reason":"no passive global window source selected"})
            );

            let top_before: std::collections::BTreeSet<&str> = before
                .as_object()
                .unwrap()
                .keys()
                .map(String::as_str)
                .collect();
            let top_after: std::collections::BTreeSet<&str> = after
                .as_object()
                .unwrap()
                .keys()
                .map(String::as_str)
                .collect();
            assert_eq!(top_after, top_before);
            let caps_after: std::collections::BTreeSet<&str> = after["capabilities"]
                .as_object()
                .unwrap()
                .keys()
                .map(String::as_str)
                .collect();
            assert_eq!(
                caps_after,
                ["cursor:global", "window:context"].into_iter().collect()
            );
            let last_after: std::collections::BTreeSet<&str> = after["last_observations"]
                .as_object()
                .unwrap()
                .keys()
                .map(String::as_str)
                .collect();
            assert_eq!(
                last_after,
                ["cursor:global", "window:context"].into_iter().collect()
            );

            assert_eq!(
                after["capabilities"]["cursor:global"],
                before["capabilities"]["cursor:global"]
            );
            assert_eq!(after["window"], before["window"]);
            assert_eq!(after["input_rects"], before["input_rects"]);
            assert_eq!(after["ui"], before["ui"]);
            assert_eq!(after["backend"], before["backend"]);
            assert_eq!(after["mode"], before["mode"]);
            assert!(harness.input_calls().is_empty());

            let state_text = serde_json::to_string(&after).unwrap();
            let ev_a_text = serde_json::to_string(&ev_a).unwrap();
            let ev_b_text = serde_json::to_string(&ev_b).unwrap();
            for token in [
                FORBIDDEN_TITLE,
                FORBIDDEN_PIXEL,
                FORBIDDEN_PATH,
                FORBIDDEN_URL,
                FORBIDDEN_CRED,
            ] {
                assert!(
                    !state_text.contains(token),
                    "forbidden input token leaked into state: {token}"
                );
                assert!(
                    !after_a_text.contains(token),
                    "forbidden input token leaked into published record: {token}"
                );
                assert!(
                    !ev_a_text.contains(token),
                    "forbidden input token leaked into event: {token}"
                );
                assert!(
                    !ev_b_text.contains(token),
                    "forbidden input token leaked into event: {token}"
                );
            }
        }

        // ---- T5: new, change-only + 30 Hz, time supplied by the command --------
        /// Given cursor commands with supplied times: different positions 10 ms and
        /// 33 ms after the first emit, a later different position after 50 ms,
        /// and a repeat of that position (S3 + contract)
        /// When the harness consumes them
        /// Then only the first, the after-interval change and a later change are new observations
        /// And each record keeps the timestamp given in its command, not the wall clock
        #[test]
        fn cursor_emission_is_change_only_and_rate_limited() {
            let mut harness = fixture_state(CursorCapability::unavailable(NO_SOURCE));

            let first = harness.consume(
                "cursor 412 388 harness 1000\ncursor 413 388 harness 1010\ncursor 414 388 harness 1033\n",
            );
            assert_eq!(first.len(), 1);
            assert_eq!(first[0]["at"], serde_json::json!(1000));
            assert_eq!(first[0]["payload"], serde_json::json!({"x": 412, "y": 388}));

            let after_interval =
                harness.consume("cursor 413 389 harness 1050\ncursor 413 389 harness 1100\n");
            assert_eq!(after_interval.len(), 1);
            assert_eq!(after_interval[0]["at"], serde_json::json!(1050));
            assert_eq!(
                after_interval[0]["payload"],
                serde_json::json!({"x": 413, "y": 389})
            );

            let later = harness.consume("cursor 414 389 harness 1150\n");
            assert_eq!(later.len(), 1);
            assert_eq!(later[0]["at"], serde_json::json!(1150));

            let state = harness.publish();
            assert_eq!(
                state["last_observations"]["cursor:global"]["at"],
                serde_json::json!(1150)
            );
            assert_eq!(
                state["last_observations"]["cursor:global"]["source"],
                serde_json::json!("harness")
            );
        }

        // ---- T6: new, observable consequence of the separate source (rule 2) ---
        /// Given two harnesses with the same surface backend and window reports but
        /// different cursor-source capabilities (rule 2, rule 3, S1)
        /// When each publishes state and one consumes a cursor command
        /// Then capability is not derived from the backend name
        /// And a present source is reported with its name while an absent one reports a reason
        /// And consuming an observation still attempts no input operation
        #[test]
        fn cursor_capability_is_separate_from_the_surface_backend() {
            let none = fixture_state(CursorCapability::unavailable(NO_SOURCE));
            let mut x11 = fixture_state(CursorCapability::available("fake-x11"));

            let no_source = none.publish();
            let with_source = x11.publish();

            assert_eq!(with_source["backend"], no_source["backend"]);
            assert_eq!(with_source["window"], no_source["window"]);
            assert_eq!(with_source["input_rects"], no_source["input_rects"]);
            assert_eq!(
                no_source["capabilities"]["cursor:global"],
                serde_json::json!({"available": false, "source": null, "reason": NO_SOURCE})
            );
            assert_eq!(
                with_source["capabilities"]["cursor:global"],
                serde_json::json!({"available": true, "source": "fake-x11", "reason": null})
            );
            assert!(no_source["last_observations"]["cursor:global"].is_null());
            assert!(with_source["last_observations"]["cursor:global"].is_null());

            let events = x11.consume("cursor 10 20 harness 2000\n");
            assert_eq!(events.len(), 1);
            assert_eq!(x11.publish()["backend"], no_source["backend"]);
            assert!(x11.input_calls().is_empty());
        }

        // ---- T7: bounded, newest-last observation ring (S6) ----------------
        /// Given 40 distinct fake observations spaced past the emit gate
        /// When the harness consumes them and publishes state
        /// Then the array keeps at least 16 but not all 40, in time order,
        /// and its newest record is still the last observation.
        #[test]
        fn observations_are_bounded_and_newest_last() {
            let mut harness = fixture_state(CursorCapability::unavailable(NO_SOURCE));
            let mut lines = String::new();
            for i in 0..40 {
                lines.push_str(&format!("cursor {i} 300 fake {}\n", 1000 + i * 40));
            }
            let events = harness.consume(&lines);
            let published = harness.publish();
            let observations = published["observations"].as_array().expect("array");
            let len = observations.len();
            assert_eq!(events.len(), 40);
            assert!(
                (16..40).contains(&len),
                "ring must keep at least 16 records and drop some of 40"
            );
            assert_eq!(observations[0]["payload"]["x"], serde_json::json!(40 - len));
            assert_eq!(observations[len - 1]["payload"]["x"], serde_json::json!(39));
            assert_eq!(observations[len - 1]["at"], serde_json::json!(2560));
            assert_eq!(
                published["last_observations"]["cursor:global"],
                observations[len - 1]
            );
        }

        /// Given a transparent webview frame containing one opaque control
        /// When the harness prepares the frame for an agent
        /// Then the PNG is at most 480 px wide with opaque checkerboard showing through transparency
        /// And the opaque control stays visible after downscaling
        #[test]
        fn frame_is_downscaled_and_transparency_is_visible() {
            let source = image::RgbaImage::from_fn(960, 480, |x, y| {
                if (100..200).contains(&x) && (100..200).contains(&y) {
                    image::Rgba([255, 0, 0, 255])
                } else {
                    image::Rgba([0, 0, 0, 0])
                }
            });
            let mut png = Vec::new();
            source
                .write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png)
                .unwrap();

            let shrunk = shrink(&png).unwrap();
            let frame = image::load_from_memory(&shrunk).unwrap().to_rgba8();

            assert!(frame.width() <= 480);
            assert!(frame.height() > 0);
            assert!(frame.pixels().all(|p| p.0[3] == 255));
            let first = frame.get_pixel(0, 0);
            let varied = frame
                .enumerate_pixels()
                .any(|(x, y, p)| !(45..105).contains(&x) && !(45..105).contains(&y) && p != first);
            assert!(varied, "transparent window area must show a checkerboard");
            let control = frame.get_pixel(75, 75);
            assert_eq!(control.0[3], 255);
            assert_ne!(*control, *first);
        }
    }
}
