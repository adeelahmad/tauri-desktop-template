//! macOS (WKWebView): certificate pinning in the navigation delegate, page mute, the
//! `CGEventTap` that captures keys, and the privacy panes in System Settings.
//!
//! - **TLS.** wry's navigation delegate does not answer authentication challenges, so this
//!   adds `webView:didReceiveAuthenticationChallenge:completionHandler:` to its class. For a
//!   server-trust challenge from the site's host and port (and only with `insecureTls`), the
//!   leaf certificate's SHA-256 is checked against the pin: a match (or the first use) is
//!   accepted with `NSURLCredential credentialForTrust:`, anything else is cancelled. Every
//!   other challenge gets the default handling. The delegate is set again afterwards so
//!   WebKit sees the new method, and the window loads the site only after this is in place.
//! - **Mute** uses WKWebView's `_setPageMuted:` where it exists (macOS 11+), plus the page
//!   script's fallback.
//! - **Key capture** is a session-level `CGEventTap` on the main run loop. Taking keys away
//!   from the system needs **Accessibility**, seeing them needs **Input Monitoring**; both
//!   are granted in System Settings → Privacy & Security, and Preferences opens the panes.
//!   Media keys arrive as system-defined events and are taken too. Brightness, Touch ID,
//!   the power button and Mission Control's trackpad gestures are not keys and stay with
//!   macOS.
//! - **Screen sharing** (`getDisplayMedia`) asks for Screen Recording the first time.

use super::Hooks;
use desktop_core::keymap::{mac_code, mac_media_code, mac_modifier_bit};
use desktop_core::wrapper::pin::PinStore;
use desktop_core::wrapper::url::Origin;
use desktop_core::wrapper::WebFeatures;
use objc2::encode::{Encoding, RefEncode};
use objc2::runtime::{AnyClass, AnyObject, Bool, Sel};
use objc2::{class, msg_send, sel};
use serde_json::json;
use std::ffi::{c_char, c_void, CStr};
use std::sync::{Mutex, OnceLock, PoisonError};
use tauri::AppHandle;

/// Captured keys are taken before the web view sees them and sent to the page.
pub const KEYS_ARRIVE_IN_PAGE: bool = false;

/// `SecTrustRef`.
#[repr(C)]
pub struct SecTrust {
    _p: [u8; 0],
}
// SAFETY: `SecTrustRef` is `struct __SecTrust *`, which is what this encodes.
unsafe impl RefEncode for SecTrust {
    const ENCODING_REF: Encoding = Encoding::Pointer(&Encoding::Struct("__SecTrust", &[]));
}

/// `CGEventRef`.
#[repr(C)]
pub struct CGEvent {
    _p: [u8; 0],
}
// SAFETY: `CGEventRef` is `struct __CGEvent *`.
unsafe impl RefEncode for CGEvent {
    const ENCODING_REF: Encoding = Encoding::Pointer(&Encoding::Struct("__CGEvent", &[]));
}

type CFTypeRef = *const c_void;
type TapCallback = unsafe extern "C-unwind" fn(*mut c_void, u32, *mut CGEvent, *mut c_void) -> *mut CGEvent;

#[link(name = "CoreFoundation", kind = "framework")]
extern "C" {
    static kCFRunLoopCommonModes: CFTypeRef;
    fn CFRelease(cf: CFTypeRef);
    fn CFRunLoopGetMain() -> *mut c_void;
    fn CFRunLoopAddSource(rl: *mut c_void, source: *mut c_void, mode: CFTypeRef);
    fn CFMachPortCreateRunLoopSource(alloc: CFTypeRef, port: *mut c_void, order: isize) -> *mut c_void;
    fn CFArrayGetCount(a: CFTypeRef) -> isize;
    fn CFArrayGetValueAtIndex(a: CFTypeRef, i: isize) -> CFTypeRef;
    fn CFDataGetLength(d: CFTypeRef) -> isize;
    fn CFDataGetBytePtr(d: CFTypeRef) -> *const u8;
}

#[link(name = "CoreGraphics", kind = "framework")]
extern "C" {
    fn CGEventTapCreate(tap: u32, place: u32, options: u32, mask: u64, callback: TapCallback, info: *mut c_void) -> *mut c_void;
    fn CGEventTapEnable(tap: *mut c_void, enable: bool);
    fn CGEventGetIntegerValueField(event: *mut CGEvent, field: u32) -> i64;
    fn CGEventGetFlags(event: *mut CGEvent) -> u64;
    fn CGPreflightListenEventAccess() -> bool;
    fn CGRequestListenEventAccess() -> bool;
    fn CGPreflightScreenCaptureAccess() -> bool;
}

#[link(name = "ApplicationServices", kind = "framework")]
extern "C" {
    fn AXIsProcessTrusted() -> u8;
}

