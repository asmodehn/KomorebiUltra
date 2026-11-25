//! Machine + environment context exposed to the assistant.
//!
//! We collect a snapshot of cheap, privacy-bounded facts (OS, CPU, RAM,
//! GPUs, current time/date in the user's timezone) and render them as a
//! short system message that gets prepended to every prompt. Heavy or
//! sensitive data (processes, disks, network) is deliberately omitted —
//! add a dedicated skill if the user wants to query them.

use once_cell::sync::Lazy;
use parking_lot::Mutex;
use std::time::{Duration, Instant};
use sysinfo::System;

/// What the LLM sees as context. Regenerated at most once per
/// `STATIC_TTL`; time/date is always fresh because we render it at render
/// time, not when the snapshot is built.
#[derive(Debug, Clone)]
pub struct SystemSnapshot {
    pub os_long: String,
    pub cpu_brand: String,
    pub cpu_cores: usize,
    pub total_memory_gb: f64,
    pub gpus: Vec<String>,
    pub hostname: String,
}

/// Reported when no OS window source is selected. The harness can still inject
/// one observation; that does not turn an unselected OS source into a capability.
pub(crate) const NO_WINDOW_SOURCE_REASON: &str = "no passive global window source selected";

/// Reported when an app identity is not a minimal identity. The rejected token
/// is never carried into a record or a turn; only this name is.
pub(crate) const INVALID_APP_REASON: &str = "INVALID_APP";

/// The same minimal-identity boundary the perception probe applies to both
/// compositor events. A non-minimal token is refused whole: no shortening and
/// no presentation of the raw value.
pub(crate) fn minimal_app(app: &str) -> bool {
    let bytes = app.as_bytes();
    let minimal_chars = !bytes.is_empty()
        && bytes
            .iter()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'_' | b'-'));
    minimal_chars && !blob_like(bytes)
}

fn is_hex(b: &u8) -> bool {
    b.is_ascii_hexdigit()
}

fn hex_group(bytes: &[u8], offset: usize, len: usize) -> bool {
    bytes[offset..offset + len].iter().all(is_hex)
}

fn blob_like(bytes: &[u8]) -> bool {
    let n = bytes.len();
    // Hyphen-separated UUID shape with any suffix.
    for i in 0..n.saturating_sub(35) {
        if hex_group(bytes, i, 8)
            && bytes[i + 8] == b'-'
            && hex_group(bytes, i + 9, 4)
            && bytes[i + 13] == b'-'
            && hex_group(bytes, i + 14, 4)
            && bytes[i + 18] == b'-'
            && hex_group(bytes, i + 19, 4)
            && bytes[i + 23] == b'-'
            && hex_group(bytes, i + 24, 12)
        {
            return true;
        }
    }
    // Long opaque runs: hex blobs and base64-like or credential blobs.
    for i in 0..n {
        let hex = bytes[i..].iter().take_while(|b| is_hex(b)).count();
        let alnum = bytes[i..]
            .iter()
            .take_while(|b| b.is_ascii_alphanumeric())
            .count();
        if hex >= 32 || alnum >= 40 {
            return true;
        }
    }
    // AKIA / ASIA followed by at least twelve uppercase letters or digits.
    for i in 0..n.saturating_sub(3) {
        if bytes[i] != b'A'
            || (bytes[i + 1] != b'K' && bytes[i + 1] != b'S')
            || bytes[i + 2] != b'I'
            || bytes[i + 3] != b'A'
        {
            continue;
        }
        let tail = bytes[i + 4..]
            .iter()
            .take_while(|b| b.is_ascii_uppercase() || b.is_ascii_digit())
            .count();
        if tail >= 12 {
            return true;
        }
    }
    false
}

/// A read-only window observation. Dev tools may read the whole record; an ACP
/// turn carries only app, rectangle and state plus source and time. Titles,
/// pixels, paths, URLs and credential-like text are deliberately absent.
#[derive(Debug, Clone, PartialEq)]
pub struct WindowContext {
    pub id: String,
    pub app: String,
    pub x: i32,
    pub y: i32,
    pub w: i32,
    pub h: i32,
    pub output: String,
    pub workspace: String,
    pub state: String,
    pub source: String,
    pub at_ms: u64,
}

