use serde::{Deserialize, Serialize};
use thiserror::Error;

/// Exact model-native reasoning capability. The order of `profiles` is native low-to-high order,
/// not a cross-model effort scale.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum NativeReasoningCapabilityV1 {
    Fixed {
        profile: String,
    },
    Toggle {
        parameter: String,
    },
    Discrete {
        parameter: String,
        profiles: Vec<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        default_profile: Option<String>,
    },
    Budget {
        parameter: String,
        minimum_tokens: u32,
        maximum_tokens: u32,
        step_tokens: u32,
    },
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum ReasoningSelectionV1 {
    Toggle { enabled: bool },
    Profile { profile: String },
    Budget { tokens: u32 },
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum ExactNativeReasoningV1 {
    Fixed {
        profile: String,
        render_mode: ReasoningRenderModeV1,
    },
    Toggle {
        parameter: String,
        enabled: bool,
        render_mode: ReasoningRenderModeV1,
    },
    Profile {
        parameter: String,
        profile: String,
        render_mode: ReasoningRenderModeV1,
    },
    Budget {
        parameter: String,
        tokens: u32,
        render_mode: ReasoningRenderModeV1,
    },
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ReasoningRenderModeV1 {
    NoControlParameter,
    ExplicitNative,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DiscreteReasoningDefault {
    Lowest,
    Highest,
    RequireExplicit,
}

impl NativeReasoningCapabilityV1 {
    pub fn validate(&self) -> Result<(), ReasoningContractError> {
        match self {
            Self::Fixed { profile } => {
                validate_native_value(profile)?;
                reject_ultra(profile)
            }
            Self::Toggle { parameter } => validate_parameter(parameter),
            Self::Discrete {
                parameter,
                profiles,
                default_profile,
            } => {
                validate_parameter(parameter)?;
                if profiles.is_empty() || profiles.len() > 16 {
                    return Err(ReasoningContractError::InvalidCapability);
                }
                let mut seen = std::collections::BTreeSet::new();
                for profile in profiles {
                    validate_native_value(profile)?;
                    reject_ultra(profile)?;
                    if !seen.insert(profile) {
                        return Err(ReasoningContractError::InvalidCapability);
                    }
                }
                if default_profile
                    .as_ref()
                    .is_some_and(|profile| !seen.contains(profile))
                {
                    return Err(ReasoningContractError::InvalidCapability);
                }
                Ok(())
            }
            Self::Budget {
                parameter,
                minimum_tokens,
                maximum_tokens,
                step_tokens,
            } => {
                validate_parameter(parameter)?;
                if *minimum_tokens == 0
                    || minimum_tokens > maximum_tokens
                    || *step_tokens == 0
                    || (maximum_tokens - minimum_tokens) % step_tokens != 0
                {
                    Err(ReasoningContractError::InvalidCapability)
                } else {
                    Ok(())
                }
            }
        }
    }

    pub fn resolve(
        &self,
        selection: Option<&ReasoningSelectionV1>,
        discrete_default: DiscreteReasoningDefault,
    ) -> Result<ExactNativeReasoningV1, ReasoningContractError> {
        self.validate()?;
        match (self, selection) {
            (Self::Fixed { profile }, None) => Ok(ExactNativeReasoningV1::Fixed {
                profile: profile.clone(),
                render_mode: ReasoningRenderModeV1::NoControlParameter,
            }),
            (Self::Fixed { .. }, Some(_)) => Err(ReasoningContractError::SelectionNotSupported),
            (Self::Toggle { parameter }, Some(ReasoningSelectionV1::Toggle { enabled })) => {
                Ok(ExactNativeReasoningV1::Toggle {
                    parameter: parameter.clone(),
                    enabled: *enabled,
                    render_mode: ReasoningRenderModeV1::ExplicitNative,
                })
            }
            (Self::Toggle { .. }, None) => Err(ReasoningContractError::SelectionRequired),
            (
                Self::Discrete {
                    parameter,
                    profiles,
                    ..
                },
                selected,
            ) => {
                let profile = match selected {
                    Some(ReasoningSelectionV1::Profile { profile }) => {
                        reject_ultra(profile)?;
                        profiles
                            .iter()
                            .find(|candidate| *candidate == profile)
                            .ok_or(ReasoningContractError::UnsupportedProfile)?
                    }
                    None => match discrete_default {
                        DiscreteReasoningDefault::Lowest => enabled_profiles(profiles)
                            .next()
                            .ok_or(ReasoningContractError::NoEnabledProfile)?,
                        DiscreteReasoningDefault::Highest => enabled_profiles(profiles)
                            .next_back()
                            .ok_or(ReasoningContractError::NoEnabledProfile)?,
                        DiscreteReasoningDefault::RequireExplicit => {
                            return Err(ReasoningContractError::SelectionRequired);
                        }
                    },
                    Some(_) => return Err(ReasoningContractError::SelectionKindMismatch),
                };
                Ok(ExactNativeReasoningV1::Profile {
                    parameter: parameter.clone(),
                    profile: profile.clone(),
                    render_mode: ReasoningRenderModeV1::ExplicitNative,
                })
            }
            (
                Self::Budget {
                    parameter,
                    minimum_tokens,
                    maximum_tokens,
                    step_tokens,
                },
                Some(ReasoningSelectionV1::Budget { tokens }),
            ) => {
                if tokens < minimum_tokens
                    || tokens > maximum_tokens
                    || (tokens - minimum_tokens) % step_tokens != 0
                {
                    return Err(ReasoningContractError::BudgetOutOfRange);
                }
                Ok(ExactNativeReasoningV1::Budget {
                    parameter: parameter.clone(),
                    tokens: *tokens,
                    render_mode: ReasoningRenderModeV1::ExplicitNative,
                })
            }
            (Self::Budget { .. }, None) => Err(ReasoningContractError::BudgetRequired),
            (Self::Toggle { .. } | Self::Budget { .. }, Some(_)) => {
                Err(ReasoningContractError::SelectionKindMismatch)
            }
        }
    }
}

impl ExactNativeReasoningV1 {
    pub fn validate(&self) -> Result<(), ReasoningContractError> {
        match self {
            Self::Fixed {
                profile,
                render_mode,
            } => {
                validate_native_value(profile)?;
                reject_ultra(profile)?;
                require_render_mode(*render_mode, ReasoningRenderModeV1::NoControlParameter)
            }
            Self::Toggle {
                parameter,
                render_mode,
                ..
            } => {
                validate_parameter(parameter)?;
                require_render_mode(*render_mode, ReasoningRenderModeV1::ExplicitNative)
            }
            Self::Profile {
                parameter,
                profile,
                render_mode,
            } => {
                validate_parameter(parameter)?;
                validate_native_value(profile)?;
                reject_ultra(profile)?;
                require_render_mode(*render_mode, ReasoningRenderModeV1::ExplicitNative)
            }
            Self::Budget {
                parameter,
                tokens,
                render_mode,
            } => {
                validate_parameter(parameter)?;
                if *tokens == 0 {
                    Err(ReasoningContractError::BudgetOutOfRange)
                } else {
                    require_render_mode(*render_mode, ReasoningRenderModeV1::ExplicitNative)
                }
            }
        }
    }
}

fn enabled_profiles(profiles: &[String]) -> impl DoubleEndedIterator<Item = &String> {
    profiles.iter().filter(|profile| {
        !matches!(
            profile.to_ascii_lowercase().as_str(),
            "none" | "off" | "disabled"
        )
    })
}

fn require_render_mode(
    actual: ReasoningRenderModeV1,
    expected: ReasoningRenderModeV1,
) -> Result<(), ReasoningContractError> {
    if actual == expected {
        Ok(())
    } else {
        Err(ReasoningContractError::InvalidRenderMode)
    }
}

fn validate_parameter(value: &str) -> Result<(), ReasoningContractError> {
    if !value.is_empty()
        && value.len() <= 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b'-'))
    {
        Ok(())
    } else {
        Err(ReasoningContractError::InvalidCapability)
    }
}

fn validate_native_value(value: &str) -> Result<(), ReasoningContractError> {
    if !value.is_empty()
        && value.len() <= 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b'-'))
    {
        Ok(())
    } else {
        Err(ReasoningContractError::InvalidCapability)
    }
}