#[link(name = "Security", kind = "framework")]
extern "C" {
    fn SecTrustCopyCertificateChain(trust: *mut SecTrust) -> CFTypeRef;
    fn SecCertificateCopyData(cert: CFTypeRef) -> CFTypeRef;
}

/// The site and pin the challenge handler checks (one wrapped site per app).
static TLS: OnceLock<Mutex<Option<(Origin, PinStore)>>> = OnceLock::new();

/// Wire the hooks into a new web view.
pub fn install(pv: &tauri_runtime_wry::Webview, hooks: Hooks) {
    let wv = pv.inner() as *mut AnyObject;
    if hooks.muted {
        mute(wv, true);
    }
    if let Some(pins) = hooks.pins {
        *TLS.get_or_init(Mutex::default).lock().unwrap_or_else(PoisonError::into_inner) = Some((hooks.site, pins));
        // SAFETY: `wv` is the live WKWebView Tauri handed us on the main thread.
        unsafe { add_challenge_handler(wv) };
    }
}

/// Add the challenge method to the class of the web view's navigation delegate (once).
///
/// # Safety
/// `wv` must be a valid `WKWebView`.
unsafe fn add_challenge_handler(wv: *mut AnyObject) {
    // SAFETY (whole block): plain Objective-C calls on objects owned by the web view, with
    // argument and return types matching the WebKit headers.
    unsafe {
        let delegate: *mut AnyObject = msg_send![wv, navigationDelegate];
        let Some(delegate) = delegate.as_ref() else {
            log::warn!("no navigation delegate: certificate pinning is not active");
            return;
        };
        let cls: &AnyClass = delegate.class();
        let sel = sel!(webView:didReceiveAuthenticationChallenge:completionHandler:);
        if cls.instance_method(sel).is_none() {
            add_method(cls, sel);
        }
        // WebKit reads which optional methods a navigation delegate has when the delegate is
        // set, so a method added afterwards is never called. Setting the same delegate again
        // makes it look again.
        let _: () = msg_send![wv, setNavigationDelegate: delegate];
    }
}

/// Add `did_receive_challenge` to `cls` as `sel`.
///
/// # Safety
/// `cls` must be the navigation delegate's class and `sel` the challenge selector.
unsafe fn add_method(cls: &AnyClass, sel: Sel) {
    // SAFETY: see the callers; the IMP's arguments match the type string.
    unsafe {
        type Imp = unsafe extern "C-unwind" fn(
            *mut AnyObject,
            Sel,
            *mut AnyObject,
            *mut AnyObject,
            *mut block2::Block<dyn Fn(isize, *mut AnyObject)>,
        );
        let imp: Imp = did_receive_challenge;
        // SAFETY: an IMP is called through objc_msgSend with exactly the arguments `types` names.
        let imp: objc2::runtime::Imp = std::mem::transmute::<Imp, objc2::runtime::Imp>(imp);
        let added = objc2::ffi::class_addMethod(cls as *const AnyClass as *mut AnyClass, sel, imp, c"v@:@@@?".as_ptr());
        if !added.as_bool() {
            log::warn!("could not add the certificate handler: pinning is not active");
        }
    }
}

/// `NSURLSessionAuthChallengeDisposition`.
const USE_CREDENTIAL: isize = 0;
const DEFAULT_HANDLING: isize = 1;
const CANCEL: isize = 2;

unsafe extern "C-unwind" fn did_receive_challenge(
    _this: *mut AnyObject,
    _cmd: Sel,
    _wv: *mut AnyObject,
    challenge: *mut AnyObject,
    handler: *mut block2::Block<dyn Fn(isize, *mut AnyObject)>,
) {
    // SAFETY: WebKit calls this with a live challenge and completion handler, which must be
    // called exactly once.
    unsafe {
        let (disposition, credential) = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| decide_challenge(challenge)))
            .unwrap_or((CANCEL, std::ptr::null_mut()));
        if let Some(h) = handler.as_ref() {
            h.call((disposition, credential));
        }
    }
}

