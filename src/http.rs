//! Plain-HTTP requests to the model server, with errors a person can read.

use std::io::{BufRead, BufReader};
use std::time::Duration;

use serde_json::Value;

use crate::text;

fn agent() -> ureq::Agent {
    ureq::Agent::config_builder()
        // Error replies carry the server's explanation in the body.
        .http_status_as_error(false)
        // The server is local; a proxy set for the internet doesn't apply.
        .proxy(None)
        .timeout_connect(Some(Duration::from_secs(3)))
        .build()
        .into()
}

pub fn get_json(address: &str, path: &str) -> Result<Value, String> {
    let response = agent()
        .get(format!("http://{address}{path}"))
        .call()
        .map_err(|error| describe(error, address))?;
    let status = response.status().as_u16();
    let body = response
        .into_body()
        .read_to_string()
        .map_err(|error| describe(error, address))?;
    let value: Value = serde_json::from_str(&body).map_err(|_| text::not_json(address))?;
    if let Some(message) = error_message(&value) {
        return Err(text::server_said(&message));
    }
    if status >= 400 {
        return Err(text::server_status(status));
    }
    Ok(value)
}

/// Posts JSON and returns the reply body to read line by line.
pub fn post_json(address: &str, path: &str, body: &Value) -> Result<impl BufRead + use<>, String> {
    let response = agent()
        .post(format!("http://{address}{path}"))
        .header("Content-Type", "application/json")
        .send(body.to_string())
        .map_err(|error| describe(error, address))?;
    let status = response.status().as_u16();
    let mut body = response.into_body();
    if status >= 400 {
        let reply = body.read_to_string().unwrap_or_default();
        let message = serde_json::from_str(&reply)
            .ok()
            .and_then(|value| error_message(&value));
        return Err(message.map_or_else(
            || text::server_status(status),
            |message| text::server_said(&message),
        ));
    }
    Ok(BufReader::new(body.into_reader()))
}

/// Servers send errors as {"error": "..."} or {"error": {"message": "..."}}.
pub fn error_message(value: &Value) -> Option<String> {
    let error = value.get("error")?;
    let message = error.as_str().or_else(|| error.get("message")?.as_str());
    Some(message.map_or_else(|| error.to_string(), String::from))
}

fn describe(error: ureq::Error, address: &str) -> String {
    match error {
        ureq::Error::Io(_)
        | ureq::Error::ConnectionFailed
        | ureq::Error::HostNotFound
        | ureq::Error::Timeout(_) => text::cant_reach(address),
        other => format!("{} ({other})", text::cant_reach(address)),
    }
}
