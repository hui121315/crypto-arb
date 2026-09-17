use gloo_timers::callback::Interval;
use js_sys::{ArrayBuffer, Uint8Array};
use std::rc::Rc;
use wasm_bindgen::{closure::Closure, JsCast};
use web_sys::{BinaryType, Event, MessageEvent, WebSocket};

const MAX_DECOMPRESSED_FRAME_BYTES: usize = 8 * 1024 * 1024;

pub(super) struct SocketCallbacks {
    pub(super) on_open: Rc<dyn Fn(WebSocket)>,
    pub(super) on_text: Rc<dyn Fn(String)>,
    pub(super) on_error: Rc<dyn Fn(String)>,
    pub(super) on_close: Rc<dyn Fn()>,
}

pub(super) struct BrowserSocket {
    socket: WebSocket,
    _on_open: Closure<dyn FnMut(Event)>,
    _on_message: Closure<dyn FnMut(MessageEvent)>,
    _on_error: Closure<dyn FnMut(Event)>,
    _on_close: Closure<dyn FnMut(Event)>,
    _heartbeat: Interval,
}

impl BrowserSocket {
    pub(super) fn open(
        url: &str,
        heartbeat_ms: u32,
        callbacks: SocketCallbacks,
    ) -> Result<Self, String> {
        let socket = WebSocket::new(url).map_err(|error| js_error(&error))?;
        socket.set_binary_type(BinaryType::Arraybuffer);

        let open_socket = socket.clone();
        let on_open_callback = callbacks.on_open;
        let on_open = Closure::wrap(Box::new(move |_: Event| {
            on_open_callback(open_socket.clone());
        }) as Box<dyn FnMut(Event)>);
        socket.set_onopen(Some(on_open.as_ref().unchecked_ref()));

        let on_text_callback = callbacks.on_text;
        let on_message_error = Rc::clone(&callbacks.on_error);
        let on_message = Closure::wrap(Box::new(move |event: MessageEvent| {
            let data = event.data();
            if let Some(text) = data.as_string() {
                on_text_callback(text);
                return;
            }
            if data.is_instance_of::<ArrayBuffer>() {
                let buffer: ArrayBuffer = data.unchecked_into();
                let bytes = Uint8Array::new(&buffer).to_vec();
                match decode_zlib_json(&bytes) {
                    Ok(text) => on_text_callback(text),
                    Err(error) => on_message_error(error),
                }
            }
        }) as Box<dyn FnMut(MessageEvent)>);
        socket.set_onmessage(Some(on_message.as_ref().unchecked_ref()));

        let on_error_callback = Rc::clone(&callbacks.on_error);
        let on_error = Closure::wrap(Box::new(move |_: Event| {
            on_error_callback("browser websocket transport error".into());
        }) as Box<dyn FnMut(Event)>);
        socket.set_onerror(Some(on_error.as_ref().unchecked_ref()));

        let on_close_callback = callbacks.on_close;
        let on_close = Closure::wrap(Box::new(move |_: Event| {
            on_close_callback();
        }) as Box<dyn FnMut(Event)>);
        socket.set_onclose(Some(on_close.as_ref().unchecked_ref()));

        let heartbeat_socket = socket.clone();
        let heartbeat_error = callbacks.on_error;
        let heartbeat = Interval::new(heartbeat_ms, move || {
            if heartbeat_socket.ready_state() != WebSocket::OPEN {
                return;
            }
            if let Err(error) = heartbeat_socket.send_with_str(r#"{"type":"ping"}"#) {
                heartbeat_error(format!("heartbeat write failed: {}", js_error(&error)));
                let _ = heartbeat_socket.close();
            }
        });

        Ok(Self {
            socket,
            _on_open: on_open,
            _on_message: on_message,
            _on_error: on_error,
            _on_close: on_close,
            _heartbeat: heartbeat,
        })
    }

    pub(super) fn close(&self) {
        let _ = self.socket.close();
    }
}

impl Drop for BrowserSocket {
    fn drop(&mut self) {
        self.socket.set_onopen(None);
        self.socket.set_onmessage(None);
        self.socket.set_onerror(None);
        self.socket.set_onclose(None);
        let _ = self.socket.close();
    }
}

pub(super) fn frame_sender(socket: WebSocket) -> Rc<dyn Fn(&str) -> Result<(), String>> {
    Rc::new(move |frame| {
        socket
            .send_with_str(frame)
            .map_err(|error| js_error(&error))
    })
}

fn js_error(error: &wasm_bindgen::JsValue) -> String {
    error.as_string().unwrap_or_else(|| format!("{error:?}"))
}

fn decode_zlib_json(bytes: &[u8]) -> Result<String, String> {
    let decoded = miniz_oxide::inflate::decompress_to_vec_zlib_with_limit(
        bytes,
        MAX_DECOMPRESSED_FRAME_BYTES,
    )
    .map_err(|error| format!("app websocket zlib decode failed: {error:?}"))?;
    String::from_utf8(decoded)
        .map_err(|error| format!("app websocket decoded frame is not UTF-8: {error}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn zlib_json_frame_round_trips() -> Result<(), String> {
        let source = r#"{"type":"message","channel":"portfolio","payload":{"ok":true}}"#;
        let encoded = miniz_oxide::deflate::compress_to_vec_zlib(source.as_bytes(), 3);

        assert_eq!(decode_zlib_json(&encoded)?, source);
        Ok(())
    }

    #[test]
    fn zlib_json_frame_rejects_invalid_payload() {
        assert!(decode_zlib_json(b"not-zlib").is_err());
    }
}
