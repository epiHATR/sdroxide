//! Browser fullscreen for the wasm client.
//!
//! Native builds use the window chrome; the web client has no OS title bar, so
//! the Display strip offers a chip that toggles the Fullscreen API on the page.

/// Whether the document is currently presenting fullscreen.
#[cfg(target_arch = "wasm32")]
pub fn is_fullscreen() -> bool {
    web_sys::window()
        .and_then(|w| w.document())
        .and_then(|d| d.fullscreen_element())
        .is_some()
}

/// Enter or leave document fullscreen. No-op when the API is unavailable.
#[cfg(target_arch = "wasm32")]
pub fn toggle() {
    let Some(window) = web_sys::window() else {
        return;
    };
    let Some(doc) = window.document() else {
        return;
    };
    if doc.fullscreen_element().is_some() {
        let _ = doc.exit_fullscreen();
        return;
    }
    let Some(el) = doc.document_element() else {
        return;
    };
    let _ = el.request_fullscreen();
}