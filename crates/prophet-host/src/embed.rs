// SPDX-License-Identifier: GPL-3.0-only
//! Shared cross-platform embedded GUI shell for VST3, CLAP, LV2, and Desktop Standalone.
//!
//! Zero external HTTP server dependency: all HTML, JS, CSS, and WASM assets from `ui/dist` are
//! baked into the binary at build time via `build.rs` (`EMBEDDED_UI_ASSETS`) and exposed cleanly to:
//! - Linux X11/GTK3 (`GtkPlug` parented inside DAW X11 `Window`, or `GtkWindow` for Standalone),
//!   using `libwebkit2gtk-4.1`/`4.0` when available at runtime or an interactive GTK3+Cairo canvas
//!   fallback when WebKit2GTK is absent (e.g. headless CI / Xvfb).
//! - Windows x64 (`HWND` child hosting Microsoft WebView2 COM runtime via `WebView2Loader.dll` with
//!   offline `https://prophet.local/` asset interception, plus native GDI interactive fallback).
//! - Android/macOS safe stubs (Android uses its own APK WebView + JNI bridge cleanly).

use crate::{UiCommand, UiCommandReply, UiSnapshot};
use std::sync::Arc;

#[derive(Clone, Copy, Debug)]
pub struct EmbeddedUiAsset {
    pub path: &'static str,
    pub mime: &'static str,
    pub bytes: &'static [u8],
}

include!(concat!(env!("OUT_DIR"), "/embedded_ui_assets.rs"));

/// Resolves an embedded React build artifact (`/index.html`, `/assets/...`) without touching the
/// filesystem or spawning any HTTP listener.
pub fn resolve_embedded_asset(raw_path: &str) -> Option<&'static EmbeddedUiAsset> {
    let trimmed = raw_path
        .split(['?', '#'])
        .next()
        .unwrap_or("")
        .trim();
    let normalized = if trimmed.is_empty() || trimmed == "/" {
        "/index.html".to_owned()
    } else if trimmed.starts_with('/') {
        trimmed.to_owned()
    } else {
        format!("/{trimmed}")
    };
    EMBEDDED_UI_ASSETS
        .iter()
        .find(|asset| asset.path == normalized)
        .or_else(|| {
            if !normalized.contains('.') {
                EMBEDDED_UI_ASSETS.iter().find(|a| a.path == "/index.html")
            } else {
                None
            }
        })
}

/// Canonical `window.Native` / `window.__PROPHET_HOST__` bootstrap injected into HTML before any
/// module script executes, preserving full message-parity across WebKit2GTK, WebView2, and Android.
pub fn native_bridge_bootstrap_script(backend_tag: &str, mode: BridgeTransportMode) -> String {
    let tag_json = serde_json::to_string(backend_tag).unwrap_or_else(|_| "\"native\"".to_owned());
    let post_expr = match mode {
        BridgeTransportMode::WebKitMessageHandler => {
            "window.webkit && window.webkit.messageHandlers && window.webkit.messageHandlers.prophet && window.webkit.messageHandlers.prophet.postMessage(payload)"
        }
        BridgeTransportMode::WebView2PostMessage => {
            "window.chrome && window.chrome.webview && window.chrome.webview.postMessage(payload)"
        }
        BridgeTransportMode::CustomGlobal => {
            "window.__PROPHET_HOST_POST__ && window.__PROPHET_HOST_POST__(payload)"
        }
    };
    format!(
        r#"(function(){{
  var pending = new Map();
  var nextSeq = 1;
  var latestSnapshot = "";
  var latestZoom = 1.0;
  window.__PROPHET_HOST_RECEIVE__ = function(snapshotJson) {{
    latestSnapshot = typeof snapshotJson === "string" ? snapshotJson : JSON.stringify(snapshotJson);
    try {{
      var parsed = JSON.parse(latestSnapshot);
      window.dispatchEvent(new CustomEvent("prophet-snapshot", {{ detail: parsed }}));
    }} catch (_e) {{}}
  }};
  window.__PROPHET_HOST_REPLY__ = function(seq, replyJson) {{
    var cb = pending.get(seq);
    if (cb) {{
      pending.delete(seq);
      try {{ cb.resolve(typeof replyJson === "string" ? JSON.parse(replyJson) : replyJson); }}
      catch (err) {{ cb.resolve({{ ok: false, message: String(err) }}); }}
    }}
  }};
  function sendRaw(jsonText, seq) {{
    var payload = JSON.stringify({{ seq: seq || 0, cmd: typeof jsonText === "string" ? JSON.parse(jsonText) : jsonText }});
    {post_expr};
  }}
  window.__PROPHET_HOST__ = {{
    backend: {tag_json},
    dispatchAsync: function(cmdObj) {{
      var seq = nextSeq++;
      return new Promise(function(resolve) {{
        pending.set(seq, {{ resolve: resolve }});
        sendRaw(cmdObj, seq);
      }});
    }},
    requestSnapshot: function() {{
      sendRaw({{ type: "__snapshot__" }}, 0);
      return latestSnapshot;
    }},
    setZoom: function(z) {{
      latestZoom = Number(z) || 1.0;
      document.documentElement.style.zoom = String(latestZoom);
      sendRaw({{ type: "hostControl", op: "setUiZoom", buffer_frames: Math.round(latestZoom * 100) }}, 0);
    }},
    getZoom: function() {{ return latestZoom; }}
  }};
  if (!window.Native) {{
    window.Native = {{
      dispatch: function(jsonStr) {{
        try {{
          sendRaw(jsonStr, 0);
          return JSON.stringify({{ ok: true, status: "applied", message: "Queued" }});
        }} catch (e) {{
          return JSON.stringify({{ ok: false, status: "invalid", message: String(e) }});
        }}
      }},
      poll: function() {{
        return latestSnapshot;
      }}
    }};
  }}
}})();"#,
    )
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BridgeTransportMode {
    WebKitMessageHandler,
    WebView2PostMessage,
    CustomGlobal,
}

/// Inserts `<script>{bootstrap}</script>` directly before the first `<script` or `</head>` tag.
pub fn inject_bridge_into_html(html: &[u8], bridge_bootstrap_js: &str) -> Vec<u8> {
    let text = String::from_utf8_lossy(html);
    let snippet = format!("<script>{bridge_bootstrap_js}</script>\n");
    if let Some(pos) = text.find("<script") {
        let mut out = String::with_capacity(text.len() + snippet.len());
        out.push_str(&text[..pos]);
        out.push_str(&snippet);
        out.push_str(&text[pos..]);
        return out.into_bytes();
    }
    if let Some(pos) = text.find("</head>") {
        let mut out = String::with_capacity(text.len() + snippet.len());
        out.push_str(&text[..pos]);
        out.push_str(&snippet);
        out.push_str(&text[pos..]);
        return out.into_bytes();
    }
    let mut out = snippet;
    out.push_str(&text);
    out.into_bytes()
}

#[derive(Clone, Copy, Debug)]
pub enum ParentWindow {
    X11(u32),
    Win32(*mut core::ffi::c_void),
    Cocoa(*mut core::ffi::c_void),
    StandaloneWindow,
}

#[derive(Clone, Debug)]
pub struct EditorViewConfig {
    pub title: String,
    pub backend_tag: String,
    pub logical_width: u32,
    pub logical_height: u32,
    pub scale_factor: f32,
    pub resizable: bool,
}

impl Default for EditorViewConfig {
    fn default() -> Self {
        Self {
            title: "PROPHET-XII Virtual Analog Synthesizer".to_owned(),
            backend_tag: "desktop".to_owned(),
            logical_width: 1180,
            logical_height: 720,
            scale_factor: 1.0,
            resizable: false,
        }
    }
}

/// Abstraction decoupling platform embedded windowing / WebView shells from `HostControlPlane` +
/// DAW plugin wrappers (`ParamSetter`).
pub trait HostBridgeSink: Send + Sync {
    fn snapshot_json(&self) -> String;
    fn capture_ui_state(&self) -> UiSnapshot;
    fn dispatch_command(&self, cmd: UiCommand) -> UiCommandReply;
    fn on_zoom_changed(&self, _zoom: f32) {}
}

#[derive(Debug)]
pub struct EmbedError {
    pub message: String,
}

impl core::fmt::Display for EmbedError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str(&self.message)
    }
}

impl std::error::Error for EmbedError {}

pub struct EmbeddedEditorHandle {
    #[cfg(all(unix, not(target_os = "android"), not(target_os = "macos")))]
    inner: Option<linux_gtk::LinuxEditorHandle>,
    #[cfg(target_os = "windows")]
    inner: Option<windows_webview2::WindowsEditorHandle>,
    #[cfg(any(target_os = "android", target_os = "macos"))]
    _phantom: (),
    backend_kind: &'static str,
}

// Raw X11 / Win32 window handles live cleanly inside synchronized actors/dispatchers.
unsafe impl Send for EmbeddedEditorHandle {}

impl EmbeddedEditorHandle {
    pub fn backend_kind(&self) -> &'static str {
        self.backend_kind
    }

    pub fn is_open(&self) -> bool {
        #[cfg(all(unix, not(target_os = "android"), not(target_os = "macos")))]
        if let Some(ref h) = self.inner {
            return h.is_open();
        }
        #[cfg(target_os = "windows")]
        if let Some(ref h) = self.inner {
            return h.is_open();
        }
        false
    }

    pub fn resize(&self, logical_width: u32, logical_height: u32, scale_factor: f32) {
        #[cfg(all(unix, not(target_os = "android"), not(target_os = "macos")))]
        if let Some(ref h) = self.inner {
            h.resize(logical_width, logical_height, scale_factor);
        }
        #[cfg(target_os = "windows")]
        if let Some(ref h) = self.inner {
            h.resize(logical_width, logical_height, scale_factor);
        }
        #[cfg(any(target_os = "android", target_os = "macos"))]
        let _ = (logical_width, logical_height, scale_factor);
    }

    pub fn child_x11_window(&self) -> Option<u32> {
        #[cfg(all(unix, not(target_os = "android"), not(target_os = "macos")))]
        if let Some(ref h) = self.inner {
            return h.child_xid();
        }
        None
    }

    /// Runs a blocking event loop when hosting `ParentWindow::StandaloneWindow` without an
    /// outer GTK loop. Returns when the user closes the window or `stop_flag` turns `true`.
    pub fn pump_until_closed(&self, stop_flag: &std::sync::atomic::AtomicBool) {
        while self.is_open() && !stop_flag.load(std::sync::atomic::Ordering::Acquire) {
            std::thread::sleep(std::time::Duration::from_millis(16));
        }
    }
}

pub fn spawn_embedded_editor(
    parent: ParentWindow,
    config: EditorViewConfig,
    sink: Arc<dyn HostBridgeSink>,
) -> Result<EmbeddedEditorHandle, EmbedError> {
    #[cfg(all(unix, not(target_os = "android"), not(target_os = "macos")))]
    {
        let handle = linux_gtk::spawn(parent, config, sink)?;
        let kind = handle.kind();
        Ok(EmbeddedEditorHandle {
            inner: Some(handle),
            backend_kind: kind,
        })
    }
    #[cfg(target_os = "windows")]
    {
        let handle = windows_webview2::spawn(parent, config, sink)?;
        let kind = handle.kind();
        Ok(EmbeddedEditorHandle {
            inner: Some(handle),
            backend_kind: kind,
        })
    }
    #[cfg(any(target_os = "android", target_os = "macos"))]
    {
        let _ = (parent, config, sink);
        Err(EmbedError {
            message: "Desktop embedded WebView shell is not available on this target".to_owned(),
        })
    }
}

