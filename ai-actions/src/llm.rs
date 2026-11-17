//! The model call: Anthropic's Messages API, an OpenAI-compatible Chat Completions API (OpenAI,
//! LM Studio, OpenRouter, Groq…) or a local Ollama, with the user's own key. Shared by the plugin
//! and its setup program.

use knobdeck_sdk::{json, Error, Result, Value};
use std::time::Duration;

pub const KEY: &str = "AI_API_KEY";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Provider {
    Anthropic,
    OpenAi,
    Ollama,
}

impl Provider {
    pub fn parse(s: &str) -> Provider {
        match s {
            "openai" => Provider::OpenAi,
            "ollama" => Provider::Ollama,
            _ => Provider::Anthropic,
        }
    }
    #[allow(dead_code)] // setup
    pub fn id(self) -> &'static str {
        match self {
            Provider::Anthropic => "anthropic",
            Provider::OpenAi => "openai",
            Provider::Ollama => "ollama",
        }
    }
    pub fn name(self) -> &'static str {
        match self {
            Provider::Anthropic => "Anthropic",
            Provider::OpenAi => "the OpenAI-compatible server",
            Provider::Ollama => "Ollama",
        }
    }
    pub fn default_url(self) -> &'static str {
        match self {
            Provider::Anthropic => "https://api.anthropic.com",
            Provider::OpenAi => "https://api.openai.com/v1",
            Provider::Ollama => "http://localhost:11434",
        }
    }
    /// The model used when setup leaves the field empty (Ollama: the first one installed).
    pub fn default_model(self) -> &'static str {
        match self {
            Provider::Anthropic => "claude-opus-5-5",
            Provider::OpenAi => "gpt-5",
            Provider::Ollama => "",
        }
    }
}

#[derive(Clone, Debug)]
pub struct Config {
    pub provider: Provider,
    pub url: String,
    pub model: String,
    pub key: String,
}

impl Config {
    /// From the saved setup (`AI_PROVIDER`, `AI_URL`, `AI_MODEL` in `.env`; the key in the OS
    /// credential store).
    pub fn from_env() -> Result<Config> {
        let provider = knobdeck_sdk::env_or("AI_PROVIDER", "");
        if provider.is_empty() {
            return Err(Error::msg("Run this plugin's setup to choose a model provider"));
        }
        let provider = Provider::parse(&provider);
        let url = knobdeck_sdk::env_or("AI_URL", provider.default_url());
        let model = knobdeck_sdk::env_or("AI_MODEL", provider.default_model());
        Ok(Config { provider, url: url.trim_end_matches('/').to_string(), model, key: knobdeck_sdk::secrets::env_or_stored(KEY) })
    }
}

fn agent() -> ureq::Agent {
    // Answers can take a while (thinking models, a cold local model).
    ureq::AgentBuilder::new().timeout_connect(Duration::from_secs(10)).timeout(Duration::from_secs(180)).build()
}

/// Turn a failed call into a sentence the user can act on.
fn call_error(cfg: &Config, e: ureq::Error) -> Error {
    match e {
        ureq::Error::Status(code, resp) => {
            let body = resp.into_string().unwrap_or_default();
            let detail = serde_json::from_str::<Value>(&body)
                .ok()
                .and_then(|j| j["error"]["message"].as_str().or(j["error"].as_str()).or(j["message"].as_str()).map(str::to_string))
                .unwrap_or_else(|| body.chars().take(160).collect());
            match code {
                401 | 403 => {
                    Error::msg(format!("{} refused the API key. Check it in this plugin's setup{}", provider_title(cfg), if detail.is_empty() { String::new() } else { format!(" ({detail})") }))
                }
                404 => Error::msg(format!("{} doesn't know the model \"{}\" ({detail}). Pick another in setup", provider_title(cfg), cfg.model)),
                429 => Error::msg(format!("{} is rate limiting you or the account is out of credit: {detail}", provider_title(cfg))),
                _ => Error::msg(format!("{} answered {code}: {detail}", provider_title(cfg))),
            }
        }
        ureq::Error::Transport(t) => {
            knobdeck_sdk::log(format!("{}: {t}", cfg.url));
            match cfg.provider {
                Provider::Ollama => Error::msg(format!("Ollama isn't answering at {}. Start Ollama and try again", cfg.url)),
                _ if t.to_string().contains("timed out") => Error::msg(format!("{} took too long to answer", provider_title(cfg))),
                _ => Error::msg(format!("Couldn't reach {} at {}. Check your connection", provider_title(cfg), cfg.url)),
            }
        }
    }
}

fn provider_title(cfg: &Config) -> String {
    let n = cfg.provider.name();
    let mut c = n.chars();
    c.next().map(|f| f.to_uppercase().collect::<String>() + c.as_str()).unwrap_or_default()
}

const ANTHROPIC_VERSION: &str = "2026-10-21";

/// Anthropic models that take `output_config.effort` (adaptive thinking; older models reject it).
fn anthropic_takes_effort(model: &str) -> bool {
    ["claude-fable-5", "claude-mythos-5", "claude-opus-5", "claude-sonnet-5", "claude-haiku-5", "claude-opus-4-6", "claude-opus-4-7", "claude-opus-4-8", "claude-sonnet-4-6"]
        .iter()
        .any(|p| model.starts_with(p))
}

