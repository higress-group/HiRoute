//! Validated launch shape; paths remain caller-selected metadata, not execution authority.
use super::WorkerHarnessV1;
use crate::OperationValidationError;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum WorkerLaunchFormV1<'a> {
    AdapterAcp {
        adapter: &'a str,
        cli: &'a str,
        node: Option<&'a str>,
    },
    NativeAcp {
        cli: &'a str,
    },
}

impl<'a> WorkerLaunchFormV1<'a> {
    pub fn validate(
        harness: WorkerHarnessV1,
        adapter: Option<&'a str>,
        cli: &'a str,
        node: Option<&'a str>,
    ) -> Result<Self, OperationValidationError> {
        let invalid = || OperationValidationError::UnregisteredEffectPlan;
        if !absolute_path(cli)
            || adapter.is_some_and(|path| !absolute_path(path))
            || node.is_some_and(|path| !absolute_path(path))
        {
            return Err(invalid());
        }
        match harness {
            WorkerHarnessV1::CodexCli | WorkerHarnessV1::ClaudeCode => Ok(Self::AdapterAcp {
                adapter: adapter.ok_or_else(invalid)?,
                cli,
                node,
            }),
            WorkerHarnessV1::QoderCli if adapter.is_none() && node.is_none() => {
                Ok(Self::NativeAcp { cli })
            }
            WorkerHarnessV1::QoderCli => Err(invalid()),
        }
    }
}

fn absolute_path(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 4096
        && !value.contains(char::is_control)
        && std::path::Path::new(value).is_absolute()
}
