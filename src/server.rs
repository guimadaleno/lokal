//! Finding the model server and the model it has loaded.

use std::net::{TcpStream, ToSocketAddrs};
use std::time::Duration;

use serde_json::{Value, json};

use crate::config::Config;
use crate::{http, system, text};

/// LM Studio's default port.
const PORT: u16 = 1234;

/// Which server it is: what Lokal can ask it beyond the chat API.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Kind {
    LmStudio,
    Ollama,
    /// Any other server that speaks the OpenAI chat API.
    Other,
}

pub struct Server {
    /// host:port
    pub address: String,
    pub model: String,
    /// The model's context length in tokens, when the server reports it.
    pub context: Option<u64>,
    /// The most context the model takes, when the server reports it.
    pub max_context: Option<u64>,
    /// Whether the model takes images; None when the server doesn't say.
    pub vision: Option<bool>,
    pub kind: Kind,
    /// Set in the settings, rather than found.
    pub pinned: bool,
}

/// The model picked, with what the server says about it.
#[derive(Debug, PartialEq)]
struct Model {
    id: String,
    context: Option<u64>,
    max_context: Option<u64>,
    vision: Option<bool>,
    kind: Kind,
}

pub fn find(config: &Config) -> Result<Server, String> {
    let address = match &config.server {
        Some(server) => {
            let address = address_from_setting(server)?;
            // A quick check first, so a server that's down is reported at once.
            if !is_listening(&address) {
                return Err(text::cant_reach(&address));
            }
            address
        }
        None => look_for_server()?,
    };
    let model = pick_model(&address, config.model.as_deref())?;
    Ok(Server {
        address,
        model: model.id,
        context: model.context,
        max_context: model.max_context,
        vision: model.vision,
        kind: model.kind,
        pinned: config.server.is_some(),
    })
}

/// Accepts `host`, `host:port` or a URL like http://host:port/v1.
fn address_from_setting(server: &str) -> Result<String, String> {
    if server.starts_with("https://") {
        return Err(text::NO_HTTPS.into());
    }
    let address = server.trim_start_matches("http://").trim_end_matches('/');
    let address = address.strip_suffix("/v1").unwrap_or(address);
    if address.contains(':') {
        Ok(address.to_string())
    } else {
        Ok(format!("{address}:{PORT}"))
    }
}

/// Tries this machine first, then, under WSL, the Windows host.
fn look_for_server() -> Result<String, String> {
    let mut candidates = vec![format!("localhost:{PORT}")];
    if system::is_wsl()
        && let Some(gateway) = system::default_gateway()
    {
        candidates.push(format!("{gateway}:{PORT}"));
    }
    candidates
        .iter()
        .find(|address| is_listening(address))
        .cloned()
        .ok_or_else(|| text::no_server(&candidates))
}

fn is_listening(address: &str) -> bool {
    let Ok(addresses) = address.to_socket_addrs() else {
        return false;
    };
    addresses
        .into_iter()
        .any(|socket| TcpStream::connect_timeout(&socket, Duration::from_millis(300)).is_ok())
}

/// The configured model, or else the one loaded in LM Studio, or else the
/// first one an OpenAI-compatible server lists; with what the server says
/// about it, when it's LM Studio or Ollama.
fn pick_model(address: &str, wanted: Option<&str>) -> Result<Model, String> {
    // LM Studio's own API says which model is loaded and with what context.
    if let Ok(listing) = http::get_json(address, "/api/v0/models")
        && let Some(models) = listing["data"].as_array()
    {
        return pick_lm_studio_model(models, wanted);
    }
    let listing = http::get_json(address, "/v1/models")?;
    let id = match wanted {
        Some(model) => model.to_string(),
        None => listing["data"][0]["id"]
            .as_str()
            .ok_or(text::NO_MODELS)?
            .to_string(),
    };
    if is_ollama(address) {
        let shown = http::post_for_json(address, "/api/show", &json!({"model": id}));
        let running = http::get_json(address, "/api/ps");
        return Ok(ollama_model(
            id,
            &shown.unwrap_or_default(),
            &running.unwrap_or_default(),
        ));
    }
    Ok(Model {
        id,
        context: None,
        max_context: None,
        vision: None,
        kind: Kind::Other,
    })
}

fn pick_lm_studio_model(models: &[Value], wanted: Option<&str>) -> Result<Model, String> {
    let model = match wanted {
        Some(name) => models
            .iter()
            .find(|model| model["id"] == name)
            .ok_or_else(|| text::no_such_model(name))?,
        None => models
            .iter()
            .find(|model| {
                model["state"] == "loaded" && (model["type"] == "llm" || model["type"] == "vlm")
            })
            .ok_or(text::NO_MODEL_LOADED)?,
    };
    Ok(Model {
        id: model["id"].as_str().unwrap_or_default().to_string(),
        context: model["loaded_context_length"].as_u64(),
        max_context: model["max_context_length"].as_u64(),
        vision: Some(model["type"] == "vlm"),
        kind: Kind::LmStudio,
    })
}

/// Ollama says its version at an address of its own, where other servers
/// say nothing, or an error.
fn is_ollama(address: &str) -> bool {
    http::get_json(address, "/api/version").is_ok_and(|reply| reply["version"].is_string())
}

/// The context Ollama has the model loaded with now, when it is.
pub fn loaded_context(server: &Server) -> Option<u64> {
    let running = http::get_json(&server.address, "/api/ps").ok()?;
    context_in(&running, &server.model)
}

