use super::{
    LocalControlAdapter,
    delegation_task_queries::{authorize, run_view},
    delegation_worker::DelegationCallerContext,
};
use hiroute_application_api::{DelegationResultRequestV1, DelegationResultV1};
use hiroute_domain::delegation::{DelegationErrorV1, DelegationRuntimePort};
use hiroute_observation::managed_text::{
    CHUNK_BYTES, ManagedTextError, ManagedTextRef, ManagedTextScope, ManagedTextState, PAGE_BYTES,
};

impl LocalControlAdapter {
    pub(super) fn read_delegation_result(
        &self,
        caller: &DelegationCallerContext,
        request: &DelegationResultRequestV1,
    ) -> Result<DelegationResultV1, DelegationErrorV1> {
        let workspace = caller.workspace_id();
        let run = DelegationRuntimePort::run(self, workspace, &request.run_id)?
            .ok_or(DelegationErrorV1::NotFound)?;
        let task = DelegationRuntimePort::task(self, workspace, &run.task_id)?
            .ok_or(DelegationErrorV1::NotFound)?;
        authorize(caller, &task, &run)?;
        let mut result = DelegationResultV1 {
            schema: "hiroute.delegation-result/v1".into(),
            run: run_view(&task, &run),
            text: None,
            next_offset: None,
            incomplete: run.result_incomplete,
        };
        let Some(body) = run.result_body else {
            return Ok(result);
        };
        let scope = ManagedTextScope {
            workspace_id: workspace.clone(),
            task_id: run.task_id,
            run_id: run.run_id,
        };
        let reference = ManagedTextRef {
            opaque_id: body.opaque_id,
            scope: scope.clone(),
            visibility_generation: body.visibility_generation,
            original_retention_deadline_ms: body.original_retention_deadline_ms,
            state: ManagedTextState::Complete,
        };
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_err(|_| DelegationErrorV1::StorageUnavailable)?
            .as_millis() as i64;
        let start = request.offset.unwrap_or(0) as usize;
        let limit = request.effective_max_bytes() as usize;
        let mut first = 0;
        let mut position = 0usize;
        let mut bytes = Vec::new();
        let mut has_more = false;
        loop {
            let page = self
                .delegation_observation
                .managed_text_read(&scope, &reference, first, PAGE_BYTES / CHUNK_BYTES, now)
                .map_err(map_result_read_error)?;
            if page.reference.state != ManagedTextState::Complete {
                return Err(DelegationErrorV1::StorageUnavailable);
            }
            let skip = start.saturating_sub(position).min(page.bytes.len());
            let count = (limit - bytes.len()).min(page.bytes.len() - skip);
            bytes.extend_from_slice(&page.bytes[skip..skip + count]);
            position += page.bytes.len();
            if bytes.len() == limit {
                has_more = skip + count < page.bytes.len() || page.next_chunk.is_some();
                break;
            }
            match page.next_chunk {
                Some(next) if next > first => first = next,
                None => break,
                _ => return Err(DelegationErrorV1::StorageUnavailable),
            }
        }
        if start > position {
            return Err(DelegationErrorV1::InvalidArguments);
        }
        // Byte offsets must start at a UTF-8 boundary; never corrupt a split multibyte suffix.
        let text = match String::from_utf8(bytes) {
            Ok(text) => text,
            Err(error) if has_more && error.utf8_error().error_len().is_none() => {
                let valid = error.utf8_error().valid_up_to();
                if valid == 0 {
                    return Err(DelegationErrorV1::InvalidArguments);
                }
                String::from_utf8(error.into_bytes()[..valid].to_vec())
                    .map_err(|_| DelegationErrorV1::StorageUnavailable)?
            }
            Err(_) => return Err(DelegationErrorV1::InvalidArguments),
        };
        let current = self
            .delegation_observation
            .managed_text_resolve(&scope, &reference, now)
            .map_err(map_result_read_error)?;
        if current.state != ManagedTextState::Complete
            || current.visibility_generation != reference.visibility_generation
        {
            return Err(DelegationErrorV1::ContentUnavailable);
        }
        if has_more {
            result.next_offset = Some(
                u32::try_from(start + text.len())
                    .map_err(|_| DelegationErrorV1::StorageUnavailable)?,
            );
        }
        result.text = Some(text);
        Ok(result)
    }
}

fn map_result_read_error(error: ManagedTextError) -> DelegationErrorV1 {
    match error {
        ManagedTextError::Unavailable | ManagedTextError::Stale => {
            DelegationErrorV1::ContentUnavailable
        }
        _ => DelegationErrorV1::StorageUnavailable,
    }
}