/// # Safety
/// `challenge` must be a valid `NSURLAuthenticationChallenge`.
unsafe fn decide_challenge(challenge: *mut AnyObject) -> (isize, *mut AnyObject) {
    let none = (DEFAULT_HANDLING, std::ptr::null_mut());
    let Some(state) = TLS.get().and_then(|m| m.lock().unwrap_or_else(PoisonError::into_inner).clone()) else { return none };
    let (site, pins) = state;
    // SAFETY: documented WebKit/Foundation accessors with matching types.
    unsafe {
        let space: *mut AnyObject = msg_send![challenge, protectionSpace];
        if space.is_null() {
            return none;
        }
        let method: *mut AnyObject = msg_send![space, authenticationMethod];
        let host: *mut AnyObject = msg_send![space, host];
        let port: isize = msg_send![space, port];
        if nsstring(method).as_deref() != Some("NSURLAuthenticationMethodServerTrust") {
            return none;
        }
        let host = nsstring(host).unwrap_or_default().to_ascii_lowercase();
        if host != site.host.trim_start_matches('[').trim_end_matches(']') || isize::try_from(site.port).ok() != Some(port) {
            return none;
        }
        let trust: *mut SecTrust = msg_send![space, serverTrust];
        let Some(der) = leaf_der(trust) else { return (CANCEL, std::ptr::null_mut()) };
        if !pins.accept(&der) {
            return (CANCEL, std::ptr::null_mut());
        }
        let credential: *mut AnyObject = msg_send![class!(NSURLCredential), credentialForTrust: trust];
        (USE_CREDENTIAL, credential)
    }
}

/// The DER bytes of the first certificate in `trust`'s chain.
///
/// # Safety
/// `trust` must be null or a valid `SecTrustRef`.
unsafe fn leaf_der(trust: *mut SecTrust) -> Option<Vec<u8>> {
    if trust.is_null() {
        return None;
    }
    // SAFETY: Security/CoreFoundation calls following the Create/Copy rule; both copies are released.
    unsafe {
        let chain = SecTrustCopyCertificateChain(trust);
        if chain.is_null() {
            return None;
        }
        let out = if CFArrayGetCount(chain) > 0 {
            let data = SecCertificateCopyData(CFArrayGetValueAtIndex(chain, 0));
            let bytes =
                (!data.is_null()).then(|| std::slice::from_raw_parts(CFDataGetBytePtr(data), CFDataGetLength(data) as usize).to_vec());
            if !data.is_null() {
                CFRelease(data);
            }
            bytes
        } else {
            None
        };
        CFRelease(chain);
        out
    }
}

/// # Safety
/// `s` must be null or an `NSString`.
unsafe fn nsstring(s: *mut AnyObject) -> Option<String> {
    if s.is_null() {
        return None;
    }
    // SAFETY: UTF8String returns a NUL-terminated buffer that lives as long as `s`.
    unsafe {
        let p: *const c_char = msg_send![s, UTF8String];
        (!p.is_null()).then(|| CStr::from_ptr(p).to_string_lossy().into_owned())
    }
}

fn mute(wv: *mut AnyObject, muted: bool) {
    // SAFETY: `wv` is a WKWebView; the private selector is only sent when it responds to it.
    unsafe {
        let responds: Bool = msg_send![wv, respondsToSelector: sel!(_setPageMuted:)];
        if responds.as_bool() {
            // WKMediaMutedState: 1 = audio muted.
            let _: () = msg_send![wv, _setPageMuted: usize::from(muted)];
        }
    }
}

/// Mute or unmute the page.
pub fn set_muted(pv: &tauri_runtime_wry::Webview, muted: bool) {
    mute(pv.inner() as *mut AnyObject, muted);
}

/// No extra browser arguments on macOS (features are switched off by the page script).
pub fn browser_args(_features: &WebFeatures) -> Option<String> {
    None
}

// ---- key capture ----

struct Tap(*mut c_void);
// SAFETY: a CFMachPort may be enabled and disabled from any thread.
unsafe impl Send for Tap {}
unsafe impl Sync for Tap {}

static TAP: OnceLock<Tap> = OnceLock::new();

const KEY_DOWN: u32 = 10;
const KEY_UP: u32 = 11;
const FLAGS_CHANGED: u32 = 12;
const SYSTEM_DEFINED: u32 = 14;
const TAP_DISABLED_BY_TIMEOUT: u32 = 0xFFFF_FFFE;
const TAP_DISABLED_BY_USER: u32 = 0xFFFF_FFFF;
const FIELD_KEYCODE: u32 = 9;

unsafe extern "C-unwind" fn tap_callback(_proxy: *mut c_void, kind: u32, event: *mut CGEvent, _info: *mut c_void) -> *mut CGEvent {
    if kind == TAP_DISABLED_BY_TIMEOUT || kind == TAP_DISABLED_BY_USER {
        if let Some(t) = TAP.get() {
            if super::capture::is_active() {
                // SAFETY: the tap is a live CFMachPort.
                unsafe { CGEventTapEnable(t.0, true) };
            }
        }
        return event;
    }
    // SAFETY: `event` is the live event of this callback.
    let taken = std::panic::catch_unwind(|| unsafe { handle(kind, event) }).unwrap_or(false);
    if taken {
        std::ptr::null_mut()
    } else {
        event
    }
}

