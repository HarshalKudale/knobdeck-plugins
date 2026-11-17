//! AI Actions setup: check the provider answers with this key and model, then save them.

#[allow(dead_code)]
#[path = "../llm.rs"]
mod llm;

use llm::{Config, Provider};

fn main() {
    knobdeck_sdk::run_setup(|ctx| {
        let provider = Provider::parse(&ctx.input_str("provider"));
        let key = ctx.input_str("key").trim().to_string();
        let url = match ctx.input_str("url").trim().trim_end_matches('/') {
            u if u.is_empty() || provider == Provider::Anthropic => provider.default_url().to_string(),
            u if u.contains("://") => u.to_string(),
            u => format!("http://{u}"),
        };
        if provider == Provider::Anthropic && key.is_empty() {
            return ctx.fail("Paste your Anthropic API key (console.anthropic.com ▸ API keys)");
        }
        if provider == Provider::OpenAi && key.is_empty() && url == provider.default_url() {
            return ctx.fail("Paste your OpenAI API key, or enter the address of your own OpenAI-compatible server");
        }
        let mut cfg = Config { provider, url, model: String::new(), key: if provider == Provider::Ollama { String::new() } else { key } };
        let models = match llm::models(&cfg) {
            Ok(m) => m,
            Err(e) => return ctx.fail(e.message()),
        };
        let wanted = ctx.input_str("model").trim().to_string();
        cfg.model = if !wanted.is_empty() {
            wanted
        } else if provider == Provider::Ollama {
            match models.first() {
                Some(m) => m.clone(),
                None => return ctx.fail("Ollama has no models yet. Run `ollama pull llama3.2` (or any model), then set up again"),
            }
        } else {
            provider.default_model().to_string()
        };
        // Lists can be partial (OpenAI-compatible servers, aliases): only refuse a model when the
        // list is non-empty and clearly lacks it.
        let known = models.is_empty() || models.iter().any(|m| m == &cfg.model || m.split(':').next() == Some(cfg.model.as_str()));
        if !known {
            let some: Vec<&str> = models.iter().take(6).map(String::as_str).collect();
            return ctx.fail(format!("There is no model \"{}\" at {}. Some it has: {}", cfg.model, cfg.url, some.join(", ")));
        }
        ctx.log(format!("[setup] {} at {} with {}", provider.id(), cfg.url, cfg.model));
        knobdeck_sdk::secrets::set(llm::KEY, &cfg.key)?;
        knobdeck_sdk::save_env(&[("AI_PROVIDER", provider.id()), ("AI_URL", &cfg.url), ("AI_MODEL", &cfg.model)])?;
        Ok(())
    })
}
