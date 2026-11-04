//! OpenRouter cloud client (streaming chat completions via SSE).
//!
//! API reference: https://openrouter.ai/docs/api-reference/chat-completion
//!
//! Security: the API key is never stored in this crate. Callers retrieve it
//! from a secure store (Phase 1: tauri-plugin-store; Phase 3 hardening: OS keyring)
//! and pass it in via [`OpenRouterClient::new`].

use futures::{Stream, StreamExt};
use komorebi_router::ChatMessage;
use serde::{Deserialize, Serialize};
use std::pin::Pin;

mod acp;
pub use acp::AcpClient;

pub const DEFAULT_MODEL: &str = "anthropic/claude-3.5-sonnet";
const OPENROUTER_URL: &str = "https://openrouter.ai/api/v1/chat/completions";

#[derive(thiserror::Error, Debug)]
pub enum CloudError {
    #[error("http: {0}")]
    Http(#[from] reqwest::Error),
    #[error("missing api key")]
    MissingApiKey,
    #[error("api error {status}: {body}")]
    Api { status: u16, body: String },
    #[error("malformed stream: {0}")]
    Stream(String),
    #[error("acp agent: {0}")]
    Acp(String),
}

#[derive(Debug, Clone, Serialize)]
struct RequestBody<'a> {
    model: &'a str,
    messages: &'a [ChatMessage],
    stream: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    temperature: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    max_tokens: Option<u32>,
}

#[derive(Debug, Deserialize)]
struct StreamChunk {
    choices: Vec<Choice>,
}

#[derive(Debug, Deserialize)]
struct Choice {
    delta: Delta,
    #[serde(default)]
    finish_reason: Option<String>,
}

#[derive(Debug, Deserialize)]
struct Delta {
    #[serde(default)]
    content: Option<String>,
}

#[derive(Debug, Clone)]
pub enum StreamEvent {
    Token(String),
    Done,
}

pub type TokenStream = Pin<Box<dyn Stream<Item = Result<StreamEvent, CloudError>> + Send>>;

#[derive(Clone)]
pub struct OpenRouterClient {
    http: reqwest::Client,
    api_key: String,
    app_referer: String,
    app_title: String,
}

impl OpenRouterClient {
    pub fn new(api_key: impl Into<String>) -> Result<Self, CloudError> {
        let api_key = api_key.into();
        if api_key.trim().is_empty() {
            return Err(CloudError::MissingApiKey);
        }
        let http = reqwest::Client::builder()
            .user_agent(concat!("komorebi/", env!("CARGO_PKG_VERSION")))
            .build()?;
        Ok(Self {
            http,
            api_key,
            app_referer: "https://komorebi.svitix.com".into(),
            app_title: "Komorebi".into(),
        })
    }

    pub async fn stream_chat(
        &self,
        model: &str,
        messages: &[ChatMessage],
    ) -> Result<TokenStream, CloudError> {
        let body = RequestBody {
            model,
            messages,
            stream: true,
            temperature: Some(0.7),
            max_tokens: Some(1024),
        };

        let resp = self
            .http
            .post(OPENROUTER_URL)
            .bearer_auth(&self.api_key)
            .header("HTTP-Referer", &self.app_referer)
            .header("X-Title", &self.app_title)
            .json(&body)
            .send()
            .await?;

        let status = resp.status();
        if !status.is_success() {
            let body = resp.text().await.unwrap_or_default();
            return Err(CloudError::Api {
                status: status.as_u16(),
                body,
            });
        }

        // Parse Server-Sent Events: lines starting with "data: " carry JSON payloads,
        // terminated by "data: [DONE]".
        let byte_stream = resp.bytes_stream();
        let event_stream = sse_to_events(byte_stream);
        Ok(Box::pin(event_stream))
    }

