//! Main text-generation orchestrator: routing, history, tool loop wiring.

use super::engines::stream_acp;
use super::events::{emit, ChatEventOut};
use super::prompts::{system_prompt, tools_system_prompt};
use super::rag::build_rag_context;
use super::skill_picker::pick_skill_intent;
use super::speak::maybe_speak;
use super::tool_loop::run_with_tools;
use super::ChatService;
use crate::settings;
use komorebi_cloud::CloudIntentClassifier;
use komorebi_router::{classify, classify_async, ChatMessage, Route};
use std::sync::atomic::Ordering;
use std::sync::Arc;
use tauri::{AppHandle, Manager, Wry};

pub(super) async fn run_generation(
    app: AppHandle<Wry>,
    id: String,
    prompt: String,
) -> Result<(), String> {
    let service: Arc<ChatService> = app
        .try_state::<Arc<ChatService>>()
        .ok_or_else(|| "chat service not initialized".to_string())?
        .inner()
        .clone();
    service.cancel.store(false, Ordering::SeqCst);

    // Relationship scoring: every user turn updates the affinity state
    // BEFORE we route. Auto-extract a self-introduction when the user name
    // isn't set yet so the persona can address them by name from message
    // two onwards.
    if settings::get_user_name(&app).is_none() {
        if let Some(name) = crate::relationship::extract_self_introduction(&prompt) {
            if let Err(e) = settings::set_user_name(&app, &name) {
                tracing::warn!(?e, "failed to persist auto-extracted user name");
            } else {
                tracing::info!(?name, "auto-extracted user name");
            }
        }
    }
    let _ = crate::relationship::apply_user_message(&app, &prompt);

    // Weather pre-check. The LLM has no built-in weather tool, so we
    // ALWAYS intercept weather queries — independent of smart routing —
    // and answer them via `komorebi-weather` (Open-Meteo by default,
    // keyless). We still avoid hijacking conversational mentions like
    // "обсудим погоду позже": require either an explicit city in the
    // prompt, an interrogative form, a leading weather noun, or a
    // configured fallback location (default city or use-IP).
    let lower_prompt = prompt.to_lowercase();
    let starts_with_weather_word = [
        "погода",
        "погоду",
        "погоды",
        "температура",
        "температуру",
        "прогноз",
        "weather",
        "temperature",
        "forecast",
    ]
    .iter()
    .any(|w| lower_prompt.trim_start().starts_with(w));
    let has_question_form = prompt.contains('?')
        || lower_prompt.starts_with("какая погод")
        || lower_prompt.starts_with("яка погод")
        || lower_prompt.starts_with("what")
        || lower_prompt.starts_with("how");
    let weather_cfg = settings::weather_config(&app);
    let has_weather_fallback = weather_cfg.default_city.is_some() || weather_cfg.use_ip;

    // Optional embedding-based detector. Returns true when the local
    // intent model is loaded AND the prompt embeds close to a weather
    // anchor phrase. Lets free-form rephrases the keyword list misses
    // (e.g. "сколько градусов на улице?", "is it raining tomorrow?")
    // still route to the weather skill. When the model isn't loaded
    // this is a cheap `false` and we fall through to keyword logic.
    let intent_says_weather = matches!(
        crate::intent::detect_intent(&app, &prompt).await,
        Some(m) if m.intent == komorebi_intent::Intent::Weather
    );

    if (komorebi_weather::is_weather_query(&prompt)
        && (komorebi_weather::extract_city_from_text(&prompt).is_some()
            || starts_with_weather_word
            || has_question_form
            || has_weather_fallback))
        || intent_says_weather
    {
        emit(
            &app,
            ChatEventOut::Started {
                id: id.clone(),
                route: "skill".into(),
            },
        );
        if let Some(reply) = crate::weather::maybe_handle(&app, &prompt).await {
            // Wrap with a default mood tag so the avatar reacts.
            let tagged = format!("<mood:happy>{reply}");
            emit(
                &app,
                ChatEventOut::Token {
                    id: id.clone(),
                    text: tagged.clone(),
                },
            );
            {
                let mut hist = service.history.lock().await;
                hist.push(ChatMessage::user(prompt.clone()));
                hist.push(ChatMessage::assistant(tagged.clone()));
            }
            emit(
                &app,
                ChatEventOut::Done {
                    id,
                    full_text: tagged.clone(),
                },
            );
            maybe_speak(&app, tagged).await;
            return Ok(());
        }
    }

    let mode = settings::get_mode(&app);
    let smart = settings::get_smart_routing(&app);
    let acp = settings::get_acp_agent_command(&app);
    tracing::info!(
        prompt_len = prompt.chars().count(),
        prompt_preview = %prompt.chars().take(80).collect::<String>(),
        ?mode,
        smart_routing = smart,
        engine = if acp.is_some() { "acp" } else { "komorebi" },
        "chat: run_generation start"
    );

    // Smart-skill pre-pass: ask the best classifier we have access to
    // whether this prompt is a skill invocation. Cloud (when keyed) is
    // strictly more accurate; local LLM is the offline fallback; keyword
    // router catches simple phrasings either model misses.
    //
    // A skill answer is computed without the chat model, so this pass sits
    // above the engine choice and answers the same way under both engines.
    if smart {
        if let Some(intent) = pick_skill_intent(&app, &service, &prompt).await {
            tracing::info!(skill = %intent.skill, "dispatching skill");
            let cmd = if intent.command.trim().is_empty() {
                prompt.clone()
            } else {
                intent.command
            };
            emit(
                &app,
                ChatEventOut::Started {
                    id: id.clone(),
                    route: "skill".into(),
                },
            );
            let reply = match service.skills.dispatch_named(&intent.skill, &cmd).await {
                Ok(r) => r.text,
                Err(komorebi_skills::SkillError::NotApplicable) => {
                    // The named skill couldn't parse the rephrased command;
                    // fall back to the registry's own keyword dispatch.
                    match service.skills.dispatch(&prompt).await {
                        Ok(r) => r.text,
                        Err(_) => "Skill couldn't run that. Try rephrasing.".into(),
                    }
                }
                Err(komorebi_skills::SkillError::Exec(m)) => {
                    format!("Skill failed: {m}")
                }
            };
            emit(
                &app,
                ChatEventOut::Token {
                    id: id.clone(),
                    text: reply.clone(),
                },
            );
            emit(
                &app,
                ChatEventOut::Done {
                    id,
                    full_text: reply.clone(),
                },
            );
            maybe_speak(&app, reply).await;
            return Ok(());
        }
    }

    // Which engine owns the turn. With smart routing off a skill can still
    // answer, because the keyword router detects one; that detection is also
    // the only routing an ACP turn gets, since the local/cloud split picks a
    // provider *inside* the `komorebi` engine.
    let is_acp = acp.is_some();
    let turn = if let Some(command) = acp {
        decide_turn(classify(&prompt, mode), Some(command))
    } else {
        // Local/Cloud routing decision. Cloud classifier (when keyed) is
        // more accurate for the "should this be heavy or light?" call;
        // otherwise we use the deterministic keyword router that respects
        // the user's mode preference.
        let route = if smart {
            match settings::get_openrouter_key(&app) {
                Some(key) => {
                    let model = settings::get_classifier_model(&app);
                    match CloudIntentClassifier::new(key, model) {
                        Ok(c) => classify_async(&prompt, mode, Some(&c)).await,
                        Err(e) => {
                            tracing::debug!(?e, "classifier init failed; using keyword router");
                            classify(&prompt, mode)
                        }
                    }
                }
                None => classify(&prompt, mode),
            }
        } else {
            classify(&prompt, mode)
        };
        decide_turn(route, None)
    };
    tracing::info!(?turn, "chat: route decided");
    emit(
        &app,
        ChatEventOut::Started {
            id: id.clone(),
            route: match &turn {
                Turn::Skill { .. } => "skill",
                Turn::Komorebi(route) => route_label(*route),
                Turn::Acp(_) => "acp",
            }
            .into(),
        },
    );

    {
        let mut hist = service.history.lock().await;
        hist.push(ChatMessage::user(prompt.clone()));
    }
    let messages = build_messages(&app, &service, &prompt, !is_acp).await;

    // Tool-call loop: each iteration streams a reply; if it contains a
    // <tool_call>, execute it, append the result as a system message,
    // and run another iteration. Skills bypass this entirely — *unless*
    // the registry says NotApplicable, in which case the turn goes back to
    // the engine (the keyword router has wider recall than the skills' own
    // parsers, so phrases like "почему у меня нет звука" can land on the
    // volume skill but not parse).
    let full_text = match turn {
        Turn::Komorebi(route) => run_with_tools(&app, &service, &id, route, messages).await?,
        Turn::Acp(command) => run_agent(&app, &service, &id, &command, messages).await?,
        Turn::Skill { acp } => match answer_with_skill(&app, &service, &id, &prompt).await {
            Some(reply) => reply,
            None => match acp {
                Some(command) => {
                    emit(
                        &app,
                        ChatEventOut::Started {
                            id: id.clone(),
                            route: "acp".into(),
                        },
                    );
                    run_agent(&app, &service, &id, &command, messages).await?
                }
                None => {
                    tracing::info!("skill not applicable, falling back to the LLM");
                    let route = match mode {
                        komorebi_router::Mode::Local => Route::Local,
                        _ => Route::Cloud,
                    };
                    emit(
                        &app,
                        ChatEventOut::Started {
                            id: id.clone(),
                            route: route_label(route).into(),
                        },
                    );
                    run_with_tools(&app, &service, &id, route, messages).await?
                }
            },
        },
    };

    {
        let mut hist = service.history.lock().await;
        hist.push(ChatMessage::assistant(full_text.clone()));
    }
    emit(
        &app,
        ChatEventOut::Done {
            id,
            full_text: full_text.clone(),
        },
    );
    maybe_speak(&app, full_text).await;
    Ok(())
}

