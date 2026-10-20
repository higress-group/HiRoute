//! One control owner for CLI, Desktop and console; no host retrieves model facts itself.
use crate::{ApplicationService, failed, succeeded};
use hiroute_application_api::*;
use serde_json::Value;

pub trait ModelFeedPort: Send + Sync {
    fn list_model_metadata(
        &self,
        _request: ListModelMetadataV1,
    ) -> Result<ModelMetadataPageV1, ErrorCode> {
        Err(ErrorCode::DaemonUnavailable)
    }
    fn get_model_metadata(
        &self,
        _request: GetModelMetadataV1,
    ) -> Result<ModelMetadataDetailV1, ErrorCode> {
        Err(ErrorCode::DaemonUnavailable)
    }
    fn feed_status(&self) -> Result<ModelFeedStatusV2, ErrorCode>;
    fn feed_refresh(&self) -> Result<ModelFeedStatusV2, ErrorCode>;
    fn feed_settings(
        &self,
        request: ModelFeedSettingsRequestV1,
    ) -> Result<ModelFeedStatusV2, ErrorCode>;
    fn feed_remove_preview(&self) -> Result<ModelFeedRemovalPreviewV2, ErrorCode>;
    fn feed_remove(
        &self,
        request: ModelFeedRemoveRequestV1,
    ) -> Result<ModelFeedStatusV2, ErrorCode>;
    fn model_availability(&self) -> Result<ModelAvailabilityResultV2, ErrorCode>;
}

pub(crate) fn dispatch(
    service: &ApplicationService,
    request: LocalControlRequestV2,
) -> MachineEnvelopeV2<Value> {
    if request.protected_grant.is_some() {
        return failed(ErrorCode::CapabilityDenied, request.request_id);
    }
    let Some(port) = service
        .ports
        .as_ref()
        .and_then(|ports| ports.model_feed.as_ref())
    else {
        return failed(ErrorCode::DaemonUnavailable, request.request_id);
    };
    let result = match request.operation_id.as_str() {
        "ListModelMetadata" => serde_json::from_value(request.payload)
            .map_err(|_| ErrorCode::InvalidArguments)
            .and_then(|payload| port.list_model_metadata(payload))
            .and_then(value),
        "GetModelMetadata" => serde_json::from_value(request.payload)
            .map_err(|_| ErrorCode::InvalidArguments)
            .and_then(|payload| port.get_model_metadata(payload))
            .and_then(value),
        "ApplyModelFeedSettings" => serde_json::from_value(request.payload)
            .map_err(|_| ErrorCode::InvalidArguments)
            .and_then(|payload| port.feed_settings(payload))
            .and_then(value),
        "ApplyModelFeedRemoval" => serde_json::from_value(request.payload)
            .map_err(|_| ErrorCode::InvalidArguments)
            .and_then(|payload| port.feed_remove(payload))
            .and_then(value),
        operation => {
            if serde_json::from_value::<ClientEmptyRequestV1>(request.payload).is_err() {
                return failed(ErrorCode::InvalidArguments, request.request_id);
            }
            match operation {
                "GetModelFeedStatus" => port.feed_status().and_then(value),
                "RefreshModelFeed" => port.feed_refresh().and_then(value),
                "PreviewModelFeedRemoval" => port.feed_remove_preview().and_then(value),
                "GetModelAvailability" => port.model_availability().and_then(value),
                _ => Err(ErrorCode::UnknownCommand),
            }
        }
    };
    match result {
        Ok(result) => succeeded(result, request.request_id),
        Err(error) => failed(error, request.request_id),
    }
}
fn value<T: serde::Serialize>(result: T) -> Result<Value, ErrorCode> {
    serde_json::to_value(result).map_err(|_| ErrorCode::Internal)
}
