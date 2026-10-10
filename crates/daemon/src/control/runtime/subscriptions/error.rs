//! Error translation at the subscription runtime boundary.

use hiroute_application::{
    control::ComputeManagementControlError, subscriptions::SubscriptionPreparationError,
};
use hiroute_cpa_bridge::CpaLifecycleError;
use hiroute_domain::{PortError, PortErrorCode};

pub(super) fn map_preparation(error: SubscriptionPreparationError) -> PortError {
    match error {
        SubscriptionPreparationError::NeedsApproval => conflict("subscription.approval.required"),
        SubscriptionPreparationError::NeedsCredential
        | SubscriptionPreparationError::InvalidModelSelection
        | SubscriptionPreparationError::InvalidRequest => invalid("subscription.prepare.invalid"),
    }
}

pub(super) fn map_cpa(error: CpaLifecycleError) -> PortError {
    if let Some(failure) = error.subscription_failure() {
        return failure;
    }
    match error {
        CpaLifecycleError::BorrowedCodexAuthSourceChanged
        | CpaLifecycleError::BorrowedClaudeAuthSourceChanged
        | CpaLifecycleError::ManagedOAuthAccountChanged => conflict("subscription.source.changed"),
        _ => unavailable("subscription.cpa.unavailable"),
    }
}

pub(super) fn map_port(error: PortError) -> ComputeManagementControlError {
    match error.code {
        PortErrorCode::NotFound => ComputeManagementControlError::NotFound,
        PortErrorCode::Conflict => ComputeManagementControlError::Conflict,
        PortErrorCode::Unavailable => ComputeManagementControlError::Unavailable,
        PortErrorCode::Corrupt => ComputeManagementControlError::Corrupt,
        PortErrorCode::InvalidData | PortErrorCode::PermissionDenied => {
            ComputeManagementControlError::Invalid
        }
        _ => ComputeManagementControlError::Unavailable,
    }
}

pub(super) fn control_error_to_port(error: ComputeManagementControlError) -> PortError {
    match error {
        ComputeManagementControlError::NotFound => not_found("subscription.not-found"),
        ComputeManagementControlError::Conflict => conflict("subscription.conflict"),
        ComputeManagementControlError::Corrupt => invalid("subscription.corrupt"),
        ComputeManagementControlError::Invalid => invalid("subscription.invalid"),
        _ => unavailable("subscription.unavailable"),
    }
}

pub(super) fn invalid(context: &'static str) -> PortError {
    PortError::new(PortErrorCode::InvalidData, context)
}

pub(super) fn conflict(context: &'static str) -> PortError {
    PortError::new(PortErrorCode::Conflict, context)
}

pub(super) fn not_found(context: &'static str) -> PortError {
    PortError::new(PortErrorCode::NotFound, context)
}

pub(super) fn unavailable(context: &'static str) -> PortError {
    PortError::new(PortErrorCode::Unavailable, context)
}