fn reject_ultra(value: &str) -> Result<(), ReasoningContractError> {
    if value.eq_ignore_ascii_case("ultra") {
        Err(ReasoningContractError::UltraIsNotModelEffort)
    } else {
        Ok(())
    }
}

#[derive(Clone, Debug, Error, Eq, PartialEq)]
pub enum ReasoningContractError {
    #[error("native reasoning capability is invalid")]
    InvalidCapability,
    #[error("reasoning selection is required for this native model")]
    SelectionRequired,
    #[error("an exact reasoning budget is required")]
    BudgetRequired,
    #[error("reasoning selection is not accepted by a fixed model")]
    SelectionNotSupported,
    #[error("reasoning selection kind does not match the native model")]
    SelectionKindMismatch,
    #[error("native reasoning profile is not supported by this model")]
    UnsupportedProfile,
    #[error("native reasoning capability contains no enabled discrete profile")]
    NoEnabledProfile,
    #[error("native reasoning budget is outside the exact model range")]
    BudgetOutOfRange,
    #[error("ultra is an Agent reasoning setting, not a model effort")]
    UltraIsNotModelEffort,
    #[error("native reasoning render mode does not match its control kind")]
    InvalidRenderMode,
}

#[cfg(test)]
mod routing_reasoning_tests {
    use super::*;