/// # Safety
/// `event` must be the event passed to the tap callback.
unsafe fn handle(kind: u32, event: *mut CGEvent) -> bool {
    use super::capture::key;
    // SAFETY: CoreGraphics accessors on the callback's event.
    unsafe {
        match kind {
            KEY_DOWN | KEY_UP => {
                let code = CGEventGetIntegerValueField(event, FIELD_KEYCODE) as u16;
                match mac_code(code) {
                    Some(c) => key(c, kind == KEY_DOWN),
                    None => super::capture::is_active(),
                }
            }
            FLAGS_CHANGED => {
                let code = CGEventGetIntegerValueField(event, FIELD_KEYCODE) as u16;
                let (Some(c), Some(bit)) = (mac_code(code), mac_modifier_bit(code)) else { return false };
                let down = CGEventGetFlags(event) & bit != 0;
                if c == "CapsLock" {
                    // Caps Lock reports its lock state: send a press for each change.
                    key(c, true) | key(c, false)
                } else {
                    key(c, down)
                }
            }
            SYSTEM_DEFINED => {
                let ns: *mut AnyObject = msg_send![class!(NSEvent), eventWithCGEvent: event];
                if ns.is_null() {
                    return false;
                }
                let subtype: i16 = msg_send![ns, subtype];
                if subtype != 8 {
                    return false;
                }
                let data1: isize = msg_send![ns, data1];
                let nx = ((data1 as usize & 0xFFFF_0000) >> 16) as u32;
                let state = (data1 as usize & 0xFF00) >> 8;
                match mac_media_code(nx) {
                    Some(c) => key(c, state == 0x0A),
                    None => false,
                }
            }
            _ => false,
        }
    }
}

/// Start the event tap (made once; enabled while capturing).
pub fn capture_start(_app: &AppHandle) -> Result<(), String> {
    if !permissions().granted {
        // SAFETY: plain call; it adds the app to the Input Monitoring list.
        unsafe { CGRequestListenEventAccess() };
        return Err("key capture needs Accessibility and Input Monitoring (Preferences → Permissions)".into());
    }
    if let Some(t) = TAP.get() {
        // SAFETY: the tap is a live CFMachPort.
        unsafe { CGEventTapEnable(t.0, true) };
        return Ok(());
    }
    let mask = (1u64 << KEY_DOWN) | (1 << KEY_UP) | (1 << FLAGS_CHANGED) | (1 << SYSTEM_DEFINED);
    // SAFETY: CGEventTapCreate with a static callback; kCGSessionEventTap (1),
    // kCGHeadInsertEventTap (0), kCGEventTapOptionDefault (0: may drop events).
    unsafe {
        let port = CGEventTapCreate(1, 0, 0, mask, tap_callback, std::ptr::null_mut());
        if port.is_null() {
            return Err("macOS refused the key tap: grant Accessibility and Input Monitoring, then try again".into());
        }
        let source = CFMachPortCreateRunLoopSource(std::ptr::null(), port, 0);
        CFRunLoopAddSource(CFRunLoopGetMain(), source, kCFRunLoopCommonModes);
        CFRelease(source);
        CGEventTapEnable(port, true);
        let _ = TAP.set(Tap(port));
    }
    Ok(())
}

/// Disable the tap (it stays installed for next time).
pub fn capture_stop(_app: &AppHandle) {
    if let Some(t) = TAP.get() {
        // SAFETY: the tap is a live CFMachPort.
        unsafe { CGEventTapEnable(t.0, false) };
    }
}

/// Accessibility, Input Monitoring and Screen Recording, as granted now.
pub fn permissions() -> super::Permissions {
    // SAFETY: plain status queries.
    let (ax, input, screen) = unsafe { (AXIsProcessTrusted() != 0, CGPreflightListenEventAccess(), CGPreflightScreenCaptureAccess()) };
    super::Permissions {
        granted: ax && input,
        items: json!([
            {"id": "accessibility", "label": "Accessibility (take keys from the system)", "granted": ax},
            {"id": "input", "label": "Input Monitoring (see keys)", "granted": input},
            {"id": "screen", "label": "Screen Recording (only for screen sharing)", "granted": screen},
        ]),
    }
}

/// Open System Settings at a privacy pane.
pub fn open_privacy(pane: &str) -> std::io::Result<()> {
    let anchor = match pane {
        "accessibility" => "Privacy_Accessibility",
        "input" => "Privacy_ListenEvent",
        "screen" => "Privacy_ScreenCapture",
        "microphone" => "Privacy_Microphone",
        "camera" => "Privacy_Camera",
        _ => return Err(std::io::Error::other("unknown pane")),
    };
    std::process::Command::new("open").arg(format!("x-apple.systempreferences:com.apple.preference.security?{anchor}")).spawn().map(|_| ())
}
