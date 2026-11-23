//! The one place this crate makes HTTP requests: HTTPS only, with timeouts and a size cap.

use std::time::Duration;

/// GET `url` (HTTPS only) and return at most `max_bytes` of body.
pub fn get_bytes(url: &str, max_bytes: u64) -> Result<Vec<u8>, String> {
    if !url.starts_with("https://") {
        return Err(format!("refusing a non-HTTPS URL: {url}"));
    }
    let agent: ureq::Agent = ureq::Agent::config_builder()
        .timeout_connect(Some(Duration::from_secs(20)))
        .timeout_global(Some(Duration::from_secs(600)))
        .user_agent(concat!("desktop-core/", env!("CARGO_PKG_VERSION")))
        .build()
        .into();
    let mut resp = agent.get(url).call().map_err(|e| e.to_string())?;
    resp.body_mut().with_config().limit(max_bytes).read_to_vec().map_err(|e| e.to_string())
}