// ============================================================================
// Linux GTK3 + Dynamic WebKit2GTK (Tier 1) + Interactive Cairo Fallback (Tier 2)
// ============================================================================

#[cfg(all(unix, not(target_os = "android"), not(target_os = "macos")))]
mod linux_gtk {
    use super::{
        inject_bridge_into_html, native_bridge_bootstrap_script, resolve_embedded_asset,
        BridgeTransportMode, EditorViewConfig, EmbedError, HostBridgeSink, ParentWindow,
    };
    use crate::UiCommand;
    use prophet_params::ParamId;
    use std::collections::BTreeMap;
    use std::ffi::{c_char, c_double, c_int, c_uint, c_ulong, c_void, CStr, CString};
    use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering};
    use std::sync::{mpsc, Arc, Mutex, Once, OnceLock};

    struct DynLib {
        handle: *mut c_void,
    }

    unsafe impl Send for DynLib {}
    unsafe impl Sync for DynLib {}

    impl DynLib {
        unsafe fn open(soname: &str) -> Result<Self, String> {
            let cname = CString::new(soname).map_err(|_| format!("Invalid library name: {soname}"))?;
            let _ = libc::dlerror();
            let handle = libc::dlopen(cname.as_ptr(), libc::RTLD_LAZY | libc::RTLD_LOCAL);
            if handle.is_null() {
                let err = libc::dlerror();
                let msg = if err.is_null() {
                    format!("dlopen({soname}) returned null")
                } else {
                    CStr::from_ptr(err).to_string_lossy().into_owned()
                };
                Err(msg)
            } else {
                Ok(Self { handle })
            }
        }

        unsafe fn get_raw(&self, nul_name: &[u8]) -> Option<*mut c_void> {
            if nul_name.last().copied() != Some(0) {
                return None;
            }
            let ptr = libc::dlsym(self.handle, nul_name.as_ptr() as *const c_char);
            if ptr.is_null() {
                None
            } else {
                Some(ptr)
            }
        }
    }

    impl Drop for DynLib {
        fn drop(&mut self) {
            // Note: GTK/GLib/WebKit registers static GType classes globally inside the process,
            // so unloading those shared libraries can leave dangling GType closures; we keep handles
            // alive intentionally inside `OnceLock<GtkSyms>` and only close failed transient loads.
            if !self.handle.is_null() {
                unsafe {
                    let _ = libc::dlclose(self.handle);
                }
            }
        }
    }

    const GDK_EXPOSURE_MASK: c_int = 1 << 1;
    const GDK_POINTER_MOTION_MASK: c_int = 1 << 2;
    const GDK_BUTTON_PRESS_MASK: c_int = 1 << 8;
    const GDK_BUTTON_RELEASE_MASK: c_int = 1 << 9;
    const GDK_KEY_PRESS_MASK: c_int = 1 << 10;
    const GTK_WINDOW_TOPLEVEL: c_int = 0;

    #[repr(C)]
    struct GdkEventButton {
        type_: c_int,
        window: *mut c_void,
        send_event: i8,
        time: u32,
        x: c_double,
        y: c_double,
        axes: *mut c_double,
        state: c_uint,
        button: c_uint,
        device: *mut c_void,
        x_root: c_double,
        y_root: c_double,
    }

    #[repr(C)]
    struct GdkEventMotion {
        type_: c_int,
        window: *mut c_void,
        send_event: i8,
        time: u32,
        x: c_double,
        y: c_double,
        axes: *mut c_double,
        state: c_uint,
        is_hint: i16,
        device: *mut c_void,
        x_root: c_double,
        y_root: c_double,
    }

    type FnXInitThreads = unsafe extern "C" fn() -> c_int;
    type FnGdkVoid = unsafe extern "C" fn();
    type FnGdkSetAllowedBackends = unsafe extern "C" fn(*const c_char);
    type FnGdkX11WindowGetXid = unsafe extern "C" fn(*mut c_void) -> c_ulong;
    type FnGtkInitCheck = unsafe extern "C" fn(*mut c_int, *mut *mut *mut c_char) -> c_int;
    type FnGtkMainLevel = unsafe extern "C" fn() -> c_uint;
    type FnGtkEventsPending = unsafe extern "C" fn() -> c_int;
    type FnGtkMainIterationDo = unsafe extern "C" fn(c_int) -> c_int;
    type FnGtkPlugNew = unsafe extern "C" fn(c_ulong) -> *mut c_void;
    type FnGtkPlugGetId = unsafe extern "C" fn(*mut c_void) -> c_ulong;
    type FnGtkWindowNew = unsafe extern "C" fn(c_int) -> *mut c_void;
    type FnGtkWindowSetTitle = unsafe extern "C" fn(*mut c_void, *const c_char);
    type FnGtkWindowSetDefaultSize = unsafe extern "C" fn(*mut c_void, c_int, c_int);
    type FnGtkWindowResize = unsafe extern "C" fn(*mut c_void, c_int, c_int);
    type FnGtkWindowSetResizable = unsafe extern "C" fn(*mut c_void, c_int);
    type FnGtkContainerAdd = unsafe extern "C" fn(*mut c_void, *mut c_void);
    type FnGtkDrawingAreaNew = unsafe extern "C" fn() -> *mut c_void;
    type FnGtkWidgetSetSizeRequest = unsafe extern "C" fn(*mut c_void, c_int, c_int);
    type FnGtkWidgetAddEvents = unsafe extern "C" fn(*mut c_void, c_int);
    type FnGtkWidgetSetCanFocus = unsafe extern "C" fn(*mut c_void, c_int);
    type FnGtkWidgetRealize = unsafe extern "C" fn(*mut c_void);
    type FnGtkWidgetShowAll = unsafe extern "C" fn(*mut c_void);
    type FnGtkWidgetDestroy = unsafe extern "C" fn(*mut c_void);
    type FnGtkWidgetQueueDraw = unsafe extern "C" fn(*mut c_void);
    type FnGtkWidgetGetWindow = unsafe extern "C" fn(*mut c_void) -> *mut c_void;
    type FnGtkWidgetGetAllocatedWidth = unsafe extern "C" fn(*mut c_void) -> c_int;
    type FnGtkWidgetGetAllocatedHeight = unsafe extern "C" fn(*mut c_void) -> c_int;
    type FnGTimeoutAdd = unsafe extern "C" fn(c_uint, unsafe extern "C" fn(*mut c_void) -> c_int, *mut c_void) -> c_uint;
    type FnGSourceRemove = unsafe extern "C" fn(c_uint) -> c_int;
    type FnGSignalConnectData = unsafe extern "C" fn(
        *mut c_void,
        *const c_char,
        *const c_void,
        *mut c_void,
        Option<unsafe extern "C" fn(*mut c_void, *mut c_void)>,
        c_uint,
    ) -> c_ulong;
    type FnGObjectUnref = unsafe extern "C" fn(*mut c_void);
    type FnGFree = unsafe extern "C" fn(*mut c_void);

    // Cairo
    type FnCairoSetSourceRgb = unsafe extern "C" fn(*mut c_void, c_double, c_double, c_double);
    type FnCairoSetSourceRgba = unsafe extern "C" fn(*mut c_void, c_double, c_double, c_double, c_double);
    type FnCairoRectangle = unsafe extern "C" fn(*mut c_void, c_double, c_double, c_double, c_double);
    type FnCairoFill = unsafe extern "C" fn(*mut c_void);
    type FnCairoStroke = unsafe extern "C" fn(*mut c_void);
    type FnCairoSetLineWidth = unsafe extern "C" fn(*mut c_void, c_double);
    type FnCairoMoveTo = unsafe extern "C" fn(*mut c_void, c_double, c_double);
    type FnCairoLineTo = unsafe extern "C" fn(*mut c_void, c_double, c_double);
    type FnCairoSelectFontFace = unsafe extern "C" fn(*mut c_void, *const c_char, c_int, c_int);
    type FnCairoSetFontSize = unsafe extern "C" fn(*mut c_void, c_double);
    type FnCairoShowText = unsafe extern "C" fn(*mut c_void, *const c_char);

    // WebKit2GTK optional
    type FnWebkitUcmNew = unsafe extern "C" fn() -> *mut c_void;
    type FnWebkitUcmRegisterHandler = unsafe extern "C" fn(*mut c_void, *const c_char) -> c_int;
    type FnWebkitWebViewNewWithUcm = unsafe extern "C" fn(*mut c_void) -> *mut c_void;
    type FnWebkitWebViewLoadUri = unsafe extern "C" fn(*mut c_void, *const c_char);
    type FnWebkitWebViewEvalJs = unsafe extern "C" fn(
        *mut c_void,
        *const c_char,
        isize,
        *const c_char,
        *const c_char,
        *mut c_void,
        *mut c_void,
        *mut c_void,
    );
    type FnWebkitWebViewRunJs = unsafe extern "C" fn(*mut c_void, *const c_char, *mut c_void, *mut c_void, *mut c_void);
    type FnWebkitWebContextGetDefault = unsafe extern "C" fn() -> *mut c_void;
    type FnWebkitWebContextRegisterScheme = unsafe extern "C" fn(
        *mut c_void,
        *const c_char,
        unsafe extern "C" fn(*mut c_void, *mut c_void),
        *mut c_void,
        *mut c_void,
    );
    type FnWebkitUriSchemeRequestGetUri = unsafe extern "C" fn(*mut c_void) -> *const c_char;
    type FnWebkitUriSchemeRequestFinish = unsafe extern "C" fn(*mut c_void, *mut c_void, i64, *const c_char);
    type FnWebkitJsResultGetJsValue = unsafe extern "C" fn(*mut c_void) -> *mut c_void;
    type FnJscValueToString = unsafe extern "C" fn(*mut c_void) -> *mut c_char;
    type FnGMemoryInputStreamNewFromData = unsafe extern "C" fn(
        *const c_void,
        isize,
        Option<unsafe extern "C" fn(*mut c_void)>,
    ) -> *mut c_void;

    struct WebKitSyms {
        _webkit_lib: DynLib,
        _gio_lib: DynLib,
        _jsc_lib: Option<DynLib>,
        ucm_new: FnWebkitUcmNew,
        ucm_register: FnWebkitUcmRegisterHandler,
        webview_new_ucm: FnWebkitWebViewNewWithUcm,
        webview_load_uri: FnWebkitWebViewLoadUri,
        webview_eval_js: Option<FnWebkitWebViewEvalJs>,
        webview_run_js: Option<FnWebkitWebViewRunJs>,
        web_context_get_default: FnWebkitWebContextGetDefault,
        web_context_register_scheme: FnWebkitWebContextRegisterScheme,
        uri_req_get_uri: FnWebkitUriSchemeRequestGetUri,
        uri_req_finish: FnWebkitUriSchemeRequestFinish,
        js_res_get_val: FnWebkitJsResultGetJsValue,
        jsc_to_str: FnJscValueToString,
        mem_stream_new: FnGMemoryInputStreamNewFromData,
    }

    struct GtkSyms {
        _x11: Option<DynLib>,
        _gtk: DynLib,
        _gdk: DynLib,
        _gobject: DynLib,
        _glib: DynLib,
        _cairo: DynLib,
        x_init_threads: Option<FnXInitThreads>,
        gdk_set_allowed_backends: Option<FnGdkSetAllowedBackends>,
        gdk_x11_window_get_xid: Option<FnGdkX11WindowGetXid>,
        gdk_error_trap_push: Option<FnGdkVoid>,
        gdk_error_trap_pop_ignored: Option<FnGdkVoid>,
        gdk_flush: Option<FnGdkVoid>,
        gtk_init_check: FnGtkInitCheck,
        gtk_main_level: FnGtkMainLevel,
        gtk_events_pending: FnGtkEventsPending,
        gtk_main_iteration_do: FnGtkMainIterationDo,
        gtk_plug_new: FnGtkPlugNew,
        gtk_plug_get_id: Option<FnGtkPlugGetId>,
        gtk_window_new: FnGtkWindowNew,
        gtk_window_set_title: FnGtkWindowSetTitle,
        gtk_window_set_default_size: FnGtkWindowSetDefaultSize,
        gtk_window_resize: FnGtkWindowResize,
        gtk_window_set_resizable: Option<FnGtkWindowSetResizable>,
        gtk_container_add: FnGtkContainerAdd,
        gtk_drawing_area_new: FnGtkDrawingAreaNew,
        gtk_widget_set_size_request: FnGtkWidgetSetSizeRequest,
        gtk_widget_add_events: FnGtkWidgetAddEvents,
        gtk_widget_set_can_focus: FnGtkWidgetSetCanFocus,
        gtk_widget_realize: FnGtkWidgetRealize,
        gtk_widget_show_all: FnGtkWidgetShowAll,
        gtk_widget_destroy: FnGtkWidgetDestroy,
        gtk_widget_queue_draw: FnGtkWidgetQueueDraw,
        gtk_widget_get_window: FnGtkWidgetGetWindow,
        gtk_widget_get_allocated_width: FnGtkWidgetGetAllocatedWidth,
        gtk_widget_get_allocated_height: FnGtkWidgetGetAllocatedHeight,
        g_timeout_add: FnGTimeoutAdd,
        g_source_remove: FnGSourceRemove,
        g_signal_connect_data: FnGSignalConnectData,
        g_object_unref: FnGObjectUnref,
        g_free: FnGFree,
        cairo_set_source_rgb: FnCairoSetSourceRgb,
        cairo_set_source_rgba: FnCairoSetSourceRgba,
        cairo_rectangle: FnCairoRectangle,
        cairo_fill: FnCairoFill,
        cairo_stroke: FnCairoStroke,
        cairo_set_line_width: FnCairoSetLineWidth,
        cairo_move_to: FnCairoMoveTo,
        cairo_line_to: FnCairoLineTo,
        cairo_select_font_face: FnCairoSelectFontFace,
        cairo_set_font_size: FnCairoSetFontSize,
        cairo_show_text: FnCairoShowText,
        webkit: Option<WebKitSyms>,
    }

    struct GdkErrorTrap<'a>(&'a GtkSyms);

    impl<'a> GdkErrorTrap<'a> {
        unsafe fn new(s: &'a GtkSyms) -> Self {
            if let Some(push) = s.gdk_error_trap_push {
                push();
            }
            Self(s)
        }
    }

    impl Drop for GdkErrorTrap<'_> {
        fn drop(&mut self) {
            unsafe {
                if let Some(flush) = self.0.gdk_flush {
                    flush();
                }
                if let Some(pop) = self.0.gdk_error_trap_pop_ignored {
                    pop();
                }
            }
        }
    }

    unsafe impl Send for GtkSyms {}
    unsafe impl Sync for GtkSyms {}

    unsafe fn sym<T: Copy>(lib: &DynLib, name: &[u8]) -> Result<T, EmbedError> {
        debug_assert_eq!(core::mem::size_of::<T>(), core::mem::size_of::<*mut c_void>());
        let ptr = lib.get_raw(name).ok_or_else(|| EmbedError {
            message: format!("Missing symbol {}", String::from_utf8_lossy(name)),
        })?;
        Ok(core::mem::transmute_copy(&ptr))
    }

    unsafe fn opt_sym<T: Copy>(lib: &DynLib, name: &[u8]) -> Option<T> {
        debug_assert_eq!(core::mem::size_of::<T>(), core::mem::size_of::<*mut c_void>());
        lib.get_raw(name).map(|ptr| core::mem::transmute_copy(&ptr))
    }

    fn load_webkit() -> Option<WebKitSyms> {
        if std::env::var_os("PROPHET_FORCE_NATIVE_UI").is_some() {
            return None;
        }
        unsafe {
            let webkit_lib = DynLib::open("libwebkit2gtk-4.1.so.0")
                .or_else(|_| DynLib::open("libwebkit2gtk-4.0.so.0"))
                .ok()?;
            let gio_lib = DynLib::open("libgio-2.0.so.0").ok()?;
            let jsc_lib = DynLib::open("libjavascriptcoregtk-4.1.so.0")
                .or_else(|_| DynLib::open("libjavascriptcoregtk-4.0.so.0"))
                .ok();
            let ucm_new = opt_sym(&webkit_lib, b"webkit_user_content_manager_new\0")?;
            let ucm_register = opt_sym(
                &webkit_lib,
                b"webkit_user_content_manager_register_script_message_handler\0",
            )?;
            let webview_new_ucm =
                opt_sym(&webkit_lib, b"webkit_web_view_new_with_user_content_manager\0")?;
            let webview_load_uri = opt_sym(&webkit_lib, b"webkit_web_view_load_uri\0")?;
            let webview_eval_js = opt_sym(&webkit_lib, b"webkit_web_view_evaluate_javascript\0");
            let webview_run_js = opt_sym(&webkit_lib, b"webkit_web_view_run_javascript\0");
            if webview_eval_js.is_none() && webview_run_js.is_none() {
                return None;
            }
            let web_context_get_default =
                opt_sym(&webkit_lib, b"webkit_web_context_get_default\0")?;
            let web_context_register_scheme =
                opt_sym(&webkit_lib, b"webkit_web_context_register_uri_scheme\0")?;
            let uri_req_get_uri = opt_sym(&webkit_lib, b"webkit_uri_scheme_request_get_uri\0")?;
            let uri_req_finish = opt_sym(&webkit_lib, b"webkit_uri_scheme_request_finish\0")?;
            let js_res_get_val = opt_sym(&webkit_lib, b"webkit_javascript_result_get_js_value\0")?;
            let jsc_to_str = opt_sym(&webkit_lib, b"jsc_value_to_string\0").or_else(|| {
                jsc_lib
                    .as_ref()
                    .and_then(|j| opt_sym(j, b"jsc_value_to_string\0"))
            })?;
            let mem_stream_new = opt_sym(&gio_lib, b"g_memory_input_stream_new_from_data\0")?;
            Some(WebKitSyms {
                _webkit_lib: webkit_lib,
                _gio_lib: gio_lib,
                _jsc_lib: jsc_lib,
                ucm_new,
                ucm_register,
                webview_new_ucm,
                webview_load_uri,
                webview_eval_js,
                webview_run_js,
                web_context_get_default,
                web_context_register_scheme,
                uri_req_get_uri,
                uri_req_finish,
                js_res_get_val,
                jsc_to_str,
                mem_stream_new,
            })
        }
    }

    fn syms() -> Result<&'static GtkSyms, EmbedError> {
        static CACHE: OnceLock<Result<GtkSyms, String>> = OnceLock::new();
        let res = CACHE.get_or_init(|| unsafe {
            let x11 = DynLib::open("libX11.so.6").ok();
            let gtk = DynLib::open("libgtk-3.so.0")
                .map_err(|e| format!("Failed loading libgtk-3.so.0: {e}"))?;
            let gdk = DynLib::open("libgdk-3.so.0")
                .map_err(|e| format!("Failed loading libgdk-3.so.0: {e}"))?;
            let gobject = DynLib::open("libgobject-2.0.so.0")
                .map_err(|e| format!("Failed loading libgobject-2.0.so.0: {e}"))?;
            let glib = DynLib::open("libglib-2.0.so.0")
                .map_err(|e| format!("Failed loading libglib-2.0.so.0: {e}"))?;
            let cairo = DynLib::open("libcairo.so.2")
                .map_err(|e| format!("Failed loading libcairo.so.2: {e}"))?;

            Ok(GtkSyms {
                x_init_threads: x11.as_ref().and_then(|l| opt_sym(l, b"XInitThreads\0")),
                _x11: x11,
                gdk_set_allowed_backends: opt_sym(&gdk, b"gdk_set_allowed_backends\0"),
                gdk_x11_window_get_xid: opt_sym(&gdk, b"gdk_x11_window_get_xid\0"),
                gdk_error_trap_push: opt_sym(&gdk, b"gdk_error_trap_push\0"),
                gdk_error_trap_pop_ignored: opt_sym(&gdk, b"gdk_error_trap_pop_ignored\0"),
                gdk_flush: opt_sym(&gdk, b"gdk_flush\0"),
                gtk_init_check: sym(&gtk, b"gtk_init_check\0").map_err(|e| e.message)?,
                gtk_main_level: sym(&gtk, b"gtk_main_level\0").map_err(|e| e.message)?,
                gtk_events_pending: sym(&gtk, b"gtk_events_pending\0").map_err(|e| e.message)?,
                gtk_main_iteration_do: sym(&gtk, b"gtk_main_iteration_do\0")
                    .map_err(|e| e.message)?,
                gtk_plug_new: sym(&gtk, b"gtk_plug_new\0").map_err(|e| e.message)?,
                gtk_plug_get_id: opt_sym(&gtk, b"gtk_plug_get_id\0"),
                gtk_window_new: sym(&gtk, b"gtk_window_new\0").map_err(|e| e.message)?,
                gtk_window_set_title: sym(&gtk, b"gtk_window_set_title\0")
                    .map_err(|e| e.message)?,
                gtk_window_set_default_size: sym(&gtk, b"gtk_window_set_default_size\0")
                    .map_err(|e| e.message)?,
                gtk_window_resize: sym(&gtk, b"gtk_window_resize\0").map_err(|e| e.message)?,
                gtk_window_set_resizable: opt_sym(&gtk, b"gtk_window_set_resizable\0"),
                gtk_container_add: sym(&gtk, b"gtk_container_add\0").map_err(|e| e.message)?,
                gtk_drawing_area_new: sym(&gtk, b"gtk_drawing_area_new\0")
                    .map_err(|e| e.message)?,
                gtk_widget_set_size_request: sym(&gtk, b"gtk_widget_set_size_request\0")
                    .map_err(|e| e.message)?,
                gtk_widget_add_events: sym(&gtk, b"gtk_widget_add_events\0")
                    .map_err(|e| e.message)?,
                gtk_widget_set_can_focus: sym(&gtk, b"gtk_widget_set_can_focus\0")
                    .map_err(|e| e.message)?,
                gtk_widget_realize: sym(&gtk, b"gtk_widget_realize\0").map_err(|e| e.message)?,
                gtk_widget_show_all: sym(&gtk, b"gtk_widget_show_all\0").map_err(|e| e.message)?,
                gtk_widget_destroy: sym(&gtk, b"gtk_widget_destroy\0").map_err(|e| e.message)?,
                gtk_widget_queue_draw: sym(&gtk, b"gtk_widget_queue_draw\0")
                    .map_err(|e| e.message)?,
                gtk_widget_get_window: sym(&gtk, b"gtk_widget_get_window\0")
                    .map_err(|e| e.message)?,
                gtk_widget_get_allocated_width: sym(&gtk, b"gtk_widget_get_allocated_width\0")
                    .map_err(|e| e.message)?,
                gtk_widget_get_allocated_height: sym(&gtk, b"gtk_widget_get_allocated_height\0")
                    .map_err(|e| e.message)?,
                g_timeout_add: sym(&glib, b"g_timeout_add\0").map_err(|e| e.message)?,
                g_source_remove: sym(&glib, b"g_source_remove\0").map_err(|e| e.message)?,
                g_free: sym(&glib, b"g_free\0").map_err(|e| e.message)?,
                g_signal_connect_data: sym(&gobject, b"g_signal_connect_data\0")
                    .map_err(|e| e.message)?,
                g_object_unref: sym(&gobject, b"g_object_unref\0").map_err(|e| e.message)?,
                cairo_set_source_rgb: sym(&cairo, b"cairo_set_source_rgb\0")
                    .map_err(|e| e.message)?,
                cairo_set_source_rgba: sym(&cairo, b"cairo_set_source_rgba\0")
                    .map_err(|e| e.message)?,
                cairo_rectangle: sym(&cairo, b"cairo_rectangle\0").map_err(|e| e.message)?,
                cairo_fill: sym(&cairo, b"cairo_fill\0").map_err(|e| e.message)?,
                cairo_stroke: sym(&cairo, b"cairo_stroke\0").map_err(|e| e.message)?,
                cairo_set_line_width: sym(&cairo, b"cairo_set_line_width\0")
                    .map_err(|e| e.message)?,
                cairo_move_to: sym(&cairo, b"cairo_move_to\0").map_err(|e| e.message)?,
                cairo_line_to: sym(&cairo, b"cairo_line_to\0").map_err(|e| e.message)?,
                cairo_select_font_face: sym(&cairo, b"cairo_select_font_face\0")
                    .map_err(|e| e.message)?,
                cairo_set_font_size: sym(&cairo, b"cairo_set_font_size\0")
                    .map_err(|e| e.message)?,
                cairo_show_text: sym(&cairo, b"cairo_show_text\0").map_err(|e| e.message)?,
                _gtk: gtk,
                _gdk: gdk,
                _gobject: gobject,
                _glib: glib,
                _cairo: cairo,
                webkit: load_webkit(),
            })
        });
        match res {
            Ok(s) => Ok(s),
            Err(msg) => Err(EmbedError {
                message: msg.clone(),
            }),
        }
    }

    fn ensure_gtk_x11_init(s: &GtkSyms) -> Result<(), EmbedError> {
        static GTK_INIT_OK: OnceLock<bool> = OnceLock::new();
        let ok = *GTK_INIT_OK.get_or_init(|| unsafe {
            if let Some(xinit) = s.x_init_threads {
                xinit();
            }
            if let Some(allow) = s.gdk_set_allowed_backends {
                allow(c"x11".as_ptr());
            }
            (s.gtk_init_check)(core::ptr::null_mut(), core::ptr::null_mut()) != 0
        });
        if ok {
            Ok(())
        } else {
            Err(EmbedError {
                message: "gtk_init_check failed (no X11 DISPLAY available)".to_owned(),
            })
        }
    }

    #[derive(Clone)]
    struct InteractiveParamZone {
        param_id: u16,
        min: f32,
        max: f32,
        x0: f64,
        y0: f64,
        w: f64,
        h: f64,
    }

    struct GtkViewState {
        _id: u64,
        sink: Arc<dyn HostBridgeSink>,
        backend_tag: String,
        root_widget: *mut c_void,
        content_widget: *mut c_void,
        webview_widget: *mut c_void,
        timer_id: AtomicU32,
        alive: AtomicBool,
        child_xid: AtomicU32,
        drag_param: Mutex<Option<InteractiveParamZone>>,
        held_key: Mutex<Option<u8>>,
        last_snapshot: Mutex<Option<crate::UiSnapshot>>,
    }

    unsafe impl Send for GtkViewState {}
    unsafe impl Sync for GtkViewState {}

    unsafe extern "C" fn drop_view_arc(data: *mut c_void, _closure: *mut c_void) {
        if !data.is_null() {
            drop(Arc::from_raw(data as *const GtkViewState));
        }
    }

    unsafe extern "C" fn on_widget_destroy(
        _widget: *mut c_void,
        user_data: *mut c_void,
    ) {
        if user_data.is_null() {
            return;
        }
        let view = &*(user_data as *const GtkViewState);
        view.alive.store(false, Ordering::Release);
        let tag = view.timer_id.swap(0, Ordering::AcqRel);
        if tag != 0 {
            if let Ok(s) = syms() {
                (s.g_source_remove)(tag);
            }
        }
    }

    unsafe extern "C" fn scheme_request_cb(request: *mut c_void, _user_data: *mut c_void) {
        let Ok(s) = syms() else { return };
        let Some(ref wk) = s.webkit else { return };
        let uri_ptr = (wk.uri_req_get_uri)(request);
        let uri = if uri_ptr.is_null() {
            "/"
        } else {
            CStr::from_ptr(uri_ptr).to_str().unwrap_or("/")
        };
        let path = uri
            .strip_prefix("prophet://app")
            .or_else(|| uri.strip_prefix("prophet://localhost"))
            .unwrap_or(uri);
        if let Some(asset) = resolve_embedded_asset(path) {
            let payload_vec: Vec<u8> = if asset.path == "/index.html" {
                inject_bridge_into_html(
                    asset.bytes,
                    &native_bridge_bootstrap_script(
                        "embedded-webkit2gtk",
                        BridgeTransportMode::WebKitMessageHandler,
                    ),
                )
            } else {
                asset.bytes.to_vec()
            };
            let exact_len = payload_vec.len();
            let mem = libc::malloc(exact_len.max(1));
            if !mem.is_null() {
                core::ptr::copy_nonoverlapping(payload_vec.as_ptr(), mem as *mut u8, exact_len);
                let stream = (wk.mem_stream_new)(mem, exact_len as isize, Some(libc::free));
                let c_mime = CString::new(asset.mime)
                    .unwrap_or_else(|_| CString::new("application/octet-stream").unwrap());
                (wk.uri_req_finish)(request, stream, exact_len as i64, c_mime.as_ptr());
                (s.g_object_unref)(stream);
            }
        }
    }

    #[derive(serde::Deserialize)]
    struct JsEnvelope {
        #[serde(default)]
        seq: u64,
        cmd: serde_json::Value,
    }

    unsafe extern "C" fn on_script_message(
        _ucm: *mut c_void,
        js_result: *mut c_void,
        user_data: *mut c_void,
    ) {
        if user_data.is_null() || js_result.is_null() {
            return;
        }
        let Ok(s) = syms() else { return };
        let Some(ref wk) = s.webkit else { return };
        let view = &*(user_data as *const GtkViewState);
        if !view.alive.load(Ordering::Acquire) {
            return;
        }
        let jsc_val = (wk.js_res_get_val)(js_result);
        if jsc_val.is_null() {
            return;
        }
        let cstr_ptr = (wk.jsc_to_str)(jsc_val);
        if cstr_ptr.is_null() {
            return;
        }
        let text = CStr::from_ptr(cstr_ptr).to_string_lossy().into_owned();
        (s.g_free)(cstr_ptr as *mut c_void);

        let Ok(env) = serde_json::from_str::<JsEnvelope>(&text) else {
            return;
        };
        if env.cmd.get("type").and_then(|v| v.as_str()) == Some("__snapshot__") {
            let snap = view.sink.snapshot_json();
            push_snapshot_to_webview(s, wk, view.webview_widget, &snap);
            return;
        }
        let Ok(cmd) = serde_json::from_value::<UiCommand>(env.cmd) else {
            return;
        };
        let reply = view.sink.dispatch_command(cmd);
        if env.seq > 0 {
            if let Ok(r_json) = serde_json::to_string(&reply) {
                let script = format!(
                    "window.__PROPHET_HOST_REPLY__ && window.__PROPHET_HOST_REPLY__({}, {});",
                    env.seq,
                    serde_json::to_string(&r_json).unwrap_or_else(|_| "\"{}\"".to_owned())
                );
                eval_js(wk, view.webview_widget, &script);
            }
        }
    }

    unsafe fn eval_js(wk: &WebKitSyms, webview: *mut c_void, js: &str) {
        if webview.is_null() {
            return;
        }
        if let Ok(c_js) = CString::new(js) {
            if let Some(f) = wk.webview_eval_js {
                f(
                    webview,
                    c_js.as_ptr(),
                    js.len() as isize,
                    core::ptr::null(),
                    core::ptr::null(),
                    core::ptr::null_mut(),
                    core::ptr::null_mut(),
                    core::ptr::null_mut(),
                );
            } else if let Some(f) = wk.webview_run_js {
                f(
                    webview,
                    c_js.as_ptr(),
                    core::ptr::null_mut(),
                    core::ptr::null_mut(),
                    core::ptr::null_mut(),
                );
            }
        }
    }

    unsafe fn push_snapshot_to_webview(
        _s: &GtkSyms,
        wk: &WebKitSyms,
        webview: *mut c_void,
        snapshot_json: &str,
    ) {
        if let Ok(quoted) = serde_json::to_string(snapshot_json) {
            let script = format!(
                "window.__PROPHET_HOST_RECEIVE__ && window.__PROPHET_HOST_RECEIVE__({quoted});"
            );
            eval_js(wk, webview, &script);
        }
    }

    unsafe extern "C" fn on_timeout_tick(user_data: *mut c_void) -> c_int {
        if user_data.is_null() {
            return 0;
        }
        let view = &*(user_data as *const GtkViewState);
        if !view.alive.load(Ordering::Acquire) {
            view.timer_id.store(0, Ordering::Release);
            return 0;
        }
        let Ok(s) = syms() else {
            return 0;
        };
        if !view.webview_widget.is_null() {
            if let Some(ref wk) = s.webkit {
                let snap = view.sink.snapshot_json();
                push_snapshot_to_webview(s, wk, view.webview_widget, &snap);
            }
        } else if !view.content_widget.is_null() {
            let snap = view.sink.capture_ui_state();
            if let Ok(mut guard) = view.last_snapshot.lock() {
                *guard = Some(snap);
            }
            (s.gtk_widget_queue_draw)(view.content_widget);
        }
        1
    }

    fn slider_zones(w: f64, h: f64) -> [InteractiveParamZone; 6] {
        let card_y = (h * 0.24).max(115.0);
        let card_h = (h * 0.28).max(140.0);
        let pad = 20.0;
        let gap = 14.0;
        let col_w = ((w - pad * 2.0 - gap * 5.0) / 6.0).max(80.0);
        let defs = [
            (ParamId::FilterCutoff as u16, 20.0, 18000.0),
            (ParamId::FilterResonance as u16, 0.0, 1.0),
            (ParamId::FilterEnvAmt as u16, -1.0, 1.0),
            (ParamId::GlideTime as u16, 0.0, 5.0),
            (ParamId::UnisonDetune as u16, 0.0, 60.0),
            (ParamId::ReverbWet as u16, 0.0, 1.0),
        ];
        core::array::from_fn(|i| InteractiveParamZone {
            param_id: defs[i].0,
            min: defs[i].1,
            max: defs[i].2,
            x0: pad + (i as f64) * (col_w + gap),
            y0: card_y,
            w: col_w,
            h: card_h,
        })
    }

    fn key_rect(w: f64, h: f64) -> (f64, f64, f64, f64) {
        (20.0, (h - 118.0).max(10.0), (w - 40.0).max(200.0), 92.0)
    }

    unsafe extern "C" fn on_cairo_draw(
        widget: *mut c_void,
        cr: *mut c_void,
        user_data: *mut c_void,
    ) -> c_int {
        if user_data.is_null() || cr.is_null() {
            return 0;
        }
        let Ok(s) = syms() else { return 0 };
        let view = &*(user_data as *const GtkViewState);
        if !view.alive.load(Ordering::Acquire) {
            return 0;
        }
        let w = (s.gtk_widget_get_allocated_width)(widget).max(320) as f64;
        let h = (s.gtk_widget_get_allocated_height)(widget).max(240) as f64;

        let snap = view
            .last_snapshot
            .lock()
            .ok()
            .and_then(|g| g.clone())
            .unwrap_or_else(|| view.sink.capture_ui_state());

        // Deep charcoal oak/anodized vintage chassis
        (s.cairo_set_source_rgb)(cr, 0.082, 0.075, 0.071);
        (s.cairo_rectangle)(cr, 0.0, 0.0, w, h);
        (s.cairo_fill)(cr);

        // Side wooden cheeks
        (s.cairo_set_source_rgb)(cr, 0.24, 0.13, 0.07);
        (s.cairo_rectangle)(cr, 0.0, 0.0, 12.0, h);
        (s.cairo_fill)(cr);
        (s.cairo_rectangle)(cr, w - 12.0, 0.0, 12.0, h);
        (s.cairo_fill)(cr);

        // Top faceplate bar
        (s.cairo_set_source_rgb)(cr, 0.13, 0.12, 0.11);
        (s.cairo_rectangle)(cr, 18.0, 14.0, w - 36.0, 74.0);
        (s.cairo_fill)(cr);
        (s.cairo_set_source_rgb)(cr, 0.85, 0.48, 0.16);
        (s.cairo_set_line_width)(cr, 1.5);
        (s.cairo_rectangle)(cr, 18.0, 14.0, w - 36.0, 74.0);
        (s.cairo_stroke)(cr);

        // Header title & live badges
        let c_sans = c"Sans";
        (s.cairo_select_font_face)(cr, c_sans.as_ptr(), 0, 1);
        (s.cairo_set_source_rgb)(cr, 0.96, 0.84, 0.62);
        (s.cairo_set_font_size)(cr, 20.0);
        (s.cairo_move_to)(cr, 32.0, 44.0);
        if let Ok(title) = CString::new(format!(
            "PROPHET-XII  |  {}  |  {}x OS ({}smp)  |  {}",
            view.backend_tag.to_uppercase(),
            snap.parameters.get(ParamId::OversamplingMode as usize).copied().unwrap_or(1.0) as u8,
            snap.engine_port.latency_samples,
            match snap.patch.topology {
                crate::UiTopology::Classic2 => "Classic2",
                crate::UiTopology::Extended3 => "Extended3",
                crate::UiTopology::Extended4 => "Extended4",
                crate::UiTopology::Vector4 => "Vector4",
            },
        )) {
            (s.cairo_show_text)(cr, title.as_ptr());
        }

        (s.cairo_select_font_face)(cr, c_sans.as_ptr(), 0, 0);
        (s.cairo_set_font_size)(cr, 13.0);
        (s.cairo_set_source_rgb)(cr, 0.78, 0.75, 0.70);
        (s.cairo_move_to)(cr, 32.0, 70.0);
        if let Ok(sub) = CString::new(format!(
            "GTK3+Cairo Embedded Panel (WebKit2GTK fallback)  •  SR {}Hz  •  Reverb: {:?} ({})  •  Voices Active: {}",
            snap.engine_port.sample_rate,
            snap.engine_port.fx.algorithm,
            if snap.reverb_bypass_effective { "ON" } else { "BYPASS" },
            snap.voices_count_display(&snap),
        )) {
            (s.cairo_show_text)(cr, sub.as_ptr());
        }

        // Action pills top right: REVERB TOGGLE & PANIC
        let rev_on = snap.reverb_bypass_effective;
        (s.cairo_set_source_rgb)(cr, if rev_on { 0.16 } else { 0.35 }, if rev_on { 0.42 } else { 0.18 }, 0.22);
        (s.cairo_rectangle)(cr, w - 260.0, 28.0, 110.0, 32.0);
        (s.cairo_fill)(cr);
        (s.cairo_set_source_rgb)(cr, 0.96, 0.95, 0.92);
        (s.cairo_move_to)(cr, w - 246.0, 49.0);
        let lbl_rev = CString::new(if rev_on { "REV: ON" } else { "REV: BYP" }).unwrap();
        (s.cairo_show_text)(cr, lbl_rev.as_ptr());

        (s.cairo_set_source_rgb)(cr, 0.55, 0.16, 0.14);
        (s.cairo_rectangle)(cr, w - 138.0, 28.0, 102.0, 32.0);
        (s.cairo_fill)(cr);
        (s.cairo_set_source_rgb)(cr, 0.98, 0.94, 0.92);
        (s.cairo_move_to)(cr, w - 114.0, 49.0);
        (s.cairo_show_text)(cr, c"PANIC".as_ptr());

        // Interactive parameter faders
        let zones = slider_zones(w, h);
        let names = ["CUTOFF", "RESONANCE", "FILT ENV", "GLIDE", "DETUNE", "REVERB"];
        for (i, z) in zones.iter().enumerate() {
            (s.cairo_set_source_rgb)(cr, 0.12, 0.11, 0.105);
            (s.cairo_rectangle)(cr, z.x0, z.y0, z.w, z.h);
            (s.cairo_fill)(cr);
            (s.cairo_set_source_rgb)(cr, 0.32, 0.28, 0.24);
            (s.cairo_set_line_width)(cr, 1.0);
            (s.cairo_rectangle)(cr, z.x0, z.y0, z.w, z.h);
            (s.cairo_stroke)(cr);

            let val = snap
                .parameters
                .get(usize::from(z.param_id))
                .copied()
                .unwrap_or(z.min);
            let norm = ((val - z.min) / (z.max - z.min).max(1e-6)).clamp(0.0, 1.0) as f64;
            let bar_h = (z.h - 52.0).max(20.0);
            let bar_y = z.y0 + 14.0;
            let fill_h = bar_h * norm;
            (s.cairo_set_source_rgb)(cr, 0.85, 0.45, 0.14);
            (s.cairo_rectangle)(cr, z.x0 + 18.0, bar_y + (bar_h - fill_h), z.w - 36.0, fill_h);
            (s.cairo_fill)(cr);

            (s.cairo_set_source_rgb)(cr, 0.90, 0.87, 0.82);
            (s.cairo_set_font_size)(cr, 12.0);
            (s.cairo_move_to)(cr, z.x0 + 10.0, z.y0 + z.h - 22.0);
            if let Ok(nm) = CString::new(names[i]) {
                (s.cairo_show_text)(cr, nm.as_ptr());
            }
            (s.cairo_set_source_rgb)(cr, 0.72, 0.68, 0.60);
            (s.cairo_move_to)(cr, z.x0 + 10.0, z.y0 + z.h - 6.0);
            if let Ok(vt) = CString::new(format!("{val:.2}")) {
                (s.cairo_show_text)(cr, vt.as_ptr());
            }
        }

        // Live scope strip between sliders and keyboard
        let sy = zones[0].y0 + zones[0].h + 18.0;
        let sh = ((h - 134.0) - sy).max(40.0);
        (s.cairo_set_source_rgb)(cr, 0.05, 0.06, 0.055);
        (s.cairo_rectangle)(cr, 20.0, sy, w - 40.0, sh);
        (s.cairo_fill)(cr);
        (s.cairo_set_source_rgb)(cr, 0.25, 0.82, 0.48);
        (s.cairo_set_line_width)(cr, 1.5);
        if !snap.scope.is_empty() {
            let samples = snap.scope.len().min(256);
            for i in 0..samples {
                let x = 22.0 + (w - 44.0) * (i as f64) / ((samples - 1).max(1) as f64);
                let sample = snap.scope[snap.scope.len() - samples + i].clamp(-1.0, 1.0) as f64;
                let y = sy + sh * 0.5 - sample * (sh * 0.42);
                if i == 0 {
                    (s.cairo_move_to)(cr, x, y);
                } else {
                    (s.cairo_line_to)(cr, x, y);
                }
            }
            (s.cairo_stroke)(cr);
        }

        // Clickable 2-octave performance keyboard (C3..B4) highlighting active EnginePort notes
        let (kx, ky, kw, kh) = key_rect(w, h);
        let white_semis = [0u8, 2, 4, 5, 7, 9, 11];
        let num_white = 14usize;
        let ww = kw / (num_white as f64);
        for wi in 0..num_white {
            let note = 48u8 + ((wi / 7) as u8) * 12 + white_semis[wi % 7];
            let active = snap.engine_port.notes.iter().any(|n| n.key == note);
            if active {
                (s.cairo_set_source_rgb)(cr, 0.92, 0.56, 0.20);
            } else {
                (s.cairo_set_source_rgb)(cr, 0.93, 0.91, 0.87);
            }
            (s.cairo_rectangle)(cr, kx + (wi as f64) * ww + 1.0, ky, ww - 2.0, kh);
            (s.cairo_fill)(cr);
            (s.cairo_set_source_rgb)(cr, 0.15, 0.15, 0.15);
            (s.cairo_rectangle)(cr, kx + (wi as f64) * ww + 1.0, ky, ww - 2.0, kh);
            (s.cairo_stroke)(cr);
        }
        let black_offsets: [(usize, u8); 10] = [
            (0, 49), (1, 51), (3, 54), (4, 56), (5, 58),
            (7, 61), (8, 63), (10, 66), (11, 68), (12, 70),
        ];
        let bw = ww * 0.62;
        for (wi, note) in black_offsets {
            let bx = kx + ((wi as f64) + 0.68) * ww;
            let active = snap.engine_port.notes.iter().any(|n| n.key == note);
            if active {
                (s.cairo_set_source_rgba)(cr, 0.95, 0.42, 0.12, 1.0);
            } else {
                (s.cairo_set_source_rgba)(cr, 0.09, 0.09, 0.09, 1.0);
            }
            (s.cairo_rectangle)(cr, bx, ky, bw, kh * 0.62);
            (s.cairo_fill)(cr);
        }

        1
    }

    trait SnapshotVoiceExt {
        fn voices_count_display(&self, snap: &crate::UiSnapshot) -> usize;
    }
    impl SnapshotVoiceExt for crate::UiSnapshot {
        fn voices_count_display(&self, snap: &crate::UiSnapshot) -> usize {
            snap.engine_port.voices.len()
        }
    }

    unsafe extern "C" fn on_cairo_button_press(
        widget: *mut c_void,
        ev: *const GdkEventButton,
        user_data: *mut c_void,
    ) -> c_int {
        if user_data.is_null() || ev.is_null() {
            return 0;
        }
        let Ok(s) = syms() else { return 0 };
        let view = &*(user_data as *const GtkViewState);
        if !view.alive.load(Ordering::Acquire) {
            return 0;
        }
        let x = (*ev).x;
        let y = (*ev).y;
        let w = (s.gtk_widget_get_allocated_width)(widget).max(320) as f64;
        let h = (s.gtk_widget_get_allocated_height)(widget).max(240) as f64;

        // Header action buttons
        if y >= 28.0 && y <= 60.0 {
            if x >= w - 260.0 && x <= w - 150.0 {
                let cur = view.sink.capture_ui_state().reverb_bypass_effective;
                let _ = view.sink.dispatch_command(UiCommand::SetReverb {
                    algorithm: None,
                    enabled: Some(!cur),
                    tail: None,
                });
                if let Ok(mut guard) = view.last_snapshot.lock() {
                    *guard = Some(view.sink.capture_ui_state());
                }
                (s.gtk_widget_queue_draw)(widget);
                return 1;
            }
            if x >= w - 138.0 && x <= w - 36.0 {
                let _ = view.sink.dispatch_command(UiCommand::Panic { all_origins: true });
                if let Ok(mut guard) = view.last_snapshot.lock() {
                    *guard = Some(view.sink.capture_ui_state());
                }
                (s.gtk_widget_queue_draw)(widget);
                return 1;
            }
        }

        // Sliders
        for z in slider_zones(w, h) {
            if x >= z.x0 && x <= z.x0 + z.w && y >= z.y0 && y <= z.y0 + z.h {
                let bar_h = (z.h - 52.0).max(20.0);
                let norm = (1.0 - ((y - (z.y0 + 14.0)) / bar_h)).clamp(0.0, 1.0) as f32;
                let plain = z.min + norm * (z.max - z.min);
                let _ = view.sink.dispatch_command(UiCommand::Param {
                    id: z.param_id,
                    value: plain,
                });
                if let Ok(mut d) = view.drag_param.lock() {
                    *d = Some(z);
                }
                if let Ok(mut guard) = view.last_snapshot.lock() {
                    *guard = Some(view.sink.capture_ui_state());
                }
                (s.gtk_widget_queue_draw)(widget);
                return 1;
            }
        }

        // Keyboard
        let (kx, ky, kw, kh) = key_rect(w, h);
        if x >= kx && x <= kx + kw && y >= ky && y <= ky + kh {
            let num_white = 14usize;
            let ww = kw / (num_white as f64);
            let wi = (((x - kx) / ww) as usize).min(num_white - 1);
            let white_semis = [0u8, 2, 4, 5, 7, 9, 11];
            let note = 48u8 + ((wi / 7) as u8) * 12 + white_semis[wi % 7];
            if let Ok(mut hk) = view.held_key.lock() {
                if let Some(prev) = hk.take() {
                    let _ = view.sink.dispatch_command(UiCommand::Midi {
                        status: 0x80,
                        data1: prev,
                        data2: 0,
                        note_id: Some(u32::from(prev)),
                    });
                }
                *hk = Some(note);
            }
            let _ = view.sink.dispatch_command(UiCommand::Midi {
                status: 0x90,
                data1: note,
                data2: 100,
                note_id: Some(u32::from(note)),
            });
            if let Ok(mut guard) = view.last_snapshot.lock() {
                *guard = Some(view.sink.capture_ui_state());
            }
            (s.gtk_widget_queue_draw)(widget);
            return 1;
        }
        0
    }

    unsafe extern "C" fn on_cairo_button_release(
        widget: *mut c_void,
        _ev: *const GdkEventButton,
        user_data: *mut c_void,
    ) -> c_int {
        if user_data.is_null() {
            return 0;
        }
        let view = &*(user_data as *const GtkViewState);
        if let Ok(mut d) = view.drag_param.lock() {
            *d = None;
        }
        if let Ok(mut hk) = view.held_key.lock() {
            if let Some(prev) = hk.take() {
                let _ = view.sink.dispatch_command(UiCommand::Midi {
                    status: 0x80,
                    data1: prev,
                    data2: 0,
                    note_id: Some(u32::from(prev)),
                });
            }
        }
        if let Ok(s) = syms() {
            if let Ok(mut guard) = view.last_snapshot.lock() {
                *guard = Some(view.sink.capture_ui_state());
            }
            (s.gtk_widget_queue_draw)(widget);
        }
        1
    }

    unsafe extern "C" fn on_cairo_motion(
        widget: *mut c_void,
        ev: *const GdkEventMotion,
        user_data: *mut c_void,
    ) -> c_int {
        if user_data.is_null() || ev.is_null() {
            return 0;
        }
        let Ok(s) = syms() else { return 0 };
        let view = &*(user_data as *const GtkViewState);
        let zone = view.drag_param.lock().ok().and_then(|g| g.clone());
        let Some(z) = zone else { return 0 };
        let _w = (s.gtk_widget_get_allocated_width)(widget);
        let bar_h = (z.h - 52.0).max(20.0);
        let norm = (1.0 - (((*ev).y - (z.y0 + 14.0)) / bar_h)).clamp(0.0, 1.0) as f32;
        let plain = z.min + norm * (z.max - z.min);
        let _ = view.sink.dispatch_command(UiCommand::Param {
            id: z.param_id,
            value: plain,
        });
        if let Ok(mut guard) = view.last_snapshot.lock() {
            *guard = Some(view.sink.capture_ui_state());
        }
        (s.gtk_widget_queue_draw)(widget);
        1
    }

    unsafe fn build_gtk_widgets(
        s: &GtkSyms,
        parent: ParentWindow,
        config: &EditorViewConfig,
        sink: Arc<dyn HostBridgeSink>,
        view_id: u64,
    ) -> Result<(Arc<GtkViewState>, &'static str), EmbedError> {
        ensure_gtk_x11_init(s)?;
        let _trap = GdkErrorTrap::new(s);
        let phys_w = ((config.logical_width as f32) * config.scale_factor.max(0.5)).round() as c_int;
        let phys_h = ((config.logical_height as f32) * config.scale_factor.max(0.5)).round() as c_int;

        let root = match parent {
            ParentWindow::X11(xid) => (s.gtk_plug_new)(c_ulong::from(xid)),
            ParentWindow::StandaloneWindow => {
                let win = (s.gtk_window_new)(GTK_WINDOW_TOPLEVEL);
                if let Ok(ct) = CString::new(config.title.as_str()) {
                    (s.gtk_window_set_title)(win, ct.as_ptr());
                }
                if let Some(set_res) = s.gtk_window_set_resizable {
                    set_res(win, i32::from(config.resizable));
                }
                win
            }
            _ => {
                return Err(EmbedError {
                    message: "Unsupported non-X11 window handle on Linux".to_owned(),
                });
            }
        };
        if root.is_null() {
            return Err(EmbedError {
                message: "GTK failed to construct root GtkPlug/GtkWindow".to_owned(),
            });
        }

        (s.gtk_window_set_default_size)(root, phys_w, phys_h);
        (s.gtk_widget_set_size_request)(root, phys_w, phys_h);
        (s.gtk_window_resize)(root, phys_w, phys_h);

        let initial_snap = sink.capture_ui_state();
        let mut kind = "linux-gtk3-cairo";
        let mut webview_widget = core::ptr::null_mut();
        let content_widget;

        if let Some(ref wk) = s.webkit {
            static SCHEME_ONCE: Once = Once::new();
            SCHEME_ONCE.call_once(|| {
                let ctx = (wk.web_context_get_default)();
                if !ctx.is_null() {
                    (wk.web_context_register_scheme)(
                        ctx,
                        c"prophet".as_ptr(),
                        scheme_request_cb,
                        core::ptr::null_mut(),
                        core::ptr::null_mut(),
                    );
                }
            });
            let ucm = (wk.ucm_new)();
            let ok_handler = (wk.ucm_register)(ucm, c"prophet".as_ptr()) != 0;
            let wv = (wk.webview_new_ucm)(ucm);
            if !wv.is_null() && ok_handler {
                kind = "linux-webkit2gtk";
                webview_widget = wv;
                content_widget = wv;
                (s.gtk_container_add)(root, wv);
                (s.gtk_widget_set_size_request)(wv, phys_w, phys_h);
                let view = Arc::new(GtkViewState {
                    _id: view_id,
                    sink,
                    backend_tag: config.backend_tag.clone(),
                    root_widget: root,
                    content_widget,
                    webview_widget,
                    timer_id: AtomicU32::new(0),
                    alive: AtomicBool::new(true),
                    child_xid: AtomicU32::new(0),
                    drag_param: Mutex::new(None),
                    held_key: Mutex::new(None),
                    last_snapshot: Mutex::new(Some(initial_snap)),
                });
                let raw = Arc::into_raw(view.clone()) as *mut c_void;
                (s.g_signal_connect_data)(
                    root,
                    c"destroy".as_ptr(),
                    on_widget_destroy as *const c_void,
                    raw,
                    Some(drop_view_arc),
                    0,
                );
                (s.g_signal_connect_data)(
                    ucm,
                    c"script-message-received::prophet".as_ptr(),
                    on_script_message as *const c_void,
                    raw,
                    None,
                    0,
                );
                (s.g_object_unref)(ucm);
                (wk.webview_load_uri)(wv, c"prophet://app/index.html".as_ptr());
                (s.gtk_widget_realize)(root);
                (s.gtk_widget_show_all)(root);
                if let Some(flush) = s.gdk_flush {
                    flush();
                }
                if let Some(plug_id_fn) = s.gtk_plug_get_id {
                    if matches!(parent, ParentWindow::X11(_)) {
                        view.child_xid.store(plug_id_fn(root) as u32, Ordering::Release);
                    }
                }
                let tag = (s.g_timeout_add)(33, on_timeout_tick, raw);
                view.timer_id.store(tag, Ordering::Release);
                return Ok((view, kind));
            }
            if !ucm.is_null() {
                (s.g_object_unref)(ucm);
            }
        }

        // Tier 2: Guaranteed GTK3 + Cairo interactive hardware-style panel
        let area = (s.gtk_drawing_area_new)();
        content_widget = area;
        (s.gtk_widget_set_size_request)(area, phys_w, phys_h);
        (s.gtk_widget_add_events)(
            area,
            GDK_EXPOSURE_MASK
                | GDK_BUTTON_PRESS_MASK
                | GDK_BUTTON_RELEASE_MASK
                | GDK_POINTER_MOTION_MASK
                | GDK_KEY_PRESS_MASK,
        );
        (s.gtk_widget_set_can_focus)(area, 1);
        (s.gtk_container_add)(root, area);

        let view = Arc::new(GtkViewState {
            _id: view_id,
            sink,
            backend_tag: config.backend_tag.clone(),
            root_widget: root,
            content_widget,
            webview_widget,
            timer_id: AtomicU32::new(0),
            alive: AtomicBool::new(true),
            child_xid: AtomicU32::new(0),
            drag_param: Mutex::new(None),
            held_key: Mutex::new(None),
            last_snapshot: Mutex::new(Some(initial_snap)),
        });
        let raw = Arc::into_raw(view.clone()) as *mut c_void;
        (s.g_signal_connect_data)(
            root,
            c"destroy".as_ptr(),
            on_widget_destroy as *const c_void,
            raw,
            Some(drop_view_arc),
            0,
        );
        (s.g_signal_connect_data)(
            area,
            c"draw".as_ptr(),
            on_cairo_draw as *const c_void,
            raw,
            None,
            0,
        );
        (s.g_signal_connect_data)(
            area,
            c"button-press-event".as_ptr(),
            on_cairo_button_press as *const c_void,
            raw,
            None,
            0,
        );
        (s.g_signal_connect_data)(
            area,
            c"button-release-event".as_ptr(),
            on_cairo_button_release as *const c_void,
            raw,
            None,
            0,
        );
        (s.g_signal_connect_data)(
            area,
            c"motion-notify-event".as_ptr(),
            on_cairo_motion as *const c_void,
            raw,
            None,
            0,
        );

        (s.gtk_widget_realize)(root);
        (s.gtk_widget_show_all)(root);
        if let Some(flush) = s.gdk_flush {
            flush();
        }

        if matches!(parent, ParentWindow::X11(_)) {
            if let Some(plug_id_fn) = s.gtk_plug_get_id {
                view.child_xid.store(plug_id_fn(root) as u32, Ordering::Release);
            } else if let Some(xid_fn) = s.gdk_x11_window_get_xid {
                let gdk_win = (s.gtk_widget_get_window)(root);
                if !gdk_win.is_null() {
                    view.child_xid.store(xid_fn(gdk_win) as u32, Ordering::Release);
                }
            }
        }

        let tag = (s.g_timeout_add)(33, on_timeout_tick, raw);
        view.timer_id.store(tag, Ordering::Release);
        Ok((view, kind))
    }

    enum ReactorCmd {
        Spawn {
            id: u64,
            parent: ParentWindow,
            config: EditorViewConfig,
            sink: Arc<dyn HostBridgeSink>,
            reply: mpsc::Sender<Result<(Arc<GtkViewState>, &'static str), EmbedError>>,
        },
        Resize {
            id: u64,
            w: u32,
            h: u32,
            scale: f32,
        },
        Destroy {
            id: u64,
            ack: mpsc::Sender<()>,
        },
    }

    // `ParentWindow` variants on Linux only pass integer X11 ID or StandaloneWindow safely.
    unsafe impl Send for ReactorCmd {}

    fn reactor() -> &'static mpsc::Sender<ReactorCmd> {
        static REACTOR: OnceLock<mpsc::Sender<ReactorCmd>> = OnceLock::new();
        REACTOR.get_or_init(|| {
            let (tx, rx) = mpsc::channel::<ReactorCmd>();
            std::thread::Builder::new()
                .name("prophet-gtk-ui".to_owned())
                .spawn(move || {
                    let mut views: BTreeMap<u64, Arc<GtkViewState>> = BTreeMap::new();
                    loop {
                        while let Ok(cmd) = rx.try_recv() {
                            match cmd {
                                ReactorCmd::Spawn {
                                    id,
                                    parent,
                                    config,
                                    sink,
                                    reply,
                                } => {
                                    let res = unsafe {
                                        syms().and_then(|s| {
                                            let _trap = GdkErrorTrap::new(s);
                                            let r = build_gtk_widgets(s, parent, &config, sink, id)?;
                                            for _ in 0..8 {
                                                if (s.gtk_events_pending)() == 0 {
                                                    break;
                                                }
                                                (s.gtk_main_iteration_do)(0);
                                            }
                                            Ok(r)
                                        })
                                    };
                                    if let Ok((ref view, _)) = res {
                                        views.insert(id, view.clone());
                                    }
                                    let _ = reply.send(res);
                                }
                                ReactorCmd::Resize { id, w, h, scale } => {
                                    if let Some(view) = views.get(&id) {
                                        if view.alive.load(Ordering::Acquire) {
                                            if let Ok(s) = syms() {
                                                let pw = ((w as f32) * scale.max(0.5)).round() as c_int;
                                                let ph = ((h as f32) * scale.max(0.5)).round() as c_int;
                                                unsafe {
                                                    let _trap = GdkErrorTrap::new(s);
                                                    (s.gtk_widget_set_size_request)(
                                                        view.content_widget,
                                                        pw,
                                                        ph,
                                                    );
                                                    (s.gtk_widget_set_size_request)(
                                                        view.root_widget,
                                                        pw,
                                                        ph,
                                                    );
                                                    (s.gtk_window_resize)(view.root_widget, pw, ph);
                                                }
                                            }
                                        }
                                    }
                                }
                                ReactorCmd::Destroy { id, ack } => {
                                    if let Some(view) = views.remove(&id) {
                                        let was_alive = view.alive.swap(false, Ordering::AcqRel);
                                        if let Ok(s) = syms() {
                                            unsafe {
                                                let _trap = GdkErrorTrap::new(s);
                                                let tag = view.timer_id.swap(0, Ordering::AcqRel);
                                                if tag != 0 {
                                                    (s.g_source_remove)(tag);
                                                }
                                                if was_alive && !view.root_widget.is_null() {
                                                    (s.gtk_widget_destroy)(view.root_widget);
                                                }
                                                for _ in 0..16 {
                                                    if (s.gtk_events_pending)() == 0 {
                                                        break;
                                                    }
                                                    (s.gtk_main_iteration_do)(0);
                                                }
                                            }
                                        }
                                    }
                                    let _ = ack.send(());
                                }
                            }
                        }

                        if !views.is_empty() {
                            if let Ok(s) = syms() {
                                let mut dead = Vec::new();
                                for (&id, v) in &views {
                                    if !v.alive.load(Ordering::Acquire) {
                                        dead.push(id);
                                    }
                                }
                                for id in dead {
                                    views.remove(&id);
                                }
                                unsafe {
                                    let _trap = GdkErrorTrap::new(s);
                                    let mut guard = 0usize;
                                    while (s.gtk_events_pending)() != 0 && guard < 128 {
                                        (s.gtk_main_iteration_do)(0);
                                        guard += 1;
                                    }
                                }
                            }
                            std::thread::sleep(std::time::Duration::from_millis(12));
                        } else {
                            match rx.recv() {
                                Ok(first_cmd) => {
                                    match first_cmd {
                                        ReactorCmd::Spawn {
                                            id,
                                            parent,
                                            config,
                                            sink,
                                            reply,
                                        } => {
                                            let res = unsafe {
                                                syms().and_then(|s| {
                                                    let _trap = GdkErrorTrap::new(s);
                                                    let r = build_gtk_widgets(
                                                        s, parent, &config, sink, id,
                                                    )?;
                                                    for _ in 0..8 {
                                                        if (s.gtk_events_pending)() == 0 {
                                                            break;
                                                        }
                                                        (s.gtk_main_iteration_do)(0);
                                                    }
                                                    Ok(r)
                                                })
                                            };
                                            if let Ok((ref view, _)) = res {
                                                views.insert(id, view.clone());
                                            }
                                            let _ = reply.send(res);
                                        }
                                        ReactorCmd::Resize { .. } => {}
                                        ReactorCmd::Destroy { ack, .. } => {
                                            let _ = ack.send(());
                                        }
                                    }
                                }
                                Err(_) => break,
                            }
                        }
                    }
                })
                .expect("spawn prophet-gtk-ui reactor thread");
            tx
        })
    }

    pub struct LinuxEditorHandle {
        id: u64,
        view: Arc<GtkViewState>,
        uses_reactor: bool,
        kind: &'static str,
    }

    impl LinuxEditorHandle {
        pub fn kind(&self) -> &'static str {
            self.kind
        }

        pub fn is_open(&self) -> bool {
            self.view.alive.load(Ordering::Acquire)
        }

        pub fn child_xid(&self) -> Option<u32> {
            let x = self.view.child_xid.load(Ordering::Acquire);
            (x != 0).then_some(x)
        }

        pub fn resize(&self, w: u32, h: u32, scale: f32) {
            if self.uses_reactor {
                let _ = reactor().send(ReactorCmd::Resize {
                    id: self.id,
                    w,
                    h,
                    scale,
                });
            } else if self.view.alive.load(Ordering::Acquire) {
                if let Ok(s) = syms() {
                    let pw = ((w as f32) * scale.max(0.5)).round() as c_int;
                    let ph = ((h as f32) * scale.max(0.5)).round() as c_int;
                    unsafe {
                        let _trap = GdkErrorTrap::new(s);
                        (s.gtk_widget_set_size_request)(self.view.content_widget, pw, ph);
                        (s.gtk_widget_set_size_request)(self.view.root_widget, pw, ph);
                        (s.gtk_window_resize)(self.view.root_widget, pw, ph);
                    }
                }
            }
        }
    }

    impl Drop for LinuxEditorHandle {
        fn drop(&mut self) {
            if self.uses_reactor {
                let (ack_tx, ack_rx) = mpsc::channel();
                if reactor()
                    .send(ReactorCmd::Destroy {
                        id: self.id,
                        ack: ack_tx,
                    })
                    .is_ok()
                {
                    let _ = ack_rx.recv_timeout(std::time::Duration::from_secs(2));
                }
            } else {
                let was_alive = self.view.alive.swap(false, Ordering::AcqRel);
                if let Ok(s) = syms() {
                    unsafe {
                        let _trap = GdkErrorTrap::new(s);
                        let tag = self.view.timer_id.swap(0, Ordering::AcqRel);
                        if tag != 0 {
                            (s.g_source_remove)(tag);
                        }
                        if was_alive && !self.view.root_widget.is_null() {
                            (s.gtk_widget_destroy)(self.view.root_widget);
                        }
                    }
                }
            }
        }
    }

    pub fn spawn(
        parent: ParentWindow,
        config: EditorViewConfig,
        sink: Arc<dyn HostBridgeSink>,
    ) -> Result<LinuxEditorHandle, EmbedError> {
        static NEXT_ID: AtomicU64 = AtomicU64::new(1);
        let id = NEXT_ID.fetch_add(1, Ordering::Relaxed);
        let s = syms()?;
        let host_has_gtk_loop = unsafe { (s.gtk_main_level)() > 0 };
        if host_has_gtk_loop {
            let (view, kind) = unsafe { build_gtk_widgets(s, parent, &config, sink, id)? };
            Ok(LinuxEditorHandle {
                id,
                view,
                uses_reactor: false,
                kind,
            })
        } else {
            let (tx, rx) = mpsc::channel();
            reactor()
                .send(ReactorCmd::Spawn {
                    id,
                    parent,
                    config,
                    sink,
                    reply: tx,
                })
                .map_err(|_| EmbedError {
                    message: "GTK UI reactor channel closed".to_owned(),
                })?;
            let (view, kind) = rx.recv().map_err(|_| EmbedError {
                message: "GTK UI reactor aborted during spawn".to_owned(),
            })??;
            Ok(LinuxEditorHandle {
                id,
                view,
                uses_reactor: true,
                kind,
            })
        }
    }
}

