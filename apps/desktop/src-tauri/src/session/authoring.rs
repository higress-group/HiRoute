//! Typed editor actions. WebView supplies edits; native code owns preview, authority and Apply.
use super::*;
use hiroute_domain::{
    PLAN_DRAFT_SCHEMA_V1, PlanDraftActionV1, PlanDraftChangeV1, PlanDraftV1, PlanEditorStateV2,
    PlanLifecycleV1,
};
#[derive(Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EditorAction {
    Publish,
    SaveDraft,
    DiscardDraft,
    Disable,
    Enable,
    Delete,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EditorInput {
    pub action: EditorAction,
    pub plan_id: Option<String>,
    pub draft_id: String,
    pub expected_head_revision: Option<u64>,
    pub expected_draft_revision: Option<u64>,
    pub editor: PlanEditorStateV2,
    pub language: String,
}
impl EditorAction {
    pub(super) fn requires_confirmation(&self) -> bool {
        match self {
            Self::Publish | Self::SaveDraft | Self::Enable => false,
            Self::DiscardDraft | Self::Disable | Self::Delete => true,
        }
    }
}
impl Session {
    pub async fn preview_reference_refresh(
        &mut self,
        language: String,
    ) -> Result<Confirmation, DesktopFailure> {
        if self.confirmation.is_open() {
            return Err("CONFIRMATION_ALREADY_OPEN".into());
        }
        if !matches!(language.as_str(), "zh" | "en") {
            return Err("INVALID_LANGUAGE".into());
        }
        let snapshot = self.snapshot().await?;
        if !snapshot.trusted_authority || !snapshot.service.mutation_available {
            return Err("TRUSTED_AUTHORITY_UNAVAILABLE".into());
        }
        let change = serde_json::to_value(PublicationCheckpointChangeV1 {
            schema: PUBLICATION_CHECKPOINT_CHANGE_SCHEMA_V1.into(),
        })
        .map_err(|_| "REQUEST_INVALID")?;
        let intent = IntentEvidence {
            schema: "hiroute.desktop-plan-intent/v1".into(),
            digest: CanonicalDigest::of(&change).map_err(|_| "REQUEST_INVALID")?,
        };
        let retry = self.retry_key(&intent).await?;
        let preview: PublicationCheckpointPreviewV1 = query(
            &self.client,
            "PreviewAgentPlanChange",
            &serde_json::json!({"change":change}),
        )
        .await?;
        if preview.schema != PUBLICATION_CHECKPOINT_PREVIEW_SCHEMA_V1 {
            return Err("PREVIEW_INVALID".into());
        }
        let label = if language == "en" {
            "Update references"
        } else {
            "更新引用状态"
        }
        .to_owned();
        Ok(Confirmation {
            requires_confirmation: false,
            action_label: label.clone(),
            permit: self.confirmation.begin()?,
            intent,
            previous_name: String::new(),
            input: RenameInput {
                plan_id: "publication/current".into(),
                display_name: label.clone(),
                language,
            },
            message_override: Some(label),
            request: NativePlanApply {
                change,
                accept_digest: preview.change_digest,
                expected_revisions: preview.expected_revisions,
                idempotency_key: retry.unwrap_or(crate::random_id()?),
            },
        })
    }
    pub async fn preview_editor(
        &mut self,
        input: EditorInput,
    ) -> Result<Confirmation, DesktopFailure> {
        if self.confirmation.is_open() {
            return Err("CONFIRMATION_ALREADY_OPEN".into());
        }
        if !matches!(input.language.as_str(), "zh" | "en") {
            return Err("INVALID_LANGUAGE".into());
        }
        hiroute_domain::AgentPlanId::parse(&input.draft_id).map_err(|_| "DRAFT_ID_INVALID")?;
        input
            .editor
            .validate_draft()
            .map_err(|_| "EDITOR_INVALID")?;
        let snapshot = self.snapshot().await?;
        if !snapshot.trusted_authority || !snapshot.service.mutation_available {
            return Err("TRUSTED_AUTHORITY_UNAVAILABLE".into());
        }
        if let Some(error) = snapshot.catalog_error {
            return Err(error);
        }
        let plan = input
            .plan_id
            .as_ref()
            .map(|id| {
                snapshot
                    .catalog
                    .plans
                    .iter()
                    .find(|p| p.agent_plan_id.as_str() == id)
                    .ok_or("PLAN_NOT_FOUND")
            })
            .transpose()?;
        let draft = snapshot
            .catalog
            .drafts
            .iter()
            .find(|d| d.draft_id == input.draft_id);
        validate_editor_base(
            input.expected_head_revision,
            input.expected_draft_revision,
            plan.map(|p| p.head.head_revision),
            draft.map(|d| d.revision),
        )?;
        let previous_name = plan
            .map(|p| p.desired.display_name.as_str().to_owned())
            .unwrap_or_default();
        let change = match input.action {
            EditorAction::Publish => {
                if plan.is_some_and(|p| input.editor.effective().ok().as_ref() == Some(&p.desired))
                {
                    return Err("PLAN_UNCHANGED".into());
                }
                let target = match plan {
                    Some(plan) => PlanContentTargetV2::Update {
                        plan_id: plan.agent_plan_id.clone(),
                        expected_head_revision: plan.head.head_revision,
                    },
                    None => PlanContentTargetV2::Create {
                        creation_key: input.draft_id.clone(),
                    },
                };
                let consumed_draft = draft
                    .filter(|d| {
                        d.plan_id.as_ref() == plan.map(|p| &p.agent_plan_id)
                            && d.base_head_revision == plan.map(|p| p.head.head_revision)
                    })
                    .map(|d| PlanDraftRefV1 {
                        draft_id: d.draft_id.clone(),
                        revision: d.revision,
                    });
                serde_json::to_value(PlanContentChangeV2 {
                    schema: PLAN_CONTENT_CHANGE_SCHEMA_V2.into(),
                    target,
                    editor: input.editor.clone(),
                    consumed_draft,
                })
                .map_err(|_| "EDITOR_INVALID")?
            }
            EditorAction::SaveDraft | EditorAction::DiscardDraft => {
                let action = if matches!(input.action, EditorAction::DiscardDraft) {
                    PlanDraftActionV1::Discard
                } else {
                    PlanDraftActionV1::Save {
                        draft: Box::new(PlanDraftV1 {
                            schema: PLAN_DRAFT_SCHEMA_V1.into(),
                            workspace_id: hiroute_domain::WorkspaceId::default(),
                            draft_id: input.draft_id.clone(),
                            revision: draft
                                .map(|d| d.revision)
                                .unwrap_or(0)
                                .checked_add(1)
                                .ok_or("DRAFT_INVALID")?,
                            plan_id: plan.map(|p| p.agent_plan_id.clone()),
                            base_head_revision: plan.map(|p| p.head.head_revision),
                            editor: input.editor.clone(),
                        }),
                    }
                };
                serde_json::to_value(PlanDraftChangeV1 {
                    schema: PLAN_DRAFT_CHANGE_SCHEMA_V1.into(),
                    workspace_id: hiroute_domain::WorkspaceId::default(),
                    draft_id: input.draft_id.clone(),
                    expected_revision: draft.map(|d| d.revision),
                    action,
                })
                .map_err(|_| "EDITOR_INVALID")?
            }
            EditorAction::Disable | EditorAction::Enable | EditorAction::Delete => {
                let plan = plan.ok_or("PLAN_NOT_FOUND")?;
                serde_json::to_value(PlanLifecycleChangeV1 {
                    schema: PLAN_LIFECYCLE_CHANGE_SCHEMA_V1.into(),
                    plan_id: plan.agent_plan_id.clone(),
                    expected_head_revision: plan.head.head_revision,
                    status: match input.action {
                        EditorAction::Disable => PlanLifecycleV1::Disabled,
                        EditorAction::Enable => PlanLifecycleV1::Enabled,
                        _ => PlanLifecycleV1::Deleted,
                    },
                })
                .map_err(|_| "EDITOR_INVALID")?
            }
        };
        let intent = IntentEvidence {
            schema: "hiroute.desktop-plan-intent/v1".into(),
            digest: CanonicalDigest::of(&change).map_err(|_| "EDITOR_INVALID")?,
        };
        let retry = self.retry_key(&intent).await?;
        let response: Value = if matches!(input.action, EditorAction::Publish) {
            Value::Null
        } else {
            query(
                &self.client,
                "PreviewAgentPlanChange",
                &serde_json::json!({"change": change}),
            )
            .await?
        };
        let en = input.language == "en";
        let (digest, revisions, summary) = match input.action {
            EditorAction::Publish => {
                let typed: PlanContentChangeV2 =
                    serde_json::from_value(change.clone()).map_err(|_| "EDITOR_INVALID")?;
                let revisions = snapshot.service.revisions.clone();
                let digest = plan_content_confirmation_digest(&typed, &revisions)
                    .map_err(|_| "EDITOR_INVALID")?;
                (digest, revisions, String::new())
            }
            EditorAction::SaveDraft | EditorAction::DiscardDraft => {
                let preview: PlanDraftPreviewV1 =
                    serde_json::from_value(response).map_err(|_| "PREVIEW_INVALID")?;
                if preview.schema != PLAN_DRAFT_PREVIEW_SCHEMA_V1 {
                    return Err("PREVIEW_INVALID".into());
                }
                let verb = match (&input.action, en) {
                    (EditorAction::SaveDraft, true) => "Save draft",
                    (EditorAction::SaveDraft, false) => "保存草稿",
                    (_, true) => "Discard draft",
                    _ => "丢弃草稿",
                };
                (
                    preview.change_digest,
                    preview.expected_revisions,
                    format!(
                        "{verb}: {}\n{}",
                        input.editor.display_name,
                        if en {
                            "This changes only the saved draft."
                        } else {
                            "只更新保存的草稿。"
                        }
                    ),
                )
            }
            _ => {
                let preview: PlanLifecyclePreviewV1 =
                    serde_json::from_value(response).map_err(|_| "PREVIEW_INVALID")?;
                if preview.schema != PLAN_LIFECYCLE_PREVIEW_SCHEMA_V1 {
                    return Err("PREVIEW_INVALID".into());
                }
                let consequence = match input.action {
                    EditorAction::Delete => {
                        if en {
                            "This route will be deleted. New calls will no longer be able to use it."
                        } else {
                            "将删除此路由，后续调用将无法再使用它。"
                        }
                    }
                    EditorAction::Disable => {
                        if en {
                            "New calls will no longer use this route. In-flight calls keep their current version."
                        } else {
                            "后续调用将无法再使用此路由。在途请求继续使用原版本。"
                        }
                    }
                    _ => {
                        if en {
                            "This route will become available for new calls."
                        } else {
                            "此路由将恢复供后续调用使用。"
                        }
                    }
                };
                let summary = format!("{}\n{consequence}", input.editor.display_name);
                (preview.change_digest, preview.expected_revisions, summary)
            }
        };
        let message = format!(
            "{summary}\n\n{}",
            if en {
                "Apply this change?"
            } else {
                "确认应用此变更？"
            }
        );
        let permit = self.confirmation.begin()?;
        let labels = match input.action {
            EditorAction::Publish => ("发布", "Publish"),
            EditorAction::SaveDraft => ("保存草稿", "Save draft"),
            EditorAction::DiscardDraft => ("丢弃草稿", "Discard draft"),
            EditorAction::Disable => ("停用路由", "Disable route"),
            EditorAction::Enable => ("恢复调用", "Enable route"),
            EditorAction::Delete => ("删除路由", "Delete route"),
        };
        Ok(Confirmation {
            requires_confirmation: input.action.requires_confirmation(),
            action_label: if en { labels.1 } else { labels.0 }.into(),
            permit,
            intent,
            previous_name,
            input: RenameInput {
                plan_id: input.plan_id.unwrap_or(input.draft_id),
                display_name: input.editor.display_name,
                language: input.language,
            },
            message_override: Some(message),
            request: NativePlanApply {
                change,
                accept_digest: digest,
                expected_revisions: revisions,
                idempotency_key: retry.unwrap_or(crate::random_id()?),
            },
        })
    }
}
// Never refresh a stale WebView edit onto the latest server revision silently.
fn validate_editor_base(
    expected_head: Option<u64>,
    expected_draft: Option<u64>,
    current_head: Option<u64>,
    current_draft: Option<u64>,
) -> Result<(), DesktopFailure> {
    if expected_head != current_head {
        return Err("PLAN_HEAD_STALE".into());
    }
    if expected_draft != current_draft {
        return Err("DRAFT_REVISION_STALE".into());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn editor_rejects_concurrent_publish_save_discard_and_id_collision() {
        for (expected_head, expected_draft, current_head, current_draft) in [
            (Some(1), None, Some(2), None),
            (Some(1), Some(1), Some(1), Some(2)),
            (None, Some(1), None, None),
            (None, None, None, Some(1)),
        ] {
            assert!(
                validate_editor_base(expected_head, expected_draft, current_head, current_draft)
                    .is_err()
            );
        }
        assert!(validate_editor_base(None, None, None, None).is_ok());
        assert!(validate_editor_base(Some(2), Some(3), Some(2), Some(3)).is_ok());
    }
}

#[cfg(test)]
mod confirmation_policy_tests {
    use super::*;
    #[test]
    fn explicit_editor_submissions_only_prompt_for_destructive_actions() {
        for action in [
            EditorAction::Publish,
            EditorAction::SaveDraft,
            EditorAction::Enable,
        ] {
            assert!(!action.requires_confirmation());
        }
        for action in [
            EditorAction::Delete,
            EditorAction::Disable,
            EditorAction::DiscardDraft,
        ] {
            assert!(action.requires_confirmation());
        }
    }
}
