//! Ingress-owned native positions; consumers do not decode protocol shapes again.
use super::{MessageRole, ModelRequestIRV1, NativeHistoryItem};
use crate::server::request_plan::IngressProtocol;
use serde_json::Value;

impl ModelRequestIRV1 {
    pub(crate) fn native_history_layout(&self) -> Option<&[NativeHistoryItem]> {
        let layout = self.native_history.as_deref()?;
        if layout.len() != self.messages.len() || layout.len() > 65_536 {
            return None;
        }
        let value = self.native_history_value()?;
        if value.is_string() {
            return (self.ingress_protocol == IngressProtocol::Responses
                && layout.len() == 1
                && layout[0].native_index.is_none())
            .then_some(layout);
        }
        let values = value.as_array()?;
        let mut previous = None;
        for (position, item) in layout.iter().enumerate() {
            let index = item.native_index?;
            if previous.is_some_and(|previous| index <= previous) {
                return None;
            }
            let native = values.get(index)?;
            // Preserve the typed peer placeholder when a request-local view is
            // changed after admission. Content storage may change, positions may not.
            if native.get("type").and_then(Value::as_str) == Some("agent_message") {
                let message = &self.messages[position];
                if item.modeled
                    || message.role != MessageRole::Assistant
                    || !message.content.is_empty()
                    || message.name.is_some()
                {
                    return None;
                }
            }
            previous = Some(index);
        }
        Some(layout)
    }

    pub(crate) fn native_history_value(&self) -> Option<&Value> {
        self.native_body.as_ref()?.get(match self.ingress_protocol {
            IngressProtocol::Responses => "input",
            IngressProtocol::ChatCompletions | IngressProtocol::Messages => "messages",
        })
    }

    pub(crate) fn native_history_item(&self, item: &NativeHistoryItem) -> Option<&Value> {
        let value = self.native_history_value()?;
        match item.native_index {
            Some(index) => value.as_array()?.get(index),
            None => value.is_string().then_some(value),
        }
    }
}