    /// Non-streaming completion. Used for proactive-agent decisions and
    /// other one-shot prompts where SSE is overkill.
    pub async fn complete(
        &self,
        model: &str,
        messages: &[ChatMessage],
        max_tokens: u32,
    ) -> Result<String, CloudError> {
        let body = RequestBody {
            model,
            messages,
            stream: false,
            temperature: Some(0.4),
            max_tokens: Some(max_tokens),
        };
        let resp = self
            .http
            .post(OPENROUTER_URL)
            .bearer_auth(&self.api_key)
            .header("HTTP-Referer", &self.app_referer)
            .header("X-Title", &self.app_title)
            .json(&body)
            .send()
            .await?;
        let status = resp.status();
        if !status.is_success() {
            let body = resp.text().await.unwrap_or_default();
            return Err(CloudError::Api {
                status: status.as_u16(),
                body,
            });
        }
        #[derive(serde::Deserialize)]
        struct Resp {
            choices: Vec<RespChoice>,
        }
        #[derive(serde::Deserialize)]
        struct RespChoice {
            message: RespMessage,
        }
        #[derive(serde::Deserialize)]
        struct RespMessage {
            #[serde(default)]
            content: String,
        }
        let parsed: Resp = resp.json().await.map_err(CloudError::Http)?;
        Ok(parsed
            .choices
            .into_iter()
            .next()
            .map(|c| c.message.content)
            .unwrap_or_default())
    }

    /// Vision completion: pass a system + user-text + PNG image. The image
    /// is encoded as a base64 data URI inside an `image_url` content part,
    /// which is the format every vision-capable OpenRouter model accepts.
    pub async fn complete_vision(
        &self,
        model: &str,
        system: &str,
        user_text: &str,
        png_bytes: &[u8],
        max_tokens: u32,
    ) -> Result<String, CloudError> {
        use base64::Engine as _;
        let b64 = base64::engine::general_purpose::STANDARD.encode(png_bytes);
        let data_uri = format!("data:image/png;base64,{b64}");
        let body = serde_json::json!({
            "model": model,
            "stream": false,
            "temperature": 0.5,
            "max_tokens": max_tokens,
            "messages": [
                {"role": "system", "content": system},
                {"role": "user", "content": [
                    {"type": "text", "text": user_text},
                    {"type": "image_url", "image_url": {"url": data_uri}}
                ]}
            ]
        });
        let resp = self
            .http
            .post(OPENROUTER_URL)
            .bearer_auth(&self.api_key)
            .header("HTTP-Referer", &self.app_referer)
            .header("X-Title", &self.app_title)
            .json(&body)
            .send()
            .await?;
        let status = resp.status();
        if !status.is_success() {
            let body = resp.text().await.unwrap_or_default();
            return Err(CloudError::Api {
                status: status.as_u16(),
                body,
            });
        }
        #[derive(serde::Deserialize)]
        struct VResp {
            choices: Vec<VRespChoice>,
        }
        #[derive(serde::Deserialize)]
        struct VRespChoice {
            message: VRespMessage,
        }
        #[derive(serde::Deserialize)]
        struct VRespMessage {
            #[serde(default)]
            content: String,
        }
        let parsed: VResp = resp.json().await.map_err(CloudError::Http)?;
        Ok(parsed
            .choices
            .into_iter()
            .next()
            .map(|c| c.message.content)
            .unwrap_or_default())
    }

    /// Non-streaming text completion. Sibling of [`complete_vision`]
    /// without the image part. Handy for short, one-shot calls where
    /// streaming machinery is overkill (e.g. the game-coach text path,
    /// telemetry classifiers). Uses moderate temperature (0.5) suitable
    /// for friendly conversational replies; callers needing
    /// determinism should use [`stream_chat`] with their own prompt
    /// engineering, or wait for a future explicit-options overload.
    pub async fn complete_text(
        &self,
        model: &str,
        system: &str,
        user_text: &str,
        max_tokens: u32,
    ) -> Result<String, CloudError> {
        let body = serde_json::json!({
            "model": model,
            "stream": false,
            "temperature": 0.5,
            "max_tokens": max_tokens,
            "messages": [
                {"role": "system", "content": system},
                {"role": "user", "content": user_text},
            ]
        });
        let resp = self
            .http
            .post(OPENROUTER_URL)
            .bearer_auth(&self.api_key)
            .header("HTTP-Referer", &self.app_referer)
            .header("X-Title", &self.app_title)
            .json(&body)
            .send()
            .await?;
        let status = resp.status();
        if !status.is_success() {
            let body = resp.text().await.unwrap_or_default();
            return Err(CloudError::Api {
                status: status.as_u16(),
                body,
            });
        }
        #[derive(serde::Deserialize)]
        struct TResp {
            choices: Vec<TRespChoice>,
        }
        #[derive(serde::Deserialize)]
        struct TRespChoice {
            message: TRespMessage,
        }
        #[derive(serde::Deserialize)]
        struct TRespMessage {
            #[serde(default)]
            content: String,
        }
        let parsed: TResp = resp.json().await.map_err(CloudError::Http)?;
        Ok(parsed
            .choices
            .into_iter()
            .next()
            .map(|c| c.message.content)
            .unwrap_or_default())
    }
}