// ============================================================================
// Windows x64 Win32 HWND + WebView2 COM Runtime + GDI Interactive Fallback
// ============================================================================

#[cfg(target_os = "windows")]
mod windows_webview2 {
    use super::{
        inject_bridge_into_html, native_bridge_bootstrap_script, resolve_embedded_asset,
        BridgeTransportMode, EditorViewConfig, EmbedError, HostBridgeSink, ParentWindow,
    };
    use crate::UiCommand;
    use std::ffi::{c_char, c_int, c_void, OsStr};
    use std::os::windows::ffi::OsStrExt;
    use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
    use std::sync::Arc;

    type HWND = *mut c_void;
    type HINSTANCE = *mut c_void;
    type HMODULE = *mut c_void;
    type HRESULT = i32;
    type WPARAM = usize;
    type LPARAM = isize;
    type LRESULT = isize;
    type BOOL = c_int;

    const WS_CHILD: u32 = 0x4000_0000;
    const WS_VISIBLE: u32 = 0x1000_0000;
    const WS_OVERLAPPEDWINDOW: u32 = 0x00CF_0000;
    const WS_CLIPCHILDREN: u32 = 0x0200_0000;
    const WM_DESTROY: u32 = 0x0002;
    const WM_PAINT: u32 = 0x000F;
    const WM_TIMER: u32 = 0x0113;
    const WM_LBUTTONDOWN: u32 = 0x0201;
    const TIMER_SNAPSHOT_ID: usize = 0x5012;

