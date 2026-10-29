//! AI assistance: the Anthropic Messages API (default) or any
//! OpenAI-compatible chat-completions endpoint, streamed to the editor.

use futures_util::StreamExt;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use tauri::Emitter;

pub const DEFAULT_MODEL: &str = "claude-opus-5-5";
const ANTHROPIC_URL: &str = "https://api.anthropic.com/v1/messages";
const ANTHROPIC_VERSION: &str = "2026-10-21";
/// Server-side refusal fallback ("default" routing by refusal category).
const FALLBACK_BETA: &str = "server-side-fallback-2026-07-01";

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    /// "anthropic" or "openai"
    pub provider: String,
    pub anthropic_key: String,
    pub anthropic_model: String,
    /// low / medium / high
    pub effort: String,
    pub openai_base_url: String,
    pub openai_key: String,
    pub openai_model: String,
}

impl Default for Settings {
    fn default() -> Self {
        Settings {
            provider: "anthropic".into(),
            anthropic_key: String::new(),
            anthropic_model: DEFAULT_MODEL.into(),
            effort: "medium".into(),
            openai_base_url: "http://localhost:11434/v1".into(),
            openai_key: String::new(),
            openai_model: String::new(),
        }
    }
}

/// What the settings dialog shows (keys are never sent back in full).
#[derive(Serialize)]
pub struct SettingsView {
    pub provider: String,
    pub anthropic_model: String,
    pub effort: String,
    pub openai_base_url: String,
    pub openai_model: String,
    pub anthropic_key_set: bool,
    pub anthropic_key_from_env: bool,
    pub openai_key_set: bool,
}

fn config_path() -> Option<PathBuf> {
    let base = std::env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".config")))?;
    Some(base.join("docex").join("ai.json"))
}

pub fn load() -> Settings {
    config_path()
        .and_then(|p| std::fs::read_to_string(p).ok())
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_default()
}

pub fn save(s: &Settings) -> Result<(), String> {
    let p = config_path().ok_or("no config directory")?;
    std::fs::create_dir_all(p.parent().unwrap()).map_err(|e| e.to_string())?;
    std::fs::write(&p, serde_json::to_string_pretty(s).unwrap()).map_err(|e| e.to_string())?;
    // The file holds API keys: owner-only.
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(&p, std::fs::Permissions::from_mode(0o600));
    }
    Ok(())
}

fn env_key() -> Option<String> {
    std::env::var("ANTHROPIC_API_KEY").ok().filter(|k| !k.trim().is_empty())
}

pub fn view(s: &Settings) -> SettingsView {
    SettingsView {
        provider: s.provider.clone(),
        anthropic_model: s.anthropic_model.clone(),
        effort: s.effort.clone(),
        openai_base_url: s.openai_base_url.clone(),
        openai_model: s.openai_model.clone(),
        anthropic_key_set: !s.anthropic_key.is_empty() || env_key().is_some(),
        anthropic_key_from_env: s.anthropic_key.is_empty() && env_key().is_some(),
        openai_key_set: !s.openai_key.is_empty(),
    }
}

/// Settings update from the dialog: empty key fields keep the stored key.
#[derive(Deserialize)]
pub struct SettingsUpdate {
    pub provider: String,
    pub anthropic_model: String,
    pub effort: String,
    pub openai_base_url: String,
    pub openai_model: String,
    #[serde(default)]
    pub anthropic_key: Option<String>,
    #[serde(default)]
    pub openai_key: Option<String>,
}

pub fn update(u: SettingsUpdate) -> Result<Settings, String> {
    let mut s = load();
    s.provider = u.provider;
    s.anthropic_model = if u.anthropic_model.trim().is_empty() { DEFAULT_MODEL.into() } else { u.anthropic_model.trim().into() };
    s.effort = u.effort;
    s.openai_base_url = u.openai_base_url.trim().trim_end_matches('/').into();
    s.openai_model = u.openai_model.trim().into();
    if let Some(k) = u.anthropic_key.filter(|k| !k.trim().is_empty()) {
        s.anthropic_key = k.trim().into();
    }
    if let Some(k) = u.openai_key.filter(|k| !k.trim().is_empty()) {
        s.openai_key = k.trim().into();
    }
    save(&s)?;
    Ok(s)
}

#[derive(Deserialize)]
pub struct Request {
    /// Identifies the stream in `ai-delta` events.
    pub id: String,
    pub system: String,
    pub prompt: String,
    #[serde(default)]
    pub max_tokens: Option<u32>,
}