// --- Intent classification -------------------------------------------------

pub const DEFAULT_CLASSIFIER_MODEL: &str = "meta-llama/llama-3.2-3b-instruct";

/// Uses a small OpenRouter model to decide between `Local` and `Cloud`
/// when the keyword router is inconclusive. Skill detection stays in the
/// keyword layer — this classifier is only asked to judge task complexity.
pub struct CloudIntentClassifier {
    http: reqwest::Client,
    api_key: String,
    model: String,
    timeout: std::time::Duration,
}

impl CloudIntentClassifier {
    pub fn new(api_key: impl Into<String>, model: impl Into<String>) -> Result<Self, CloudError> {
        let api_key = api_key.into();
        if api_key.trim().is_empty() {
            return Err(CloudError::MissingApiKey);
        }
        let http = reqwest::Client::builder()
            .user_agent(concat!("komorebi/", env!("CARGO_PKG_VERSION")))
            .build()?;
        Ok(Self {
            http,
            api_key,
            model: model.into(),
            timeout: std::time::Duration::from_secs(3),
        })
    }

    pub fn with_timeout(mut self, timeout: std::time::Duration) -> Self {
        self.timeout = timeout;
        self
    }

    async fn call(&self, query: &str) -> Result<String, CloudError> {
        let system = ChatMessage::system(CLASSIFIER_SYSTEM_PROMPT);
        let user = ChatMessage::user(query.to_string());
        let body = RequestBody {
            model: &self.model,
            messages: &[system, user],
            stream: false,
            temperature: Some(0.0),
            max_tokens: Some(4),
        };
        let fut = self
            .http
            .post(OPENROUTER_URL)
            .bearer_auth(&self.api_key)
            .header("HTTP-Referer", "https://komorebi.svitix.com")
            .header("X-Title", "Komorebi")
            .json(&body)
            .send();
        let resp = tokio::time::timeout(self.timeout, fut)
            .await
            .map_err(|_| CloudError::Stream("classifier timeout".into()))??;
        let status = resp.status();
        if !status.is_success() {
            let body = resp.text().await.unwrap_or_default();
            return Err(CloudError::Api {
                status: status.as_u16(),
                body,
            });
        }
        #[derive(serde::Deserialize)]
        struct Resp {
            choices: Vec<RespChoice>,
        }
        #[derive(serde::Deserialize)]
        struct RespChoice {
            message: RespMessage,
        }
        #[derive(serde::Deserialize)]
        struct RespMessage {
            #[serde(default)]
            content: String,
        }
        let parsed: Resp = resp.json().await.map_err(CloudError::Http)?;
        Ok(parsed
            .choices
            .into_iter()
            .next()
            .map(|c| c.message.content)
            .unwrap_or_default())
    }
}

const CLASSIFIER_SYSTEM_PROMPT: &str = "You classify user queries for an on-device \
assistant. Output exactly one token: LOCAL or CLOUD.\n\n\
LOCAL = small talk, greetings, short factual questions, simple requests a 3B \
model can handle.\n\
CLOUD = coding help, translations, long analysis, nuanced reasoning, essays, \
anything that benefits from a large model.\n\n\
Respond with only the single word LOCAL or CLOUD. No punctuation, no explanation.";