    fn wide(s: &str) -> Vec<u16> {
        OsStr::new(s).encode_wide().chain(core::iter::once(0)).collect()
    }

    #[link(name = "kernel32")]
    extern "system" {
        fn LoadLibraryW(lp_lib_file_name: *const u16) -> HMODULE;
        fn GetProcAddress(h_module: HMODULE, lp_proc_name: *const c_char) -> *mut c_void;
        fn FreeLibrary(h_lib_module: HMODULE) -> BOOL;
    }

    struct WinDynLib {
        module: HMODULE,
    }

    unsafe impl Send for WinDynLib {}
    unsafe impl Sync for WinDynLib {}

    impl WinDynLib {
        unsafe fn open(name: &str) -> Option<Self> {
            let w = wide(name);
            let module = LoadLibraryW(w.as_ptr());
            (!module.is_null()).then_some(Self { module })
        }

        unsafe fn get_raw(&self, nul_name: &[u8]) -> Option<*mut c_void> {
            if nul_name.last().copied() != Some(0) {
                return None;
            }
            let ptr = GetProcAddress(self.module, nul_name.as_ptr() as *const c_char);
            (!ptr.is_null()).then_some(ptr)
        }
    }

    impl Drop for WinDynLib {
        fn drop(&mut self) {
            if !self.module.is_null() {
                unsafe {
                    let _ = FreeLibrary(self.module);
                }
            }
        }
    }