/// The model's view of the turn: static/context system messages, then the
/// conversation. `tools` adds Komorebi's `<tool_call>` protocol — only the
/// `komorebi` engine can execute it, so ACP turns pass `false`.
async fn build_messages(
    app: &AppHandle<Wry>,
    service: &ChatService,
    prompt: &str,
    tools: bool,
) -> Vec<ChatMessage> {
    let hist = service.history.lock().await;
    let mut m = Vec::with_capacity(hist.len() + 4);
    let lang = crate::settings::resolve_language(app);
    m.push(system_prompt(lang));
    // Relationship persona/context — drives tone, pet-names, and how
    // closely the assistant treats the user. Always present.
    m.push(ChatMessage::system(
        crate::relationship::system_prompt_addition(app),
    ));
    // Always include a fresh machine/time context so the LLM can answer
    // simple environment questions ("what time is it", "how much RAM do I
    // have") without a dedicated skill.
    m.push(ChatMessage::system(crate::sysctx::render_context_message()));
    // Tool-use protocol: only for the engine that can execute it.
    if tools && settings::get_chat_tool_calls_enabled(app) {
        m.push(tools_system_prompt(
            settings::get_desktop_automation_enabled(app),
        ));
    }
    // RAG: retrieve top-k chunks for the current user prompt and prepend
    // them as an additional system message.
    if settings::get_rag_enabled(app) {
        if let Some(ctx) = build_rag_context(app, prompt) {
            m.push(ChatMessage::system(ctx));
        }
    }
    m.extend(hist.iter().cloned());
    m
}

