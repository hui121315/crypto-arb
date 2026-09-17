use realtime::WsMessage;

pub(super) const BATCH_LIMIT: usize = 64;
pub(super) const FLUSH_MS: u64 = 100;

pub(super) type HubFrame = (String, WsMessage);

pub(super) struct OutboundBatch {
    frames: Vec<HubFrame>,
    limit: usize,
}

impl OutboundBatch {
    pub(super) fn new(limit: usize) -> Self {
        Self {
            frames: Vec::with_capacity(limit),
            limit: limit.max(1),
        }
    }

    pub(super) fn push(&mut self, frame: HubFrame) -> bool {
        self.frames.push(frame);
        self.frames.len() >= self.limit
    }

    pub(super) fn is_empty(&self) -> bool {
        self.frames.is_empty()
    }

    /// 手工装配 envelope：`JsonText` 载体原样内嵌（发布侧已序列化一次，
    /// 此处零重序列化）；`drain` 复用 frames 的预分配容量。
    pub(super) fn serialize_and_clear(&mut self) -> Result<Option<String>, serde_json::Error> {
        if self.frames.is_empty() {
            return Ok(None);
        }
        let mut out = String::with_capacity(estimated_len(&self.frames));
        if self.frames.len() == 1 {
            let (channel, msg) = &self.frames[0];
            out.push_str("{\"type\":\"message\",\"channel\":");
            out.push_str(&serde_json::to_string(channel)?);
            out.push_str(",\"payload\":");
            push_payload(&mut out, msg)?;
            out.push('}');
        } else {
            out.push_str("{\"type\":\"batch\",\"messages\":[");
            for (index, (channel, msg)) in self.frames.iter().enumerate() {
                if index > 0 {
                    out.push(',');
                }
                out.push_str("{\"channel\":");
                out.push_str(&serde_json::to_string(channel)?);
                out.push_str(",\"payload\":");
                push_payload(&mut out, msg)?;
                out.push('}');
            }
            out.push_str("]}");
        }
        self.frames.clear();
        Ok(Some(out))
    }
}

fn estimated_len(frames: &[HubFrame]) -> usize {
    64 + frames
        .iter()
        .map(|(channel, msg)| {
            channel.len()
                + 32
                + match msg {
                    WsMessage::JsonText(text) => text.len(),
                    WsMessage::Text(text) => text.len() + 2,
                    WsMessage::Json(_) | WsMessage::Binary(_) => 64,
                }
        })
        .sum::<usize>()
}

fn push_payload(out: &mut String, msg: &WsMessage) -> Result<(), serde_json::Error> {
    match msg {
        WsMessage::JsonText(text) => {
            out.push_str(text);
            Ok(())
        }
        WsMessage::Json(payload) => {
            out.push_str(&serde_json::to_string(payload)?);
            Ok(())
        }
        WsMessage::Text(payload) => {
            out.push_str(&serde_json::to_string(payload)?);
            Ok(())
        }
        WsMessage::Binary(_) => {
            out.push_str("\"<binary>\"");
            Ok(())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::Value;

    #[test]
    fn single_frame_keeps_message_envelope() -> Result<(), serde_json::Error> {
        let mut batch = OutboundBatch::new(64);
        assert!(!batch.push(("orders".into(), WsMessage::Text("ok".into()))));

        let text = batch
            .serialize_and_clear()?
            .ok_or_else(|| serde_json::Error::io(std::io::Error::other("missing frame")))?;
        let value: Value = serde_json::from_str(&text)?;

        assert_eq!(value["type"], "message");
        assert_eq!(value["channel"], "orders");
        assert_eq!(value["payload"], "ok");
        assert!(batch.is_empty());
        Ok(())
    }

    #[test]
    fn multiple_frames_use_batch_envelope() -> Result<(), serde_json::Error> {
        let mut batch = OutboundBatch::new(64);
        batch.push(("orders".into(), WsMessage::Text("a".into())));
        batch.push(("risk-alerts".into(), WsMessage::Text("b".into())));

        let text = batch
            .serialize_and_clear()?
            .ok_or_else(|| serde_json::Error::io(std::io::Error::other("missing frame")))?;
        let value: Value = serde_json::from_str(&text)?;

        assert_eq!(value["type"], "batch");
        assert_eq!(value["messages"].as_array().map(Vec::len), Some(2));
        assert_eq!(value["messages"][0]["channel"], "orders");
        assert_eq!(value["messages"][1]["payload"], "b");
        Ok(())
    }
}