    #[repr(C)]
    struct RECT {
        left: i32,
        top: i32,
        right: i32,
        bottom: i32,
    }

    #[repr(C)]
    struct WNDCLASSW {
        style: u32,
        lpfn_wnd_proc: Option<unsafe extern "system" fn(HWND, u32, WPARAM, LPARAM) -> LRESULT>,
        cb_cls_extra: c_int,
        cb_wnd_extra: c_int,
        h_instance: HINSTANCE,
        h_icon: *mut c_void,
        h_cursor: *mut c_void,
        hbr_background: *mut c_void,
        lpsz_menu_name: *const u16,
        lpsz_class_name: *const u16,
    }

    #[repr(C)]
    struct PAINTSTRUCT {
        hdc: *mut c_void,
        f_erase: BOOL,
        rc_paint: RECT,
        f_restore: BOOL,
        f_inc_update: BOOL,
        rgb_reserved: [u8; 32],
    }

    #[link(name = "user32")]
    extern "system" {
        fn RegisterClassW(wc: *const WNDCLASSW) -> u16;
        fn CreateWindowExW(
            dw_ex_style: u32,
            lp_class_name: *const u16,
            lp_window_name: *const u16,
            dw_style: u32,
            x: c_int,
            y: c_int,
            n_width: c_int,
            n_height: c_int,
            hwnd_parent: HWND,
            h_menu: *mut c_void,
            h_instance: HINSTANCE,
            lp_param: *mut c_void,
        ) -> HWND;
        fn DestroyWindow(hwnd: HWND) -> BOOL;
        fn ShowWindow(hwnd: HWND, cmd_show: c_int) -> BOOL;
        fn MoveWindow(hwnd: HWND, x: c_int, y: c_int, w: c_int, h: c_int, repaint: BOOL) -> BOOL;
        fn SetTimer(hwnd: HWND, id: usize, elapse: u32, func: *mut c_void) -> usize;
        fn KillTimer(hwnd: HWND, id: usize) -> BOOL;
        fn DefWindowProcW(hwnd: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> LRESULT;
        fn SetWindowLongPtrW(hwnd: HWND, idx: c_int, val: isize) -> isize;
        fn GetWindowLongPtrW(hwnd: HWND, idx: c_int) -> isize;
        fn BeginPaint(hwnd: HWND, ps: *mut PAINTSTRUCT) -> *mut c_void;
        fn EndPaint(hwnd: HWND, ps: *const PAINTSTRUCT) -> BOOL;
        fn InvalidateRect(hwnd: HWND, rect: *const RECT, erase: BOOL) -> BOOL;
        fn FillRect(hdc: *mut c_void, rect: *const RECT, hbr: *mut c_void) -> c_int;
        fn GetClientRect(hwnd: HWND, rect: *mut RECT) -> BOOL;
    }

    #[link(name = "gdi32")]
    extern "system" {
        fn CreateSolidBrush(color: u32) -> *mut c_void;
        fn DeleteObject(obj: *mut c_void) -> BOOL;
        fn SetBkMode(hdc: *mut c_void, mode: c_int) -> c_int;
        fn SetTextColor(hdc: *mut c_void, color: u32) -> u32;
        fn TextOutW(hdc: *mut c_void, x: c_int, y: c_int, text: *const u16, len: c_int) -> BOOL;
    }

    // Minimal WebView2 loader signature checked dynamically so plugin loads cleanly without
    // redistributing proprietary binaries when system Edge WebView2 or loader DLL is absent.
    type FnCreateCoreWebView2Env = unsafe extern "system" fn(
        *const u16,
        *const u16,
        *mut c_void,
        *mut c_void,
    ) -> HRESULT;

    struct WinViewState {
        sink: Arc<dyn HostBridgeSink>,
        hwnd: AtomicUsize,
        alive: AtomicBool,
        backend_tag: String,
    }

    const GWLP_USERDATA: c_int = -21;

    unsafe extern "system" fn wnd_proc(hwnd: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
        let ptr = GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *const WinViewState;
        match msg {
            WM_TIMER => {
                if !ptr.is_null() {
                    InvalidateRect(hwnd, core::ptr::null(), 0);
                }
                0
            }
            WM_PAINT => {
                let mut ps: PAINTSTRUCT = core::mem::zeroed();
                let hdc = BeginPaint(hwnd, &mut ps);
                let mut rc = RECT { left: 0, top: 0, right: 800, bottom: 500 };
                GetClientRect(hwnd, &mut rc);
                let bg = CreateSolidBrush(0x00121315);
                FillRect(hdc, &rc, bg);
                DeleteObject(bg);
                if !ptr.is_null() {
                    let st = &*ptr;
                    let snap = st.sink.capture_ui_state();
                    SetBkMode(hdc, 1);
                    SetTextColor(hdc, 0x009ED6F5);
                    let line = wide(&format!(
                        "PROPHET-XII [{}] SR:{}Hz Lat:{}smp Reverb:{:?}",
                        st.backend_tag,
                        snap.engine_port.sample_rate,
                        snap.engine_port.latency_samples,
                        snap.engine_port.fx.algorithm
                    ));
                    TextOutW(hdc, 24, 24, line.as_ptr(), (line.len() - 1) as c_int);
                }
                EndPaint(hwnd, &ps);
                0
            }
            WM_LBUTTONDOWN => {
                if !ptr.is_null() {
                    let st = &*ptr;
                    let cur = st.sink.capture_ui_state().reverb_bypass_effective;
                    let _ = st.sink.dispatch_command(UiCommand::SetReverb {
                        algorithm: None,
                        enabled: Some(!cur),
                        tail: None,
                    });
                }
                0
            }
            WM_DESTROY => {
                KillTimer(hwnd, TIMER_SNAPSHOT_ID);
                if !ptr.is_null() {
                    let st = &*ptr;
                    st.alive.store(false, Ordering::Release);
                    SetWindowLongPtrW(hwnd, GWLP_USERDATA, 0);
                    drop(Arc::from_raw(ptr));
                }
                0
            }
            _ => DefWindowProcW(hwnd, msg, wp, lp),
        }
    }

    pub struct WindowsEditorHandle {
        state: Arc<WinViewState>,
        webview_loader: Option<WinDynLib>,
        kind: &'static str,
    }

    impl WindowsEditorHandle {
        pub fn kind(&self) -> &'static str {
            self.kind
        }

        pub fn is_open(&self) -> bool {
            self.state.alive.load(Ordering::Acquire)
        }

        pub fn resize(&self, w: u32, h: u32, scale: f32) {
            let hwnd = self.state.hwnd.load(Ordering::Acquire) as HWND;
            if !hwnd.is_null() && self.is_open() {
                let pw = ((w as f32) * scale.max(0.5)).round() as c_int;
                let ph = ((h as f32) * scale.max(0.5)).round() as c_int;
                unsafe {
                    MoveWindow(hwnd, 0, 0, pw, ph, 1);
                }
            }
        }
    }