/// Models that accept server-side refusal fallbacks (`fallbacks: "default"`).
fn anthropic_takes_fallbacks(model: &str) -> bool {
    ["claude-fable-5-2", "claude-fable-5-1", "claude-opus-5-5", "claude-opus-5", "claude-sonnet-5-5"].contains(&model)
}

/// Ask the model; returns its text answer.
pub fn complete(cfg: &Config, system: &str, user: &str) -> Result<String> {
    if cfg.model.trim().is_empty() {
        return Err(Error::msg("No model chosen. Pick one in this plugin's setup"));
    }
    if cfg.provider == Provider::Anthropic && cfg.key.is_empty() {
        return Err(Error::msg("Add your Anthropic API key in this plugin's setup"));
    }
    let a = agent();
    let text = match cfg.provider {
        Provider::Anthropic => {
            let mut body = json!({
                "model": cfg.model,
                "max_tokens": 16000,
                "system": system,
                "messages": [{"role": "user", "content": user}],
            });
            if anthropic_takes_effort(&cfg.model) {
                // Text edits on a key press: answer fast.
                body["output_config"] = json!({"effort": "low"});
            }
            let mut req = a.post(&format!("{}/v1/messages", cfg.url)).set("x-api-key", &cfg.key).set("anthropic-version", ANTHROPIC_VERSION);
            if anthropic_takes_fallbacks(&cfg.model) {
                // A request a safety classifier declines is retried on a suitable model.
                req = req.set("anthropic-beta", "server-side-fallback-2026-07-01");
                body["fallbacks"] = json!("default");
            }
            let v: Value = req.send_json(body).map_err(|e| call_error(cfg, e))?.into_json()?;
            if v["stop_reason"] == "refusal" {
                return Err(Error::msg("The model declined this request"));
            }
            v["content"].as_array().into_iter().flatten().filter(|b| b["type"] == "text").filter_map(|b| b["text"].as_str()).collect::<Vec<_>>().join("")
        }
        Provider::OpenAi => {
            let mut req = a.post(&format!("{}/chat/completions", cfg.url));
            if !cfg.key.is_empty() {
                req = req.set("Authorization", &format!("Bearer {}", cfg.key));
            }
            let body = json!({"model": cfg.model, "messages": [{"role": "system", "content": system}, {"role": "user", "content": user}]});
            let v: Value = req.send_json(body).map_err(|e| call_error(cfg, e))?.into_json()?;
            if v["choices"][0]["finish_reason"] == "content_filter" {
                return Err(Error::msg("The model declined this request"));
            }
            v["choices"][0]["message"]["content"].as_str().unwrap_or_default().to_string()
        }
        Provider::Ollama => {
            let body = json!({"model": cfg.model, "stream": false, "messages": [{"role": "system", "content": system}, {"role": "user", "content": user}]});
            let v: Value = a.post(&format!("{}/api/chat", cfg.url)).send_json(body).map_err(|e| call_error(cfg, e))?.into_json()?;
            v["message"]["content"].as_str().unwrap_or_default().to_string()
        }
    };
    let text = strip_think(&text).trim().to_string();
    if text.is_empty() {
        return Err(Error::msg("The model sent an empty answer"));
    }
    Ok(text)
}

/// Local reasoning models (DeepSeek R1, Qwen 3…) put their reasoning in `<think>…</think>` first.
fn strip_think(text: &str) -> &str {
    let t = text.trim_start();
    if let Some(rest) = t.strip_prefix("<think>") {
        if let Some(end) = rest.find("</think>") {
            return &rest[end + "</think>".len()..];
        }
    }
    text
}

/// The models the provider offers (setup checks the key and the model with this).
#[allow(dead_code)] // setup
pub fn models(cfg: &Config) -> Result<Vec<String>> {
    let a = agent();
    let ids = |v: &Value, list: &str, field: &str| -> Vec<String> { v[list].as_array().into_iter().flatten().filter_map(|m| m[field].as_str().map(str::to_string)).collect() };
    Ok(match cfg.provider {
        Provider::Anthropic => {
            let v: Value = a
                .get(&format!("{}/v1/models?limit=1000", cfg.url))
                .set("x-api-key", &cfg.key)
                .set("anthropic-version", ANTHROPIC_VERSION)
                .call()
                .map_err(|e| call_error(cfg, e))?
                .into_json()?;
            ids(&v, "data", "id")
        }
        Provider::OpenAi => {
            let mut req = a.get(&format!("{}/models", cfg.url));
            if !cfg.key.is_empty() {
                req = req.set("Authorization", &format!("Bearer {}", cfg.key));
            }
            let v: Value = req.call().map_err(|e| call_error(cfg, e))?.into_json()?;
            ids(&v, "data", "id")
        }
        Provider::Ollama => {
            let v: Value = a.get(&format!("{}/api/tags", cfg.url)).call().map_err(|e| call_error(cfg, e))?.into_json()?;
            ids(&v, "models", "name")
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn think_is_stripped() {
        assert_eq!(strip_think("<think>hmm</think>\nHello").trim(), "Hello");
        assert_eq!(strip_think("Hello <think>x</think>"), "Hello <think>x</think>");
    }

    #[test]
    fn effort_only_where_supported() {
        assert!(anthropic_takes_effort("claude-opus-5-5"));
        assert!(anthropic_takes_effort("claude-haiku-5-5"));
        assert!(!anthropic_takes_effort("claude-haiku-4-5"));
        assert!(anthropic_takes_fallbacks("claude-opus-5-5"));
        assert!(!anthropic_takes_fallbacks("claude-haiku-5-5"));
    }
}
