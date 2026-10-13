//! Finding the model server and the model it has loaded.

use std::net::{TcpStream, ToSocketAddrs};
use std::time::Duration;

use serde_json::Value;

use crate::config::Config;
use crate::{http, system, text};

/// LM Studio's default port.
const PORT: u16 = 1234;

pub struct Server {
    /// host:port
    pub address: String,
    pub model: String,
    /// The model's context length in tokens, when the server reports it.
    pub context: Option<u64>,
    /// Set in the settings, rather than found.
    pub pinned: bool,
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
    let (model, context) = pick_model(&address, config.model.as_deref())?;
    Ok(Server {
        address,
        model,
        context,
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
/// first one an OpenAI-compatible server lists.
fn pick_model(address: &str, wanted: Option<&str>) -> Result<(String, Option<u64>), String> {
    // LM Studio's own API says which model is loaded and with what context.
    if let Ok(listing) = http::get_json(address, "/api/v0/models")
        && let Some(models) = listing["data"].as_array()
    {
        return pick_lm_studio_model(models, wanted);
    }
    let listing = http::get_json(address, "/v1/models")?;
    if let Some(model) = wanted {
        return Ok((model.to_string(), None));
    }
    listing["data"][0]["id"]
        .as_str()
        .map(|id| (id.to_string(), None))
        .ok_or_else(|| text::NO_MODELS.into())
}

fn pick_lm_studio_model(
    models: &[Value],
    wanted: Option<&str>,
) -> Result<(String, Option<u64>), String> {
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
    let id = model["id"].as_str().unwrap_or_default().to_string();
    Ok((id, model["loaded_context_length"].as_u64()))
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
            {"id": "chat", "type": "llm", "state": "loaded", "loaded_context_length": 32768}
        ]);
        let models = models.as_array().unwrap();
        assert_eq!(
            pick_lm_studio_model(models, None).unwrap(),
            ("chat".into(), Some(32768))
        );
        assert_eq!(
            pick_lm_studio_model(models, Some("idle")).unwrap(),
            ("idle".into(), None)
        );
        assert!(pick_lm_studio_model(models, Some("missing")).is_err());
    }
}