    #[test]
    fn routing_reasoning_resolves_only_exact_native_profiles() {
        let capability = NativeReasoningCapabilityV1::Discrete {
            parameter: "reasoning_effort".into(),
            profiles: vec!["low".into(), "medium".into(), "high".into()],
            default_profile: None,
        };
        assert_eq!(
            capability
                .resolve(None, DiscreteReasoningDefault::Lowest)
                .unwrap(),
            ExactNativeReasoningV1::Profile {
                parameter: "reasoning_effort".into(),
                profile: "low".into(),
                render_mode: ReasoningRenderModeV1::ExplicitNative,
            }
        );
        assert_eq!(
            capability
                .resolve(None, DiscreteReasoningDefault::Highest)
                .unwrap(),
            ExactNativeReasoningV1::Profile {
                parameter: "reasoning_effort".into(),
                profile: "high".into(),
                render_mode: ReasoningRenderModeV1::ExplicitNative,
            }
        );
        assert_eq!(
            capability
                .resolve(
                    Some(&ReasoningSelectionV1::Profile {
                        profile: "ultra".into(),
                    }),
                    DiscreteReasoningDefault::RequireExplicit,
                )
                .unwrap_err(),
            ReasoningContractError::UltraIsNotModelEffort
        );
    }

    #[test]
    fn routing_reasoning_budget_has_no_implicit_default() {
        let capability = NativeReasoningCapabilityV1::Budget {
            parameter: "thinking_budget".into(),
            minimum_tokens: 512,
            maximum_tokens: 4096,
            step_tokens: 512,
        };
        assert_eq!(
            capability
                .resolve(None, DiscreteReasoningDefault::Lowest)
                .unwrap_err(),
            ReasoningContractError::BudgetRequired
        );
    }

    #[test]
    fn routing_reasoning_template_skips_disabled_discrete_values() {
        let capability = NativeReasoningCapabilityV1::Discrete {
            parameter: "reasoning_effort".into(),
            profiles: vec!["disabled".into(), "low".into(), "high".into()],
            default_profile: None,
        };
        assert!(matches!(
            capability
                .resolve(None, DiscreteReasoningDefault::Lowest)
                .unwrap(),
            ExactNativeReasoningV1::Profile { profile, .. } if profile == "low"
        ));
    }
}