fn parse_classifier_reply(raw: &str) -> Option<komorebi_router::Route> {
    let s = raw.trim().to_ascii_uppercase();
    // The model sometimes wraps the answer in quotes or adds a period.
    let s = s
        .trim_start_matches(['"', '\'', '`'])
        .trim_end_matches(['"', '\'', '`', '.', ',']);
    if s.starts_with("LOCAL") {
        Some(komorebi_router::Route::Local)
    } else if s.starts_with("CLOUD") {
        Some(komorebi_router::Route::Cloud)
    } else {
        None
    }
}

#[async_trait::async_trait]
impl komorebi_router::IntentClassifier for CloudIntentClassifier {
    async fn decide(&self, query: &str) -> Option<komorebi_router::Route> {
        match self.call(query).await {
            Ok(reply) => {
                let parsed = parse_classifier_reply(&reply);
                if parsed.is_none() {
                    tracing::debug!(raw = %reply, "classifier returned unparseable reply");
                }
                parsed
            }
            Err(e) => {
                tracing::debug!(?e, "classifier call failed, falling back to keywords");
                None
            }
        }
    }
}

// --- Skill classification --------------------------------------------------

/// LLM-driven skill picker. Replaces brittle keyword regexes — given any
/// natural-language phrasing, asks a small fast model which of our
/// built-in skills (if any) the user wants to invoke.
///
/// Returned name must match one of the skill names registered in
/// `komorebi_skills::SkillRegistry`. If the model says "none" or
/// returns garbage, we fall back to the keyword router.
pub struct CloudSkillClassifier {
    http: reqwest::Client,
    api_key: String,
    model: String,
    timeout: std::time::Duration,
    skills_doc: String,
}

#[derive(Debug, Clone)]
pub struct SkillIntent {
    pub skill: String,
    /// Free-form rephrased command for the skill to parse, e.g.
    /// "set volume to 50" → `volume` skill consumes "set volume to 50".
    pub command: String,
}

impl CloudSkillClassifier {
    /// `skills` is a list of `(name, one-line description)` pairs the
    /// classifier can choose from. The classifier is told it can also
    /// answer "none" if the query is regular conversation.
    pub fn new(
        api_key: impl Into<String>,
        model: impl Into<String>,
        skills: &[(&str, &str)],
    ) -> Result<Self, CloudError> {
        let api_key = api_key.into();
        if api_key.trim().is_empty() {
            return Err(CloudError::MissingApiKey);
        }
        let http = reqwest::Client::builder()
            .user_agent(concat!("komorebi/", env!("CARGO_PKG_VERSION")))
            .build()?;
        let mut doc = String::from("Available skills:\n");
        for (name, desc) in skills {
            doc.push_str(&format!("- {name}: {desc}\n"));
        }
        Ok(Self {
            http,
            api_key,
            model: model.into(),
            timeout: std::time::Duration::from_secs(3),
            skills_doc: doc,
        })
    }

    pub async fn pick(&self, query: &str) -> Result<Option<SkillIntent>, CloudError> {
        let system = ChatMessage::system(build_skill_system_prompt(&self.skills_doc));
        let user = ChatMessage::user(query.to_string());
        let body = RequestBody {
            model: &self.model,
            messages: &[system, user],
            stream: false,
            temperature: Some(0.0),
            max_tokens: Some(64),
        };
        let fut = self
            .http
            .post(OPENROUTER_URL)
            .bearer_auth(&self.api_key)
            .header("HTTP-Referer", "https://komorebi.svitix.com")
            .header("X-Title", "Komorebi")
            .json(&body)
            .send();
        let resp = tokio::time::timeout(self.timeout, fut)
            .await
            .map_err(|_| CloudError::Stream("skill-classifier timeout".into()))??;
        let status = resp.status();
        if !status.is_success() {
            let body = resp.text().await.unwrap_or_default();
            return Err(CloudError::Api {
                status: status.as_u16(),
                body,
            });
        }
        #[derive(serde::Deserialize)]
        struct Resp {
            choices: Vec<RespChoice>,
        }
        #[derive(serde::Deserialize)]
        struct RespChoice {
            message: RespMessage,
        }
        #[derive(serde::Deserialize)]
        struct RespMessage {
            #[serde(default)]
            content: String,
        }
        let parsed: Resp = resp.json().await.map_err(CloudError::Http)?;
        let raw = parsed
            .choices
            .into_iter()
            .next()
            .map(|c| c.message.content)
            .unwrap_or_default();
        Ok(parse_skill_reply(&raw))
    }
}

