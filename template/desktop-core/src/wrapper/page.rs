//! The script the wrapper adds to every page of the site, and the small calls it makes
//! into it.
//!
//! The page gets no IPC. The script only *receives* calls (`eval` from the app): key events
//! while keys are captured, the "Keys captured" overlay, download notices and the mute
//! fallback. The two things the page can ask for travel as a navigation to
//! [`CONTROL_HOST`] that the app catches and cancels (the same trick as the web view
//! bridge): "release the keys" (Linux, where the keys arrive in the page itself) and "show
//! this download in the file manager".
//!
//! It also removes the browser features switched off in Preferences (WebRTC, service
//! workers, WebAssembly) where the web view has no switch of its own.
//!
//! ```
//! use desktop_core::wrapper::{page, WebFeatures};
//!
//! let js = page::init_script(&WebFeatures { wasm: false, ..WebFeatures::default() });
//! assert!(js.contains("__kitchenWrap") && js.contains("WebAssembly"));
//! assert_eq!(page::control("https://kitchen-wrapper.invalid/release"), Some(page::Control::Release));
//! assert_eq!(page::control("https://kitchen-wrapper.invalid/reveal?id=3"), Some(page::Control::Reveal(3)));
//! assert_eq!(page::control("https://example.com/release"), None);
//! ```

use super::WebFeatures;

/// Navigations to this host are messages from the page script, never loaded. `.invalid`
/// never resolves (RFC 2606).
pub const CONTROL_HOST: &str = "kitchen-wrapper.invalid";

/// A message from the page script.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Control {
    /// The release shortcut was pressed (Linux).
    Release,
    /// Show download number `n` in the file manager.
    Reveal(u32),
}

/// Read a navigation to [`CONTROL_HOST`]; `None` for any other URL.
pub fn control(url: &str) -> Option<Control> {
    let rest = url.strip_prefix("https://")?.strip_prefix(CONTROL_HOST)?;
    match rest {
        "/release" => Some(Control::Release),
        r => r.strip_prefix("/reveal?id=").and_then(|n| n.parse().ok()).map(Control::Reveal),
    }
}

/// The initialization script, with `features` switched off where asked.
pub fn init_script(features: &WebFeatures) -> String {
    let mut off = String::new();
    if !features.webrtc {
        off.push_str("['RTCPeerConnection','webkitRTCPeerConnection','RTCDataChannel','RTCSessionDescription','RTCIceCandidate'].forEach(function(n){try{delete window[n];}catch(e){} });");
    }
    if !features.service_workers {
        off.push_str("try{Object.defineProperty(Navigator.prototype,'serviceWorker',{get:function(){return undefined;},configurable:false});}catch(e){}");
    }
    if !features.wasm {
        off.push_str("try{delete window.WebAssembly;}catch(e){}");
    }
    INIT_JS.replace("/*__FEATURES__*/", &off).replace("__HOST__", CONTROL_HOST)
}

/// Show or hide the overlay; `text` is shown as plain text.
pub fn overlay_script(on: bool, text: &str) -> String {
    format!("window.__kitchenWrap&&window.__kitchenWrap.overlay({on},{})", json(text))
}

/// A short notice at the bottom of the page; with `reveal`, a "Show in folder" button.
pub fn toast_script(text: &str, reveal: Option<(u32, &str)>) -> String {
    let (id, label) = reveal.unwrap_or((0, ""));
    format!("window.__kitchenWrap&&window.__kitchenWrap.toast({},{id},{})", json(text), json(label))
}

/// Mute or unmute every audio and video element (the fallback where the web view has no
/// mute of its own).
pub fn mute_script(muted: bool) -> String {
    format!("window.__kitchenWrap&&window.__kitchenWrap.mute({muted})")
}

/// Watch for the release shortcut in the page (Linux, where captured keys arrive in the
/// page as normal key events). `None` stops watching.
pub fn watch_script(release: Option<&crate::keymap::Shortcut>) -> String {
    match release {
        Some(s) => format!("window.__kitchenWrap&&window.__kitchenWrap.watch({},{})", json(&s.code), s.mods.bits()),
        None => "window.__kitchenWrap&&window.__kitchenWrap.watch(null,0)".into(),
    }
}

fn json(s: &str) -> String {
    serde_json::to_string(s).unwrap_or_else(|_| "\"\"".into()).replace('\u{2028}', "\\u2028").replace('\u{2029}', "\\u2029")
}

