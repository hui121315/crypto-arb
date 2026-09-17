#[cfg(target_arch = "wasm32")]
#[wasm_bindgen::prelude::wasm_bindgen(inline_js = r#"
export function crosslineWasmDecodeProbeStart() {
    return Array.isArray(globalThis.__crosslineWasmSerdeMetrics)
        ? performance.now()
        : NaN;
}

export function crosslineWasmDecodeProbeFinish(startedAt, success) {
    const metrics = globalThis.__crosslineWasmSerdeMetrics;
    if (!Array.isArray(metrics)) return;
    metrics.push({
        path: "/api/v3/arbitrage/opportunities/list",
        decodeMs: performance.now() - startedAt,
        success,
    });
}
"#)]
extern "C" {
    #[wasm_bindgen::prelude::wasm_bindgen(js_name = crosslineWasmDecodeProbeStart)]
    fn crossline_wasm_decode_probe_start() -> f64;
    #[wasm_bindgen::prelude::wasm_bindgen(js_name = crosslineWasmDecodeProbeFinish)]
    fn crossline_wasm_decode_probe_finish(started_at: f64, success: bool);
}

#[cfg(target_arch = "wasm32")]
pub(super) fn wasm_decode_probe_start(path: &str) -> f64 {
    if !path.starts_with("/api/v3/arbitrage/opportunities/list") {
        return f64::NAN;
    }
    crossline_wasm_decode_probe_start()
}

#[cfg(target_arch = "wasm32")]
pub(super) fn wasm_decode_probe_finish(started_at_ms: f64, success: bool) {
    if started_at_ms.is_finite() {
        crossline_wasm_decode_probe_finish(started_at_ms, success);
    }
}

#[cfg(not(target_arch = "wasm32"))]
pub(super) fn wasm_decode_probe_start(_path: &str) -> f64 {
    f64::NAN
}

#[cfg(not(target_arch = "wasm32"))]
pub(super) fn wasm_decode_probe_finish(_started_at_ms: f64, _success: bool) {}