fn parse_skill_reply(raw: &str) -> Option<SkillIntent> {
    parse_skill_json(raw)
}

/// Build the system prompt the skill classifier sends to the LLM.
///
/// `skills_doc` is the rendered "Available skills:\n- name: desc\n..."
/// block. Exposed `pub` so non-cloud classifiers (e.g. the local-LLM
/// fallback) can reuse the exact same contract — the parser on the
/// other end is shared too (see [`parse_skill_json`]).
pub fn build_skill_system_prompt(skills_doc: &str) -> String {
    format!(
        "You are an intent classifier for a desktop assistant. Pick the \
single skill that best matches the user's request, or answer \"none\".\n\n\
{skills_doc}\n\
Respond with strict JSON: {{\"skill\":\"NAME_OR_none\",\"command\":\"REPHRASED_INSTRUCTION\"}}.\n\
The command field, if skill != none, should be a short imperative the skill \
can parse (e.g. \"set volume to 50\", \"take a screenshot\", \"open https://github.com\"). \
For \"none\", set command to an empty string.\n\
Output ONLY the JSON, no markdown, no commentary."
    )
}

/// Parse a classifier reply into a [`SkillIntent`]. Tolerates leading/
/// trailing prose and ```json fences (which small local models like to
/// emit). Shared between [`CloudSkillClassifier`] and the local-LLM
/// classifier so the JSON contract stays in one place.
pub fn parse_skill_json(raw: &str) -> Option<SkillIntent> {
    let trimmed = raw.trim();
    // Strip optional ```json ... ``` fences.
    let unfenced = trimmed
        .strip_prefix("```json")
        .or_else(|| trimmed.strip_prefix("```"))
        .map(|s| s.trim_end_matches("```").trim())
        .unwrap_or(trimmed);
    // If the model added prose around the JSON, locate the first
    // balanced `{...}` chunk and try that.
    let candidate = first_json_object(unfenced).unwrap_or(unfenced);
    #[derive(serde::Deserialize)]
    struct ReplyJson {
        skill: String,
        #[serde(default)]
        command: String,
    }
    let v: ReplyJson = serde_json::from_str(candidate).ok()?;
    let skill = v.skill.trim().to_lowercase();
    if skill.is_empty() || skill == "none" || skill == "null" {
        return None;
    }
    Some(SkillIntent {
        skill,
        command: if v.command.trim().is_empty() {
            String::new()
        } else {
            v.command
        },
    })
}

/// Return the first balanced `{...}` substring, or `None` if no
/// matching pair exists. Naive but adequate for short classifier
/// responses (max ~64 tokens).
fn first_json_object(s: &str) -> Option<&str> {
    let bytes = s.as_bytes();
    let start = bytes.iter().position(|&b| b == b'{')?;
    let mut depth = 0i32;
    let mut in_str = false;
    let mut escape = false;
    for (i, &b) in bytes.iter().enumerate().skip(start) {
        if in_str {
            if escape {
                escape = false;
            } else if b == b'\\' {
                escape = true;
            } else if b == b'"' {
                in_str = false;
            }
            continue;
        }
        match b {
            b'"' => in_str = true,
            b'{' => depth += 1,
            b'}' => {
                depth -= 1;
                if depth == 0 {
                    return Some(&s[start..=i]);
                }
            }
            _ => {}
        }
    }
    None
}

