//! Stand-in for `desktop_core::net`: the spike has no offline mode or deny list, so every
//! host is allowed. In the template the proxy goes through the real network policy.

/// Whether the proxy may contact `host`.
pub fn check_host(_host: &str) -> Result<(), String> {
    Ok(())
}
