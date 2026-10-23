//! Ingress-owned native positions; consumers do not decode protocol shapes again.
use super::{ModelRequestIRV1, NativeHistoryItem};
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
        for item in layout {
            let index = item.native_index?;
            if previous.is_some_and(|previous| index <= previous) {
                return None;
            }
            values.get(index)?;
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