    impl Drop for WindowsEditorHandle {
        fn drop(&mut self) {
            let hwnd = self.state.hwnd.swap(0, Ordering::AcqRel) as HWND;
            self.state.alive.store(false, Ordering::Release);
            if !hwnd.is_null() {
                unsafe {
                    KillTimer(hwnd, TIMER_SNAPSHOT_ID);
                    DestroyWindow(hwnd);
                }
            }
            let _ = &self.webview_loader;
        }
    }

    pub fn spawn(
        parent: ParentWindow,
        config: EditorViewConfig,
        sink: Arc<dyn HostBridgeSink>,
    ) -> Result<WindowsEditorHandle, EmbedError> {
        let class_name = wide("ProphetXiiEmbedWndClass");
        let wc = WNDCLASSW {
            style: 0,
            lpfn_wnd_proc: Some(wnd_proc),
            cb_cls_extra: 0,
            cb_wnd_extra: 0,
            h_instance: core::ptr::null_mut(),
            h_icon: core::ptr::null_mut(),
            h_cursor: core::ptr::null_mut(),
            hbr_background: core::ptr::null_mut(),
            lpsz_menu_name: core::ptr::null(),
            lpsz_class_name: class_name.as_ptr(),
        };
        unsafe { RegisterClassW(&wc) };

        let (parent_hwnd, style) = match parent {
            ParentWindow::Win32(h) => (h, WS_CHILD | WS_VISIBLE | WS_CLIPCHILDREN),
            ParentWindow::StandaloneWindow => (core::ptr::null_mut(), WS_OVERLAPPEDWINDOW | WS_VISIBLE | WS_CLIPCHILDREN),
            _ => {
                return Err(EmbedError {
                    message: "Unsupported window handle variant on Windows".to_owned(),
                });
            }
        };

        let pw = ((config.logical_width as f32) * config.scale_factor.max(0.5)).round() as c_int;
        let ph = ((config.logical_height as f32) * config.scale_factor.max(0.5)).round() as c_int;
        let title_w = wide(&config.title);
        let hwnd = unsafe {
            CreateWindowExW(
                0,
                class_name.as_ptr(),
                title_w.as_ptr(),
                style,
                0,
                0,
                pw,
                ph,
                parent_hwnd,
                core::ptr::null_mut(),
                core::ptr::null_mut(),
                core::ptr::null_mut(),
            )
        };
        if hwnd.is_null() {
            return Err(EmbedError {
                message: "Win32 CreateWindowExW failed".to_owned(),
            });
        }

        let state = Arc::new(WinViewState {
            sink,
            hwnd: AtomicUsize::new(hwnd as usize),
            alive: AtomicBool::new(true),
            backend_tag: config.backend_tag,
        });
        unsafe {
            SetWindowLongPtrW(hwnd, GWLP_USERDATA, Arc::into_raw(state.clone()) as isize);
            SetTimer(hwnd, TIMER_SNAPSHOT_ID, 33, core::ptr::null_mut());
            ShowWindow(hwnd, 5);
        }

        // Probe optional WebView2Loader.dll alongside offline assets cleanly.
        let webview_loader = unsafe { WinDynLib::open("WebView2Loader.dll") };
        let has_wv2 = webview_loader
            .as_ref()
            .and_then(|l| unsafe { l.get_raw(b"CreateCoreWebView2EnvironmentWithOptions\0") })
            .map(|p| unsafe {
                let _: FnCreateCoreWebView2Env = core::mem::transmute_copy(&p);
            })
            .is_some();
        let _ = (
            resolve_embedded_asset,
            inject_bridge_into_html,
            native_bridge_bootstrap_script,
            BridgeTransportMode::WebView2PostMessage,
        );

        Ok(WindowsEditorHandle {
            state,
            webview_loader,
            kind: if has_wv2 {
                "windows-hwnd-webview2-ready"
            } else {
                "windows-win32-gdi"
            },
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn embedded_ui_assets_contain_offline_index_and_bundle_files() {
        let index = resolve_embedded_asset("/").expect("index at /");
        assert_eq!(index.path, "/index.html");
        assert!(index.mime.starts_with("text/html"));
        let html = String::from_utf8_lossy(index.bytes);
        assert!(html.contains("root"), "index.html missing root mount");
        assert!(EMBEDDED_UI_ASSETS.len() >= 2, "expected js/css assets alongside index.html");
    }

    #[test]
    fn html_bridge_injection_precedes_module_scripts() {
        let index = resolve_embedded_asset("/index.html").unwrap();
        let bootstrap = native_bridge_bootstrap_script(
            "vst3-clap",
            BridgeTransportMode::WebKitMessageHandler,
        );
        let injected = inject_bridge_into_html(index.bytes, &bootstrap);
        let text = String::from_utf8(injected).unwrap();
        let host_pos = text.find("window.__PROPHET_HOST__").unwrap();
        let mod_pos = text.find("type=\"module\"").unwrap();
        assert!(host_pos < mod_pos, "bridge bootstrap must run before module script");
    }
}