/// The script, before its placeholders are filled in (no `format!`: the braces would clash
/// with the template engine).
const INIT_JS: &str = r#"(function () {
  /*__FEATURES__*/
  if (window.top !== window || window.__kitchenWrap) return;
  var muted = false, watching = null, box = null, note = null;
  function el(tag, css) { var e = document.createElement(tag); e.style.cssText = css; return e; }
  function attach(e) { (document.body || document.documentElement).appendChild(e); }
  function applyMute(root) {
    (root.querySelectorAll ? root.querySelectorAll("audio,video") : []).forEach(function (m) { m.muted = muted; });
  }
  new MutationObserver(function (list) {
    if (!muted) return;
    list.forEach(function (r) { r.addedNodes.forEach(function (n) { if (n.nodeType === 1) { if (n.matches("audio,video")) n.muted = true; applyMute(n); } }); });
  }).observe(document, { childList: true, subtree: true });
  window.addEventListener("keydown", function (e) {
    if (!watching || e.code !== watching[0]) return;
    var m = (e.ctrlKey ? 1 : 0) | (e.altKey ? 2 : 0) | (e.shiftKey ? 4 : 0) | (e.metaKey ? 8 : 0);
    if (m !== watching[1]) return;
    e.preventDefault(); e.stopImmediatePropagation();
    location.href = "https://__HOST__/release";
  }, true);
  var api = {
    key: function (type, key, code, keyCode, mods, repeat) {
      var ev = new KeyboardEvent(type, { key: key, code: code, bubbles: true, cancelable: true, composed: true, repeat: repeat,
        ctrlKey: !!(mods & 1), altKey: !!(mods & 2), shiftKey: !!(mods & 4), metaKey: !!(mods & 8) });
      Object.defineProperty(ev, "keyCode", { get: function () { return keyCode; } });
      Object.defineProperty(ev, "which", { get: function () { return keyCode; } });
      (document.activeElement || document.body || document).dispatchEvent(ev);
    },
    overlay: function (on, text) {
      if (!box) {
        box = el("div", "position:fixed;top:10px;left:50%;transform:translateX(-50%);z-index:2147483647;pointer-events:none;" +
          "background:rgba(20,20,20,.85);color:#fff;font:13px system-ui,sans-serif;padding:6px 12px;border-radius:6px;transition:opacity .3s");
        attach(box);
      }
      box.textContent = text;
      box.style.display = on ? "block" : "none";
    },
    toast: function (text, id, label) {
      if (note) note.remove();
      note = el("div", "position:fixed;bottom:14px;right:14px;z-index:2147483647;background:rgba(20,20,20,.9);color:#fff;" +
        "font:13px system-ui,sans-serif;padding:8px 12px;border-radius:6px;display:flex;gap:10px;align-items:center");
      var t = el("span", ""); t.textContent = text; note.appendChild(t);
      if (id > 0) {
        var b = el("button", "font:inherit;color:#fff;background:#3a6df0;border:0;border-radius:4px;padding:3px 8px;cursor:pointer");
        b.textContent = label;
        b.onclick = function () { location.href = "https://__HOST__/reveal?id=" + id; };
        note.appendChild(b);
      }
      attach(note);
      var mine = note;
      setTimeout(function () { if (note === mine) { note.remove(); note = null; } }, 8000);
    },
    mute: function (m) { muted = !!m; applyMute(document); },
    watch: function (code, mods) { watching = code ? [code, mods] : null; }
  };
  Object.defineProperty(window, "__kitchenWrap", { value: Object.freeze(api) });
})();"#;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn features_are_removed_only_when_off() {
        let all = init_script(&WebFeatures::default());
        assert!(!all.contains("RTCPeerConnection") && !all.contains("serviceWorker") && !all.contains("delete window.WebAssembly"));
        let none = init_script(&WebFeatures { webrtc: false, service_workers: false, wasm: false });
        assert!(none.contains("RTCPeerConnection") && none.contains("serviceWorker") && none.contains("WebAssembly"));
        assert!(none.contains("https://kitchen-wrapper.invalid/release"));
    }

    #[test]
    fn calls_quote_their_text() {
        assert_eq!(overlay_script(true, "a\"b"), r#"window.__kitchenWrap&&window.__kitchenWrap.overlay(true,"a\"b")"#);
        assert!(toast_script("done", Some((4, "Show in Finder"))).ends_with(r#"toast("done",4,"Show in Finder")"#));
        assert!(toast_script("x</script>", None).contains(r#"toast("x</script>",0,"")"#));
        assert!(mute_script(true).ends_with("mute(true)"));
        let s: crate::keymap::Shortcut = "Ctrl+Alt+Shift+K".parse().unwrap();
        assert!(watch_script(Some(&s)).ends_with(r#"watch("KeyK",7)"#));
        assert!(watch_script(None).ends_with("watch(null,0)"));
    }

    #[test]
    fn control_messages() {
        assert_eq!(control("https://kitchen-wrapper.invalid/release"), Some(Control::Release));
        assert_eq!(control("https://kitchen-wrapper.invalid/reveal?id=12"), Some(Control::Reveal(12)));
        assert_eq!(control("https://kitchen-wrapper.invalid/reveal?id=x"), None);
        assert_eq!(control("https://kitchen-wrapper.invalid.evil.com/release"), None);
        assert_eq!(control("http://kitchen-wrapper.invalid/release"), None);
    }
}
