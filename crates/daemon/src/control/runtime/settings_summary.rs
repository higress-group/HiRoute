//! Metadata-only Home hints; no native, grant, Skill, or execution proof.
use super::*;
use hiroute_application_api::{
    AgentSettingsFacetSummaryV1, AgentSettingsRecordedActionV1, AgentSettingsSummaryCoverageV1,
    AgentSettingsSummaryEntryV1, AgentSettingsSummaryRequestV1, AgentSettingsSummaryStateV1,
    AgentSettingsSummaryV1,
};
use hiroute_diagnostics::agent_read::{
    AgentReadCategory as Category, AgentReadStage as Stage, Timer,
};
use hiroute_local_storage::AgentLocatorSummary;

fn summary(result: PortResult<AgentLocatorSummary>, known: bool) -> AgentSettingsFacetSummaryV1 {
    use AgentSettingsRecordedActionV1 as Action;
    use AgentSettingsSummaryCoverageV1 as Coverage;
    use AgentSettingsSummaryStateV1 as State;
    let (state, coverage, action) = if !known {
        (State::NotReady, Coverage::NotReady, None)
    } else {
        match result {
            Ok(AgentLocatorSummary::Absent) => (State::Absent, Coverage::Complete, None),
            Ok(AgentLocatorSummary::RecordedConfigure) => {
                (State::Recorded, Coverage::Complete, Some(Action::Configure))
            }
            Ok(AgentLocatorSummary::RecordedRestore) => {
                (State::Recorded, Coverage::Complete, Some(Action::Restore))
            }
            Ok(AgentLocatorSummary::Pending) => (State::Pending, Coverage::Complete, None),
            Ok(AgentLocatorSummary::NeedsAttention) => {
                (State::NeedsAttention, Coverage::Complete, None)
            }
            Err(_) => (State::Unavailable, Coverage::Unavailable, None),
        }
    };
    AgentSettingsFacetSummaryV1 {
        state,
        coverage,
        action,
        native_checked: false,
    }
}

impl LocalControlAdapter {
    pub(super) fn agent_settings_summary(
        &self,
        request: &AgentSettingsSummaryRequestV1,
    ) -> Result<AgentSettingsSummaryV1, ControlReadError> {
        let diagnostics = self.agent_read_diagnostics();
        let _total = Timer::start(&diagnostics, Category::HomeSummary, Stage::Total);
        if !request.valid() {
            return Err(ControlReadError::Denied);
        }
        let known: Vec<_> = request
            .context_ids
            .iter()
            .map(|context| self.settings_agent_for_context(context).is_some())
            .collect();
        let capture = || {
            let waiting = Timer::start(&diagnostics, Category::HomeSummary, Stage::LockWait);
            let stores = self.stores_lock().map_err(super::map_port)?;
            drop(waiting);
            let _held = Timer::start(&diagnostics, Category::HomeSummary, Stage::LockHeld);
            let collect = |control: &hiroute_local_storage::ControlStore| {
                let _locator = Timer::start(&diagnostics, Category::HomeSummary, Stage::Locator);
                let results = control
                    .agent_locator_summaries(&WorkspaceId::default(), &request.context_ids)?;
                let entries = request
                    .context_ids
                    .iter()
                    .zip(&known)
                    .zip(results)
                    .map(|((context, &known), [model, collaboration, management])| {
                        AgentSettingsSummaryEntryV1 {
                            context_id: context.clone(),
                            model: summary(model, known),
                            collaboration: summary(collaboration, known),
                            management_skill: summary(management, known),
                        }
                    })
                    .collect();
                Ok::<_, PortError>(AgentSettingsSummaryV1 { entries })
            };
            let copy = Timer::start(&diagnostics, Category::HomeSummary, Stage::SnapshotCopy);
            let (result, stamp) = stores
                .capture_agent_read(false, |control, _| collect(control))
                .map_err(super::map_port)?;
            drop(copy);
            let _fence = Timer::start(&diagnostics, Category::HomeSummary, Stage::Fence);
            stores
                .fence_agent_read(&stamp, |control, _| {
                    if collect(control)? != result {
                        return Err(PortError::new(
                            PortErrorCode::Conflict,
                            "agent.summary.read-fence",
                        ));
                    }
                    Ok(())
                })
                .map_err(super::map_port)?;
            Ok(result)
        };
        match capture() {
            Err(ControlReadError::SnapshotChanged) => capture(),
            result => result,
        }
    }
}
