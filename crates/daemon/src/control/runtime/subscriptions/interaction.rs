//! System authorization UI belongs only to the current explicit check dispatch.
//! Transaction execution is synchronous; background maintenance and startup recovery have
//! no marker, even when they recover an operation originally created by a user.
use std::cell::RefCell;

use hiroute_domain::{OperationId, OperationV1};

thread_local! {
    static EXPLICIT_CHECK: RefCell<Option<OperationId>> = const { RefCell::new(None) };
}

pub(in crate::control::runtime) struct ExplicitSubscriptionCheck {
    previous: Option<OperationId>,
}
impl ExplicitSubscriptionCheck {
    pub(in crate::control::runtime) fn from_accepted(operation: &OperationV1) -> Self {
        let explicit = matches!(
            operation.idempotency.principal.as_str(),
            "interactive-user" | "desktop"
        ) && operation
            .plan
            .external()
            .iter()
            .any(hiroute_domain::is_subscription_check_effect);
        Self::enter(explicit.then(|| operation.operation_id.clone()))
    }
    fn enter(operation: Option<OperationId>) -> Self {
        let previous = EXPLICIT_CHECK.with(|current| current.replace(operation));
        Self { previous }
    }
}
impl Drop for ExplicitSubscriptionCheck {
    fn drop(&mut self) {
        EXPLICIT_CHECK.with(|current| current.replace(self.previous.take()));
    }
}

pub(super) fn is_explicit_check(operation: &OperationId) -> bool {
    EXPLICIT_CHECK.with(|current| current.borrow().as_ref() == Some(operation))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn interaction_is_exact_scoped_and_does_not_reach_background_or_recovery() {
        let first = OperationId::parse(format!("op_{}", "a".repeat(32))).unwrap();
        let second = OperationId::parse(format!("op_{}", "b".repeat(32))).unwrap();
        assert!(!is_explicit_check(&first));
        {
            let _explicit = ExplicitSubscriptionCheck::enter(Some(first.clone()));
            assert!(is_explicit_check(&first));
            assert!(!is_explicit_check(&second));
            std::thread::scope(|scope| {
                scope
                    .spawn(|| assert!(!is_explicit_check(&first)))
                    .join()
                    .unwrap();
            });
            {
                let _maintenance = ExplicitSubscriptionCheck::enter(None);
                assert!(!is_explicit_check(&first));
            }
            assert!(is_explicit_check(&first));
        }
        assert!(!is_explicit_check(&first));
    }
}
