//! Portable preference transfer shared by Desktop and the web console.
use super::*;
use serde::{Deserialize, Serialize};

pub const CONFIGURATION_BUNDLE_SCHEMA_V1: &str = "hiroute.configuration-bundle/v1";

/// The portable subset of HiRoute preferences. Routing plans, decision services and
/// model connections are intentionally absent: their model bindings, source
/// references and credentials are installation-specific and cannot be re-applied
/// verbatim on another instance.
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ConfigurationBundleV1 {
    pub schema: String,
    pub worker_settings: WorkerSettingsV1,
    pub observation_capacity: ObservationCapacityUpdateV1,
    pub model_feed: ModelFeedSettingsRequestV1,
}

impl ConfigurationBundleV1 {
    pub fn valid(&self) -> bool {
        self.schema == CONFIGURATION_BUNDLE_SCHEMA_V1
            && self.worker_settings.valid()
            && self
                .observation_capacity
                .max_active_bytes
                .is_none_or(|limit| limit > 0)
    }
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ConfigurationImportStateV1 {
    Applied,
    Unchanged,
    Failed,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ConfigurationImportItemV1 {
    pub state: ConfigurationImportStateV1,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error_code: Option<EventErrorCode>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ConfigurationImportResultV1 {
    pub schema: String,
    pub worker_settings: ConfigurationImportItemV1,
    pub observation_capacity: ConfigurationImportItemV1,
    pub model_feed: ConfigurationImportItemV1,
}

pub async fn configuration_export(
    window: &ClientContext<'_>,
    state: &CommandState<'_>,
) -> Result<ConfigurationBundleV1, DesktopFailure> {
    main_window(window)?;
    let worker_settings = worker_settings_get(window, state).await?;
    let observation = observation_capacity_get(window, state).await?;
    let feed = model_feed_status(window, state).await?;
    Ok(ConfigurationBundleV1 {
        schema: CONFIGURATION_BUNDLE_SCHEMA_V1.into(),
        worker_settings,
        observation_capacity: ObservationCapacityUpdateV1 {
            max_active_bytes: observation.max_active_bytes,
        },
        model_feed: ModelFeedSettingsRequestV1 {
            retrieval_enabled: Some(feed.retrieval_enabled),
            acknowledge_disclosure: Some(feed.disclosure_acknowledged),
        },
    })
}

pub async fn configuration_import(
    window: &ClientContext<'_>,
    state: &CommandState<'_>,
    input: ConfigurationBundleV1,
) -> Result<ConfigurationImportResultV1, DesktopFailure> {
    main_window(window)?;
    if !input.valid() {
        return Err("CONFIGURATION_BUNDLE_INVALID".into());
    }
    let worker_settings = import_worker_settings(window, state, &input).await;
    let observation_capacity = import_observation_capacity(window, state, &input).await;
    let model_feed = import_model_feed(window, state, &input).await;
    Ok(ConfigurationImportResultV1 {
        schema: CONFIGURATION_BUNDLE_SCHEMA_V1.into(),
        worker_settings,
        observation_capacity,
        model_feed,
    })
}

async fn import_worker_settings(
    window: &ClientContext<'_>,
    state: &CommandState<'_>,
    input: &ConfigurationBundleV1,
) -> ConfigurationImportItemV1 {
    let current = match worker_settings_get(window, state).await {
        Ok(value) => value,
        Err(error) => return failed(error),
    };
    if current == input.worker_settings {
        return unchanged();
    }
    match worker_settings_set(window, state, input.worker_settings.clone()).await {
        Ok(_) => applied(),
        Err(error) => failed(error),
    }
}

async fn import_observation_capacity(
    window: &ClientContext<'_>,
    state: &CommandState<'_>,
    input: &ConfigurationBundleV1,
) -> ConfigurationImportItemV1 {
    let current = match observation_capacity_get(window, state).await {
        Ok(value) => value,
        Err(error) => return failed(error),
    };
    if current.max_active_bytes == input.observation_capacity.max_active_bytes {
        return unchanged();
    }
    match observation_capacity_set(window, state, input.observation_capacity.clone()).await {
        Ok(_) => applied(),
        Err(error) => failed(error),
    }
}

async fn import_model_feed(
    window: &ClientContext<'_>,
    state: &CommandState<'_>,
    input: &ConfigurationBundleV1,
) -> ConfigurationImportItemV1 {
    let request = &input.model_feed;
    if request.retrieval_enabled.is_none() && request.acknowledge_disclosure.is_none() {
        return unchanged();
    }
    let current = match model_feed_status(window, state).await {
        Ok(value) => value,
        Err(error) => return failed(error),
    };
    let retrieval = request
        .retrieval_enabled
        .unwrap_or(current.retrieval_enabled);
    let acknowledged = request
        .acknowledge_disclosure
        .unwrap_or(current.disclosure_acknowledged);
    if retrieval == current.retrieval_enabled && acknowledged == current.disclosure_acknowledged {
        return unchanged();
    }
    match model_feed_settings_apply(window, state, request.clone()).await {
        Ok(_) => applied(),
        Err(error) => failed(error),
    }
}

fn applied() -> ConfigurationImportItemV1 {
    ConfigurationImportItemV1 {
        state: ConfigurationImportStateV1::Applied,
        error_code: None,
    }
}

fn unchanged() -> ConfigurationImportItemV1 {
    ConfigurationImportItemV1 {
        state: ConfigurationImportStateV1::Unchanged,
        error_code: None,
    }
}

fn failed(error: DesktopFailure) -> ConfigurationImportItemV1 {
    ConfigurationImportItemV1 {
        state: ConfigurationImportStateV1::Failed,
        error_code: Some(error.event_code()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn bundle() -> ConfigurationBundleV1 {
        ConfigurationBundleV1 {
            schema: CONFIGURATION_BUNDLE_SCHEMA_V1.into(),
            worker_settings: WorkerSettingsV1::new(4),
            observation_capacity: ObservationCapacityUpdateV1 {
                max_active_bytes: Some(1_048_576),
            },
            model_feed: ModelFeedSettingsRequestV1 {
                retrieval_enabled: Some(true),
                acknowledge_disclosure: Some(true),
            },
        }
    }

    #[test]
    fn valid_bundle_passes() {
        assert!(bundle().valid());
    }

    #[test]
    fn invalid_schema_rejected() {
        let mut value = bundle();
        value.schema = "hiroute.configuration-bundle/v2".into();
        assert!(!value.valid());
    }

    #[test]
    fn invalid_worker_concurrency_rejected() {
        let mut value = bundle();
        value.worker_settings.max_concurrent = 0;
        assert!(!value.valid());
    }

    #[test]
    fn zero_capacity_rejected() {
        let mut value = bundle();
        value.observation_capacity.max_active_bytes = Some(0);
        assert!(!value.valid());
    }

    #[test]
    fn null_capacity_allowed() {
        let mut value = bundle();
        value.observation_capacity.max_active_bytes = None;
        assert!(value.valid());
    }

    #[test]
    fn bundle_serde_round_trip() {
        let value = bundle();
        let json = serde_json::to_string(&value).unwrap();
        let restored: ConfigurationBundleV1 = serde_json::from_str(&json).unwrap();
        assert!(restored.valid());
        assert_eq!(restored.worker_settings, value.worker_settings);
        assert_eq!(
            restored.observation_capacity.max_active_bytes,
            value.observation_capacity.max_active_bytes
        );
        assert_eq!(restored.model_feed, value.model_feed);
    }

    #[test]
    fn unknown_field_rejected() {
        let json = concat!(
            "{\"schema\":\"hiroute.configuration-bundle/v1\",",
            "\"worker_settings\":{\"schema\":\"hiroute.worker-settings/v1\",\"max_concurrent\":4},",
            "\"observation_capacity\":{\"max_active_bytes\":null},",
            "\"model_feed\":{\"retrieval_enabled\":true,\"acknowledge_disclosure\":true},",
            "\"extra\":1}"
        );
        assert!(serde_json::from_str::<ConfigurationBundleV1>(json).is_err());
    }

    #[test]
    fn import_state_serializes_snake_case() {
        assert_eq!(
            serde_json::to_string(&ConfigurationImportStateV1::Applied).unwrap(),
            "\"applied\""
        );
        assert_eq!(
            serde_json::to_string(&ConfigurationImportStateV1::Unchanged).unwrap(),
            "\"unchanged\""
        );
        assert_eq!(
            serde_json::to_string(&ConfigurationImportStateV1::Failed).unwrap(),
            "\"failed\""
        );
    }

    #[test]
    fn import_item_error_code_presence() {
        let json = serde_json::to_string(&unchanged()).unwrap();
        assert!(!json.contains("error_code"));
        let failed_item = ConfigurationImportItemV1 {
            state: ConfigurationImportStateV1::Failed,
            error_code: Some(EventErrorCode::ResponseMismatch),
        };
        let json = serde_json::to_string(&failed_item).unwrap();
        assert!(json.contains("\"error_code\":\"response_mismatch\""));
    }
}