fn sse_to_events<S>(bytes: S) -> impl Stream<Item = Result<StreamEvent, CloudError>> + Send
where
    S: Stream<Item = Result<bytes::Bytes, reqwest::Error>> + Send + 'static,
{
    async_stream::stream! {
        let mut buf = String::new();
        let mut stream = Box::pin(bytes);
        while let Some(chunk) = stream.next().await {
            let chunk = match chunk {
                Ok(b) => b,
                Err(e) => { yield Err(CloudError::Http(e)); return; }
            };
            buf.push_str(&String::from_utf8_lossy(&chunk));

            // SSE events are separated by blank lines, but OpenRouter streams one
            // JSON object per "data: " line, so we can split on newlines directly.
            while let Some(nl) = buf.find('\n') {
                let line = buf[..nl].trim_end_matches('\r').to_string();
                buf.drain(..=nl);
                let Some(payload) = line.strip_prefix("data: ").or_else(|| line.strip_prefix("data:")) else {
                    continue;
                };
                let payload = payload.trim();
                if payload.is_empty() { continue; }
                if payload == "[DONE]" {
                    yield Ok(StreamEvent::Done);
                    return;
                }
                match serde_json::from_str::<StreamChunk>(payload) {
                    Ok(chunk) => {
                        for ch in chunk.choices {
                            if let Some(t) = ch.delta.content {
                                if !t.is_empty() {
                                    yield Ok(StreamEvent::Token(t));
                                }
                            }
                            if ch.finish_reason.is_some() {
                                yield Ok(StreamEvent::Done);
                                return;
                            }
                        }
                    }
                    Err(e) => {
                        tracing::debug!(?e, payload = %payload, "skip malformed sse chunk");
                    }
                }
            }
        }
        yield Ok(StreamEvent::Done);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use futures::stream;

    fn bytes_stream(chunks: &[&str]) -> impl Stream<Item = Result<bytes::Bytes, reqwest::Error>> {
        let items: Vec<_> = chunks
            .iter()
            .map(|s| Ok(bytes::Bytes::from(s.to_string())))
            .collect();
        stream::iter(items)
    }

    #[tokio::test]
    async fn parses_tokens_and_done() {
        let raw = [
            "data: {\"choices\":[{\"delta\":{\"content\":\"Hel\"}}]}\n",
            "data: {\"choices\":[{\"delta\":{\"content\":\"lo\"}}]}\n",
            "data: [DONE]\n",
        ];
        let s = sse_to_events(bytes_stream(&raw));
        let out: Vec<_> = Box::pin(s).collect::<Vec<_>>().await;
        let tokens: Vec<String> = out
            .into_iter()
            .filter_map(|r| match r.ok()? {
                StreamEvent::Token(t) => Some(t),
                StreamEvent::Done => None,
            })
            .collect();
        assert_eq!(tokens.join(""), "Hello");
    }

    #[tokio::test]
    async fn skips_malformed_chunks() {
        let raw = [
            "data: not-json\n",
            "data: {\"choices\":[{\"delta\":{\"content\":\"ok\"}}]}\n",
            "data: [DONE]\n",
        ];
        let s = sse_to_events(bytes_stream(&raw));
        let out: Vec<_> = Box::pin(s).collect::<Vec<_>>().await;
        let tokens: Vec<String> = out
            .into_iter()
            .filter_map(|r| match r.ok()? {
                StreamEvent::Token(t) => Some(t),
                _ => None,
            })
            .collect();
        assert_eq!(tokens, vec!["ok"]);
    }

    #[test]
    fn classifier_reply_parses_variants() {
        use komorebi_router::Route;
        assert_eq!(parse_classifier_reply("LOCAL"), Some(Route::Local));
        assert_eq!(parse_classifier_reply("  local\n"), Some(Route::Local));
        assert_eq!(parse_classifier_reply("\"CLOUD\"."), Some(Route::Cloud));
        assert_eq!(parse_classifier_reply("cloud,"), Some(Route::Cloud));
        assert_eq!(
            parse_classifier_reply("LOCAL — small talk"),
            Some(Route::Local)
        );
        assert_eq!(parse_classifier_reply("maybe"), None);
        assert_eq!(parse_classifier_reply(""), None);
    }
}