impl WindowContext {
    /// The bounded line an ACP turn carries. The opaque id, output and workspace
    /// stay in the observation record; only app, rectangle, state, source and
    /// time cross into the turn.
    fn context_line(&self) -> String {
        format!(
            "window context: available source={} at={} app={} x={} y={} w={} h={} state={}",
            self.source, self.at_ms, self.app, self.x, self.y, self.w, self.h, self.state
        )
    }
}

/// The exact window line a turn carries: one marker, no trailing newline.
/// The dev harness uses this with its own observation, never with the global one.
pub(crate) fn window_context_line(context: Option<&WindowContext>) -> String {
    match context {
        Some(window) if !minimal_app(&window.app) => {
            format!("- window context: unavailable reason={INVALID_APP_REASON}")
        }
        Some(window) => format!("- {}", window.context_line()),
        None => format!("- window context: unavailable reason={NO_WINDOW_SOURCE_REASON}"),
    }
}

/// Set by the dev harness when it consumes a window observation. No OS window
/// source writes here, so a process with no harness and no adapter reports
/// the unavailable reason below.
static WINDOW_CONTEXT: Lazy<Mutex<Option<WindowContext>>> = Lazy::new(|| Mutex::new(None));

pub fn set_window_context(context: WindowContext) {
    *WINDOW_CONTEXT.lock() = Some(context);
}

static CACHE: Lazy<Mutex<Option<(Instant, SystemSnapshot)>>> = Lazy::new(|| Mutex::new(None));
const STATIC_TTL: Duration = Duration::from_secs(300);

pub fn snapshot() -> SystemSnapshot {
    {
        let guard = CACHE.lock();
        if let Some((at, snap)) = guard.as_ref() {
            if at.elapsed() < STATIC_TTL {
                return snap.clone();
            }
        }
    }
    let fresh = build_snapshot();
    *CACHE.lock() = Some((Instant::now(), fresh.clone()));
    fresh
}

fn build_snapshot() -> SystemSnapshot {
    let mut sys = System::new_all();
    sys.refresh_cpu_all();
    sys.refresh_memory();

    let os_long = format!(
        "{} {} ({})",
        System::name().unwrap_or_else(|| "Unknown OS".into()),
        System::os_version().unwrap_or_default(),
        System::kernel_version().unwrap_or_default(),
    );
    let cpu_brand = sys
        .cpus()
        .first()
        .map(|c| c.brand().to_string())
        .unwrap_or_default();
    let cpu_cores = System::physical_core_count().unwrap_or_else(|| sys.cpus().len());
    let total_memory_gb = (sys.total_memory() as f64) / 1_073_741_824.0;
    let hostname = System::host_name().unwrap_or_default();
    let gpus = detect_gpus();
    SystemSnapshot {
        os_long,
        cpu_brand,
        cpu_cores,
        total_memory_gb,
        gpus,
        hostname,
    }
}

/// Best-effort GPU enumeration. On Windows we use `wmic` / PowerShell;
/// on Linux we shell out to `lspci`. Failures just return an empty list.
fn detect_gpus() -> Vec<String> {
    #[cfg(windows)]
    {
        use std::process::Command;
        // PowerShell is more reliable than wmic on Win11 (wmic is deprecated).
        let mut cmd = Command::new("powershell");
        cmd.args([
            "-NoProfile",
            "-NonInteractive",
            "-Command",
            "Get-CimInstance Win32_VideoController | Select-Object -ExpandProperty Name",
        ]);
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            cmd.creation_flags(0x0800_0000); // CREATE_NO_WINDOW
        }
        if let Ok(out) = cmd.output() {
            if out.status.success() {
                return String::from_utf8_lossy(&out.stdout)
                    .lines()
                    .map(|s| s.trim().to_string())
                    .filter(|s| !s.is_empty())
                    .collect();
            }
        }
    }
    #[cfg(target_os = "linux")]
    {
        use std::process::Command;
        if let Ok(out) = Command::new("lspci").output() {
            if out.status.success() {
                return String::from_utf8_lossy(&out.stdout)
                    .lines()
                    .filter(|l| l.contains("VGA") || l.contains("3D") || l.contains("Display"))
                    .map(|l| l.split(':').nth(2).unwrap_or(l).trim().to_string())
                    .collect();
            }
        }
    }
    Vec::new()
}

