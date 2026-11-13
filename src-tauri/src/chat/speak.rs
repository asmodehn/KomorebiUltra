//! Optional TTS playback — fire-and-forget, plus text sanitization.

use tauri::{AppHandle, Wry};

/// Fire-and-forget TTS: if a TTS provider is configured, synthesize the
/// reply and emit it to the frontend for playback + Live2D lip-sync.
/// Any error is logged but never surfaced to the UI.
pub(super) async fn maybe_speak(app: &AppHandle<Wry>, text: String) {
    let clean = sanitize_for_tts(&text);
    if clean.trim().is_empty() {
        return;
    }
    tracing::info!(
        raw_len = text.len(),
        clean_len = clean.len(),
        preview = %clean.chars().take(120).collect::<String>(),
        "tts input"
    );
    let app = app.clone();
    tauri::async_runtime::spawn(async move {
        match crate::commands::synthesize_via_provider(&app, &clean).await {
            Ok(Some(wav)) => crate::commands::emit_tts_wav(&app, &wav),
            Ok(None) => {}
            Err(e) => tracing::warn!(%e, "tts synthesis failed"),
        }
    });
}

/// Strip markdown/code fences and other symbols Piper mispronounces as
/// clicks, buzzes, or garbled phonemes. Keeps letters, digits, basic
/// punctuation, and common Unicode letters (Cyrillic, etc.).
pub(crate) fn sanitize_for_tts(text: &str) -> String {
    // First: drop any <mood:X> tags so they aren't pronounced as
    // "less-than mood colon happy greater-than".
    let stripped = strip_mood_tags(text);
    let stripped = strip_inline_tag_block(&stripped, "tool_call");
    let stripped = strip_inline_tag_block(&stripped, "tool_status");
    // Remove fenced code blocks entirely.
    let mut out = String::with_capacity(stripped.len());
    let mut in_fence = false;
    for line in stripped.lines() {
        if line.trim_start().starts_with("```") {
            in_fence = !in_fence;
            continue;
        }
        if in_fence {
            continue;
        }
        out.push_str(line);
        out.push('\n');
    }
    // Strip inline markdown markers and URL/path noise.
    let mut buf = String::with_capacity(out.len());
    let mut prev_space = true;
    for ch in out.chars() {
        let keep = match ch {
            // Preserve sentence structure.
            '.' | ',' | '!' | '?' | ':' | ';' | '\'' | '"' | '-' | '\n' | ' ' => true,
            // Remove markdown noise / code symbols that Piper pronounces as
            // static ("asterisk", "hash", "underscore", backtick clicks).
            '*' | '#' | '`' | '_' | '~' | '[' | ']' | '(' | ')' | '{' | '}' | '<' | '>' | '|'
            | '\\' | '/' | '=' | '+' => false,
            c if c.is_alphanumeric() => true,
            _ => false,
        };
        if keep {
            if ch.is_whitespace() {
                if !prev_space {
                    buf.push(' ');
                    prev_space = true;
                }
            } else {
                buf.push(ch);
                prev_space = false;
            }
        } else if !prev_space {
            // Collapse a removed symbol into a single space to keep word
            // boundaries, e.g. "foo*bar*baz" → "foo bar baz".
            buf.push(' ');
            prev_space = true;
        }
    }
    buf.trim().to_string()
}

/// Remove `<mood:NAME>` markers (case-insensitive). Cheap O(n) scan, no regex.
fn strip_mood_tags(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let bytes = text.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'<'
            && bytes
                .get(i + 1..i + 6)
                .map(|s| s.eq_ignore_ascii_case(b"mood:"))
                .unwrap_or(false)
        {
            // Find closing '>'.
            if let Some(rel) = bytes[i + 6..].iter().position(|&c| c == b'>') {
                i += 6 + rel + 1;
                continue;
            }
        }
        // Push next UTF-8 char as a whole.
        let ch_len = utf8_char_len(bytes[i]);
        let end = (i + ch_len).min(bytes.len());
        if let Ok(s) = std::str::from_utf8(&bytes[i..end]) {
            out.push_str(s);
        }
        i = end;
    }
    out
}

/// Remove `<NAME>...</NAME>` blocks from `text`. Used to keep tool-call
/// protocol markers out of TTS audio.
fn strip_inline_tag_block(text: &str, name: &str) -> String {
    let open = format!("<{name}>");
    let close = format!("</{name}>");
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(start) = rest.find(&open) {
        out.push_str(&rest[..start]);
        let after = &rest[start + open.len()..];
        if let Some(end) = after.find(&close) {
            rest = &after[end + close.len()..];
        } else {
            // Unterminated: drop the rest to be safe.
            rest = "";
            break;
        }
    }
    out.push_str(rest);
    out
}

#[inline]
fn utf8_char_len(b: u8) -> usize {
    // ASCII or unexpected continuation byte (shouldn't happen at boundary).
    if b < 0xC0 {
        1
    } else if b < 0xE0 {
        2
    } else if b < 0xF0 {
        3
    } else {
        4
    }
}

#[cfg(test)]
mod tests {
    use super::sanitize_for_tts;

    #[test]
    fn strips_kaomoji_symbols_from_reaction_lines() {
        assert_eq!(sanitize_for_tts("Эй~ щекотно~ ^_^ ♪"), "Эй щекотно");
        assert_eq!(sanitize_for_tts("*хихикает* Привет!~"), "хихикает Привет!");
    }

    #[test]
    fn symbol_only_text_becomes_empty() {
        assert_eq!(sanitize_for_tts("~ ^_^ ~"), "");
    }
}
