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

/// Stream `url` (HTTPS only) into `dest`, resuming a partial `<dest>.part` and renaming it
/// into place when complete. `progress(bytes_so_far, total)` runs as data arrives.
pub fn download_to(url: &str, dest: &std::path::Path, progress: &mut dyn FnMut(u64, Option<u64>)) -> Result<(), String> {
    use std::io::{Read, Write};
    if !url.starts_with("https://") {
        return Err(format!("refusing a non-HTTPS URL: {url}"));
    }
    if let Some(dir) = dest.parent() {
        std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    }
    let part = dest.with_extension(format!("{}part", dest.extension().map(|e| format!("{}.", e.to_string_lossy())).unwrap_or_default()));
    let have = std::fs::metadata(&part).map(|m| m.len()).unwrap_or(0);
    let agent: ureq::Agent = ureq::Agent::config_builder()
        .timeout_connect(Some(Duration::from_secs(20)))
        .timeout_recv_body(Some(Duration::from_secs(120)))
        .http_status_as_error(false)
        .user_agent(concat!("desktop-core/", env!("CARGO_PKG_VERSION")))
        .build()
        .into();
    let mut req = agent.get(url);
    if have > 0 {
        req = req.header("range", &format!("bytes={have}-"));
    }
    let resp = req.call().map_err(|e| e.to_string())?;
    let status = resp.status().as_u16();
    let resumed = status == 206;
    if !(200..300).contains(&status) {
        return Err(format!("HTTP {status} for {url}"));
    }
    let len = resp.headers().get("content-length").and_then(|v| v.to_str().ok()).and_then(|v| v.parse::<u64>().ok());
    let mut done = if resumed { have } else { 0 };
    let total = len.map(|l| l + done);
    let mut file = std::fs::OpenOptions::new().create(true).write(true).append(resumed).truncate(!resumed).open(&part).map_err(|e| e.to_string())?;
    let mut body = resp.into_body().into_reader();
    let mut buf = vec![0u8; 1 << 16];
    let mut last = 0u64;
    loop {
        let n = body.read(&mut buf).map_err(|e| e.to_string())?;
        if n == 0 {
            break;
        }
        file.write_all(&buf[..n]).map_err(|e| e.to_string())?;
        done += n as u64;
        if done - last >= 1 << 20 {
            last = done;
            progress(done, total);
        }
    }
    file.sync_all().map_err(|e| e.to_string())?;
    if total.is_some_and(|t| t != done) {
        return Err(format!("download stopped at {done} of {} bytes; try again to resume", total.unwrap_or(0)));
    }
    progress(done, total);
    std::fs::rename(&part, dest).map_err(|e| e.to_string())
}