/// True when at least one detected GPU looks like an NVIDIA card —
/// used to pick a sensible default for local-LLM GPU offload.
pub fn has_nvidia_gpu() -> bool {
    snapshot().gpus.iter().any(|g| {
        g.to_lowercase().contains("nvidia")
            || g.to_lowercase().contains("geforce")
            || g.to_lowercase().contains("rtx")
            || g.to_lowercase().contains("gtx")
    })
}

/// Render the system + time context as a short markdown bullet list
/// suitable for a `system` role message. Language-agnostic — the LLM
/// will translate naturally when replying.
pub fn render_context_message() -> String {
    let snap = snapshot();
    let now = chrono::Local::now();
    let date = now.format("%A, %B %-d, %Y").to_string();
    let time = now.format("%H:%M").to_string();
    let tz = iana_time_zone::get_timezone().unwrap_or_else(|_| now.offset().to_string());

    let mut out = String::from("User environment (facts you may reference if relevant):\n");
    out.push_str(&format!("- current date: {date}\n"));
    out.push_str(&format!("- current time: {time} ({tz})\n"));
    out.push_str(&format!("- OS: {}\n", snap.os_long));
    if !snap.cpu_brand.is_empty() {
        out.push_str(&format!(
            "- CPU: {} ({} cores)\n",
            snap.cpu_brand, snap.cpu_cores
        ));
    }
    if snap.total_memory_gb > 0.0 {
        out.push_str(&format!("- RAM: {:.1} GB\n", snap.total_memory_gb));
    }
    if !snap.gpus.is_empty() {
        out.push_str(&format!("- GPU(s): {}\n", snap.gpus.join(", ")));
    }
    if !snap.hostname.is_empty() {
        out.push_str(&format!("- hostname: {}\n", snap.hostname));
    }
    out.push_str(&format!(
        "{}\n",
        window_context_line(WINDOW_CONTEXT.lock().as_ref())
    ));
    out.push_str("Use these only when the user asks; do not proactively reveal them.");
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Given a per-turn ACP context with no window observation and no OS source selected
    /// When the per-turn context message is rendered
    /// Then it reports exactly one window line,
    ///   `- window context: unavailable reason=no passive global window source selected`
    /// And the preamble, current date, current time, OS and trailer lines remain unchanged
    ///
    /// Given one mirror-shaped window row whose app is `org.example.Editor`, rectangle
    ///   x=100, y=200, w=800, h=600, state `active`, source `mirror` and timestamp 1800,
    ///   with opaque id `0x2a`, output `DP-1` and workspace `ws-3` supplied
    /// When that row is installed through `set_window_context` and the message is rendered
    /// Then exactly one window line appears with app, rectangle and state plus source and time,
    ///   followed by a newline
    /// And no `id=`, `output=` or `workspace=` token and no unavailable line appears
    /// And the preamble, current date, current time, OS and trailer lines remain unchanged
    ///
    /// Given a window row whose app is a UUID (bare and with an allowed suffix), AKIA, ASIA,
    ///   a 32-hex run, a 40-alphanumeric run (bare and `==`-padded), a path, a URL, or a
    ///   labelled title or pixel value
    /// When that row is installed through the same entry point and rendered
    /// Then no available line appears, the rejected token is absent, and the one window line is
    ///   `- window context: unavailable reason=INVALID_APP`
    /// And the preamble, date, time, OS and trailer lines remain unchanged
    #[test]
    fn context_message_reports_no_window_source() {
        let msg = render_context_message();
        assert!(msg.starts_with("User environment (facts you may reference if relevant):\n"));
        assert!(msg.contains("- current date: "));
        assert!(msg.contains("- current time: "));
        assert!(msg.contains("- OS: "));
        assert!(msg.ends_with("Use these only when the user asks; do not proactively reveal them."));
        assert!(msg.contains(
            "- window context: unavailable reason=no passive global window source selected"
        ));
        assert!(!msg.contains("- window context: available"));
        assert_eq!(
            msg.lines()
                .filter(|line| line.starts_with("- window context:"))
                .collect::<Vec<_>>(),
            vec!["- window context: unavailable reason=no passive global window source selected"]
        );

        struct RestoreNoWindowContext;
        impl Drop for RestoreNoWindowContext {
            fn drop(&mut self) {
                *WINDOW_CONTEXT.lock() = None;
            }
        }
        let _restore = RestoreNoWindowContext; // installed before the set, restores on panic too

        set_window_context(WindowContext {
            id: "0x2a".into(),
            app: "org.example.Editor".into(),
            x: 100,
            y: 200,
            w: 800,
            h: 600,
            output: "DP-1".into(),
            workspace: "ws-3".into(),
            state: "active".into(),
            source: "mirror".into(),
            at_ms: 1800,
        });
        let with_observation = render_context_message();
        const ACP_WINDOW_LINE: &str = "- window context: available source=mirror at=1800 app=org.example.Editor x=100 y=200 w=800 h=600 state=active";
        assert!(
            with_observation.contains(&format!("{ACP_WINDOW_LINE}\n")),
            "per-turn context message must render the exact window line"
        );
        assert_eq!(
            with_observation
                .lines()
                .filter(|line| line.starts_with("- window context:"))
                .collect::<Vec<_>>(),
            vec![ACP_WINDOW_LINE]
        );
        assert!(!with_observation.contains("id="));
        assert!(!with_observation.contains("output="));
        assert!(!with_observation.contains("workspace="));
        assert!(!with_observation.contains("- window context: unavailable"));
        assert!(with_observation
            .starts_with("User environment (facts you may reference if relevant):\n"));
        assert!(with_observation
            .ends_with("Use these only when the user asks; do not proactively reveal them."));

        let normal = WindowContext {
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
        };
        let rejected_apps = [
            ("uuid", "123e4567-e89b-12d3-a456-426614174000"),
            ("uuid-suffix", "123e4567-e89b-12d3-a456-426614174000.x"),
            ("akia", "AKIAEXAMPLE1234567890"),
            ("asia", "ASIAEXAMPLE1234567890"),
            ("hex32", "9f3ad41c7b2e4a5180cd6f27ba39e412"),
            ("alnum40", "Ab12Cd34Ef56Gh78Ij90Kl12Mn34Op56Qr78St90"),
            (
                "alnum40-padded",
                "Ab12Cd34Ef56Gh78Ij90Kl12Mn34Op56Qr78St90==",
            ),
            ("path", "/home/alexv/.config/com.example.App/secrets.ini"),
            ("url", "https://user:token9f3a@portal.invalid/app"),
            ("title", "title=ACME_SALARY_42"),
            ("pixel", "pixel=AAABBBCCC"),
        ];
        for (name, app) in rejected_apps {
            set_window_context(WindowContext {
                app: app.into(),
                ..normal.clone()
            });
            let msg = render_context_message();
            assert!(!msg.contains("- window context: available"), "{name}");
            assert!(!msg.contains(app), "{name}: rejected token leaked");
            assert!(msg.contains("INVALID_APP"), "{name}: named reason absent");
            assert_eq!(
                msg.lines()
                    .filter(|line| line.starts_with("- window context:"))
                    .collect::<Vec<_>>(),
                vec!["- window context: unavailable reason=INVALID_APP"],
                "{name}: exact window line"
            );
            assert!(
                msg.starts_with("User environment (facts you may reference if relevant):\n"),
                "{name}: preamble changed"
            );
            assert!(
                msg.ends_with("Use these only when the user asks; do not proactively reveal them."),
                "{name}: trailer changed"
            );
            assert!(msg.contains("- current date: "), "{name}: date line lost");
            assert!(msg.contains("- current time: "), "{name}: time line lost");
            assert!(msg.contains("- OS: "), "{name}: OS line lost");
        }
    }
}