#[derive(Serialize, Clone)]
struct Delta<'a> {
    id: &'a str,
    text: &'a str,
}

/// Run one request, emitting `ai-delta` events as text arrives; returns the full text.
pub async fn complete(app: tauri::AppHandle, req: Request) -> Result<String, String> {
    let s = load();
    let client = reqwest::Client::new();
    let max_tokens = req.max_tokens.unwrap_or(16000);
    let resp = if s.provider == "openai" {
        if s.openai_model.is_empty() {
            return Err("Set a model for the OpenAI-compatible endpoint in AI settings.".into());
        }
        let mut r = client.post(format!("{}/chat/completions", s.openai_base_url)).json(&serde_json::json!({
            "model": s.openai_model,
            "stream": true,
            "max_tokens": max_tokens,
            "messages": [
                {"role": "system", "content": req.system},
                {"role": "user", "content": req.prompt},
            ],
        }));
        if !s.openai_key.is_empty() {
            r = r.bearer_auth(&s.openai_key);
        }
        r.send().await
    } else {
        let key = if s.anthropic_key.is_empty() { env_key().unwrap_or_default() } else { s.anthropic_key.clone() };
        if key.is_empty() {
            return Err("No Anthropic API key. Add one in AI settings (Claude Max includes monthly API credits: claim them on claude.ai under Settings → Billing, then create a key at platform.claude.com).".into());
        }
        let effort = if matches!(s.effort.as_str(), "low" | "medium" | "high" | "xhigh" | "max") { s.effort.as_str() } else { "medium" };
        client
            .post(ANTHROPIC_URL)
            .header("x-api-key", key)
            .header("anthropic-version", ANTHROPIC_VERSION)
            .header("anthropic-beta", FALLBACK_BETA)
            .json(&serde_json::json!({
                "model": s.anthropic_model,
                "max_tokens": max_tokens,
                "stream": true,
                "system": req.system,
                "output_config": {"effort": effort},
                "fallbacks": "default",
                "messages": [{"role": "user", "content": req.prompt}],
            }))
            .send()
            .await
    }
    .map_err(|e| format!("Could not reach the AI service: {e}"))?;

    let status = resp.status();
    if !status.is_success() {
        let body = resp.text().await.unwrap_or_default();
        let msg = serde_json::from_str::<serde_json::Value>(&body)
            .ok()
            .and_then(|v| v.pointer("/error/message").and_then(|m| m.as_str()).map(String::from))
            .unwrap_or(body);
        return Err(format!("AI request failed ({status}): {msg}"));
    }

    // Server-sent events: `data: {...}` lines.
    let mut out = String::new();
    let mut buf = String::new();
    let mut stop_reason = String::new();
    let mut stream = resp.bytes_stream();
    while let Some(chunk) = stream.next().await {
        let chunk = chunk.map_err(|e| e.to_string())?;
        buf.push_str(&String::from_utf8_lossy(&chunk));
        while let Some(nl) = buf.find('\n') {
            let line = buf[..nl].trim_end_matches('\r').to_string();
            buf.drain(..=nl);
            let Some(data) = line.strip_prefix("data:").map(str::trim) else { continue };
            if data == "[DONE]" {
                continue;
            }
            let Ok(v) = serde_json::from_str::<serde_json::Value>(data) else { continue };
            let text = if s.provider == "openai" {
                v.pointer("/choices/0/delta/content").and_then(|t| t.as_str()).map(String::from)
            } else {
                match v.get("type").and_then(|t| t.as_str()) {
                    Some("content_block_delta") => {
                        v.pointer("/delta/text").and_then(|t| t.as_str()).filter(|_| v.pointer("/delta/type").and_then(|t| t.as_str()) == Some("text_delta")).map(String::from)
                    }
                    Some("message_delta") => {
                        if let Some(r) = v.pointer("/delta/stop_reason").and_then(|r| r.as_str()) {
                            stop_reason = r.to_string();
                        }
                        None
                    }
                    Some("error") => {
                        let msg = v.pointer("/error/message").and_then(|m| m.as_str()).unwrap_or("stream error");
                        return Err(format!("AI request failed: {msg}"));
                    }
                    _ => None,
                }
            };
            if let Some(t) = text.filter(|t| !t.is_empty()) {
                out.push_str(&t);
                let _ = app.emit("ai-delta", Delta { id: &req.id, text: &t });
            }
        }
    }
    if stop_reason == "refusal" {
        return Err("The model declined this request.".into());
    }
    Ok(out)
}