/// What Ollama says about a model, from `/api/show` and `/api/ps`.
fn ollama_model(id: String, shown: &Value, running: &Value) -> Model {
    let architecture = shown["model_info"]["general.architecture"]
        .as_str()
        .unwrap_or_default();
    let max_context = shown["model_info"][format!("{architecture}.context_length")].as_u64();
    let vision = shown["capabilities"]
        .as_array()
        .map(|capabilities| capabilities.iter().any(|capability| capability == "vision"));
    // The model's own num_ctx is what a request without options loads it
    // with; without one, the context it's loaded with now, if it is.
    let context = own_num_ctx(shown).or_else(|| context_in(running, &id));
    Model {
        id,
        context,
        max_context,
        vision,
        kind: Kind::Ollama,
    }
}

/// The num_ctx in a model's own parameters, a line like `num_ctx  65536`.
fn own_num_ctx(shown: &Value) -> Option<u64> {
    shown["parameters"].as_str()?.lines().find_map(|line| {
        let mut words = line.split_whitespace();
        (words.next() == Some("num_ctx"))
            .then(|| words.next()?.parse().ok())
            .flatten()
    })
}

/// The context of the model as `/api/ps` lists it while it's loaded. A
/// name without a tag is the `latest` one, as for Ollama.
fn context_in(running: &Value, id: &str) -> Option<u64> {
    let tagged = |name: &str| {
        let last = name.rsplit('/').next().unwrap_or(name);
        if last.contains(':') {
            name.to_string()
        } else {
            format!("{name}:latest")
        }
    };
    let id = tagged(id);
    running["models"]
        .as_array()?
        .iter()
        .find(|model| model["name"].as_str().map(tagged).as_deref() == Some(id.as_str()))?
        ["context_length"]
        .as_u64()
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn reads_server_setting_in_any_form() {
        assert_eq!(
            address_from_setting("192.0.2.10").unwrap(),
            "192.0.2.10:1234"
        );
        assert_eq!(
            address_from_setting("192.0.2.10:8080").unwrap(),
            "192.0.2.10:8080"
        );
        assert_eq!(
            address_from_setting("http://192.0.2.10:1234/v1/").unwrap(),
            "192.0.2.10:1234"
        );
        assert!(address_from_setting("https://example.com").is_err());
    }

    #[test]
    fn picks_the_loaded_chat_model() {
        let models = json!([
            {"id": "embedder", "type": "embeddings", "state": "loaded"},
            {"id": "idle", "type": "llm", "state": "not-loaded"},
            {"id": "chat", "type": "llm", "state": "loaded",
             "loaded_context_length": 32768, "max_context_length": 131072}
        ]);
        let models = models.as_array().unwrap();
        assert_eq!(
            pick_lm_studio_model(models, None).unwrap(),
            Model {
                id: "chat".into(),
                context: Some(32768),
                max_context: Some(131072),
                vision: Some(false),
                kind: Kind::LmStudio,
            }
        );
        let idle = pick_lm_studio_model(models, Some("idle")).unwrap();
        assert_eq!((idle.id.as_str(), idle.context), ("idle", None));
        assert!(pick_lm_studio_model(models, Some("missing")).is_err());
    }

    #[test]
    fn lm_studio_says_a_model_sees_with_its_type() {
        let models = json!([{"id": "seer", "type": "vlm", "state": "loaded"}]);
        let model = pick_lm_studio_model(models.as_array().unwrap(), None).unwrap();
        assert_eq!(model.vision, Some(true));
    }

    /// As Ollama 0.40.2 answers, cut to the fields Lokal reads.
    fn shown(parameters: &str) -> Value {
        json!({
            "parameters": parameters,
            "model_info": {"general.architecture": "qwen35", "qwen35.context_length": 262144,
                           "qwen35.vision.block_count": 27},
            "capabilities": ["completion", "vision", "tools", "thinking"]
        })
    }

    fn running(name: &str, context: u64) -> Value {
        json!({"models": [{"name": name, "model": name, "context_length": context}]})
    }

    #[test]
    fn reads_what_ollama_says_about_a_model() {
        let model = ollama_model(
            "qwen".into(),
            &shown("num_ctx                        65536\nstop \"<|im_end|>\""),
            &Value::Null,
        );
        assert_eq!(
            model,
            Model {
                id: "qwen".into(),
                context: Some(65536),
                max_context: Some(262144),
                vision: Some(true),
                kind: Kind::Ollama,
            }
        );
    }

    #[test]
    fn without_its_own_num_ctx_the_loaded_context_counts() {
        let loaded = running("qwen:latest", 4096);
        assert_eq!(
            ollama_model("qwen".into(), &shown(""), &loaded).context,
            Some(4096)
        );
        let elsewhere = running("hf.co/a/b:Q4", 8192);
        assert_eq!(
            ollama_model("hf.co/a/b:Q4".into(), &shown(""), &elsewhere).context,
            Some(8192)
        );
        assert_eq!(
            ollama_model("qwen:7b".into(), &shown(""), &loaded).context,
            None
        );
    }

    #[test]
    fn a_model_ollama_doesnt_describe_has_nothing_known() {
        let model = ollama_model("nope".into(), &Value::Null, &Value::Null);
        assert_eq!(
            (model.context, model.max_context, model.vision),
            (None, None, None)
        );
        let text_only = json!({"capabilities": ["completion", "tools"]});
        assert_eq!(
            ollama_model("t".into(), &text_only, &Value::Null).vision,
            Some(false)
        );
    }
}
