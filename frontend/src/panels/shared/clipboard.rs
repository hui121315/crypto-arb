#[cfg(target_arch = "wasm32")]
#[wasm_bindgen::prelude::wasm_bindgen(inline_js = r#"
export function crosslineCopyText(value) {
    if (globalThis.navigator?.clipboard) {
        void globalThis.navigator.clipboard.writeText(value);
    }
}
"#)]
extern "C" {
    #[wasm_bindgen::prelude::wasm_bindgen(js_name = crosslineCopyText)]
    fn crossline_copy_text(value: &str);
}

pub(crate) fn copy_text(value: &str) {
    #[cfg(target_arch = "wasm32")]
    crossline_copy_text(value);

    #[cfg(not(target_arch = "wasm32"))]
    let _ = value;
}