/// Who answers the turn. A skill answer needs no model, so it is decided
/// above the engine; the local/cloud split only exists inside Komorebi's own
/// engine, and an agent turn is a route of its own.
#[derive(Debug, PartialEq, Eq)]
enum Turn {
    /// A deterministic skill answers. `acp` remembers the agent that takes
    /// the turn when the registry cannot parse the prompt.
    Skill { acp: Option<String> },
    /// Komorebi's own loop, streaming through this route (`local` or `cloud`).
    Komorebi(Route),
    /// An ACP agent owns the turn.
    Acp(String),
}

/// Turn the router's verdict into a turn. Under an agent, `local` and `cloud`
/// carry no meaning — they choose a provider inside Komorebi's engine — so the
/// agent takes them; a skill survives because it answers without a model.
fn decide_turn(route: Route, acp: Option<String>) -> Turn {
    match (route, acp) {
        (Route::Skill, acp) => Turn::Skill { acp },
        (_, Some(command)) => Turn::Acp(command),
        (route, None) => Turn::Komorebi(route),
    }
}

fn route_label(route: Route) -> &'static str {
    match route {
        Route::Local => "local",
        Route::Cloud => "cloud",
        Route::Skill => "skill",
    }
}

/// Answer `prompt` with a registry skill. `None` means the registry could not
/// parse it, so the engine takes the turn instead.
async fn answer_with_skill(
    app: &AppHandle<Wry>,
    service: &ChatService,
    id: &str,
    prompt: &str,
) -> Option<String> {
    let reply = match service.skills.dispatch(prompt).await {
        Ok(resp) => resp.text,
        Err(komorebi_skills::SkillError::NotApplicable) => return None,
        Err(komorebi_skills::SkillError::Exec(msg)) => format!("Skill failed: {msg}"),
    };
    emit(
        app,
        ChatEventOut::Token {
            id: id.into(),
            text: reply.clone(),
        },
    );
    Some(reply)
}

/// Stream one ACP turn. The agent owns routing, tools and context; Komorebi
/// only supplies the conversation. The caller announces `route: "acp"`.
async fn run_agent(
    app: &AppHandle<Wry>,
    service: &ChatService,
    id: &str,
    command: &str,
    messages: Vec<ChatMessage>,
) -> Result<String, String> {
    tracing::info!(command = %command, "chat: acp turn");
    stream_acp(app, service, id, &messages, command).await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_agent_only_takes_turns_no_skill_answers() {
        let agent = || Some("opencode acp".to_string());
        // local/cloud choose a provider inside Komorebi's engine, so under an
        // agent they are not routes at all: the agent owns the turn.
        assert!(matches!(decide_turn(Route::Local, agent()), Turn::Acp(_)));
        assert!(matches!(decide_turn(Route::Cloud, agent()), Turn::Acp(_)));
        // A skill answers without a model, so it answers under either engine.
        assert!(matches!(
            decide_turn(Route::Skill, agent()),
            Turn::Skill { acp: Some(_) }
        ));
        assert!(matches!(
            decide_turn(Route::Skill, None),
            Turn::Skill { acp: None }
        ));
        // Without an agent, both providers are Komorebi's.
        assert_eq!(
            decide_turn(Route::Local, None),
            Turn::Komorebi(Route::Local)
        );
        assert_eq!(
            decide_turn(Route::Cloud, None),
            Turn::Komorebi(Route::Cloud)
        );
    }
}
