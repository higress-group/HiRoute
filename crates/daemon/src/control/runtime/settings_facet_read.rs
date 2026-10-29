//! Owned Control snapshot and final dependency fence for one Skill facet.
use super::*;
use hiroute_application_api::AgentSettingsStatusRequestV2;
use hiroute_diagnostics::agent_read::{
    AgentReadCategory as Category, AgentReadStage as Stage, Timer,
};
use hiroute_domain::{AgentNativeTargetV1, AgentSettingsFacet, ManagedCollaborationSkill};
use hiroute_local_storage::{
    AgentLocatorScope, AgentLocatorWitness, AgentNativeTargetWitness, AgentReadStamp,
    CapturedAgentLocator, SucceededAgentOperationV1, control::PendingAgentStatus,
};

fn category(facet: AgentSettingsFacet) -> Category {
    match facet {
        AgentSettingsFacet::Collaboration => Category::Collaboration,
        AgentSettingsFacet::ManagementSkill => Category::Management,
        AgentSettingsFacet::Model => Category::Model,
    }
}

pub(super) enum FacetSelection {
    Found(Arc<SucceededAgentOperationV1>),
    Pending(PendingAgentStatus),
    Absent,
}

pub(super) struct FacetRead {
    pub(super) selection: FacetSelection,
    /// The outer option means this dependency was consumed.
    pub(super) skill: Option<Option<ManagedCollaborationSkill>>,
    pub(super) root: String,
    pub(super) target: String,
    witness: AgentLocatorWitness,
    native_witness: Option<AgentNativeTargetWitness>,
    stamp: AgentReadStamp,
    category: Category,
}

impl LocalControlAdapter {
    pub(super) fn capture_settings_facet(
        &self,
        request: &AgentSettingsStatusRequestV2,
        facet: AgentSettingsFacet,
    ) -> Result<FacetRead, ControlReadError> {
        let diagnostics = self.agent_read_diagnostics();
        let category = category(facet);
        let root = self.settings_skill_root(&request.context_id, facet)?;
        let needs_target = self.settings_agent_for_context(&request.context_id)
            == Some(super::settings_facts::SettingsAgentClass::Hermes)
            && request.native_target.is_none();
        let (capture, skill, target, stamp) = {
            let waiting = Timer::start(&diagnostics, category, Stage::LockWait);
            let stores = self.stores_lock().map_err(super::map_port)?;
            drop(waiting);
            let _held = Timer::start(&diagnostics, category, Stage::LockHeld);
            let _copy = Timer::start(&diagnostics, category, Stage::SnapshotCopy);
            let ((capture, skill, target), stamp) = stores
                .capture_agent_read(false, |control, _| {
                    let mut locator = Timer::start(&diagnostics, category, Stage::Locator);
                    let capture = control.capture_agent_locator(
                        &WorkspaceId::default(),
                        AgentLocatorScope::ContextFacet {
                            context_id: &request.context_id,
                            facet,
                        },
                    )?;
                    super::agent_read_diagnostics::count_locator(&mut locator, &capture);
                    drop(locator);
                    let skill = if request.include_skills
                        || matches!(capture, CapturedAgentLocator::Found(_))
                    {
                        Some(control.skill_installation(&WorkspaceId::default(), &root)?)
                    } else {
                        None
                    };
                    let target = if needs_target {
                        Some(control.capture_agent_native_target(
                            &WorkspaceId::default(),
                            &request.context_id,
                        )?)
                    } else {
                        None
                    };
                    Ok((capture, skill, target))
                })
                .map_err(super::map_port)?;
            (capture, skill, target, stamp)
        };
        let _verify = Timer::start(&diagnostics, category, Stage::Verify);
        let witness = capture.witness().map_err(super::map_port)?;
        let native_witness = target.as_ref().map(|target| target.witness());
        let target: Option<AgentNativeTargetV1> = match target {
            Some(target) => Some(
                target
                    .verify()
                    .map_err(super::map_port)?
                    .ok_or(ControlReadError::NotFound)?,
            ),
            None => request.native_target.clone(),
        };
        let (_, file_target) =
            self.settings_skill_resource(&request.context_id, target.as_ref(), facet)?;
        let selection = match capture {
            CapturedAgentLocator::Found(capture) => {
                FacetSelection::Found(capture.verify().map_err(super::map_port)?)
            }
            CapturedAgentLocator::Pending(capture, _) => {
                FacetSelection::Pending(capture.verify().map_err(super::map_port)?)
            }
            CapturedAgentLocator::AbsentWithCompleteCoverage(_) => FacetSelection::Absent,
        };
        Ok(FacetRead {
            selection,
            skill,
            root,
            target: file_target,
            witness,
            native_witness,
            stamp,
            category,
        })
    }

    pub(super) fn fence_settings_facet(&self, read: &FacetRead) -> Result<(), ControlReadError> {
        let diagnostics = self.agent_read_diagnostics();
        let waiting = Timer::start(&diagnostics, read.category, Stage::LockWait);
        let stores = self.stores_lock().map_err(super::map_port)?;
        drop(waiting);
        let _held = Timer::start(&diagnostics, read.category, Stage::LockHeld);
        let _fence = Timer::start(&diagnostics, read.category, Stage::Fence);
        stores
            .fence_agent_read(&read.stamp, |control, _| {
                let changed = || PortError::new(PortErrorCode::Conflict, "agent.facet.read-fence");
                if !read.witness.matches_current(control)? {
                    return Err(changed());
                }
                if let Some(target) = &read.native_witness
                    && !target.matches_current(control)?
                {
                    return Err(changed());
                }
                if let Some(expected) = &read.skill
                    && control.skill_installation(&WorkspaceId::default(), &read.root)? != *expected
                {
                    return Err(changed());
                }
                Ok(())
            })
            .map_err(super::map_port)
    }
}
