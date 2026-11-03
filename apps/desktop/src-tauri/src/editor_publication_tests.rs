//! Real native Session -> protected Application -> daemon/store publication path.
use super::*;

fn editor_input(
    action: &str,
    plan: &AgentPlanStatusV2,
    editor: &hiroute_domain::PlanEditorStateV2,
    draft_revision: Option<u64>,
) -> crate::session::EditorInput {
    serde_json::from_value(serde_json::json!({
        "action": action,
        "plan_id": plan.agent_plan_id,
        "draft_id": "draft/native-publication",
        "expected_head_revision": plan.head.head_revision,
        "expected_draft_revision": draft_revision,
        "editor": editor,
        "language": "en"
    }))
    .unwrap()
}

#[tokio::test]
async fn edited_draft_publish_uses_submitted_content_and_consumes_the_exact_draft() {
    let root = prepare();
    let mut session = open(root.path());
    let original = plan(&mut session).await;
    let mut editor = original
        .desired
        .editor(Some(original.model_alias.as_str().into()))
        .unwrap();
    editor.purpose = "saved draft before the final edit".into();
    let draft = session
        .preview_editor(editor_input("save_draft", &original, &editor, None))
        .await
        .unwrap();
    assert!(!draft.requires_confirmation());
    session
        .finish_native_confirmation(draft, true)
        .await
        .unwrap();
    let snapshot = session.snapshot().await.unwrap();
    let draft_revision = snapshot
        .catalog
        .drafts
        .iter()
        .find(|draft| draft.draft_id == "draft/native-publication")
        .unwrap()
        .revision;

    editor.purpose = "content edited immediately before ordinary Publish".into();
    let publication = session
        .preview_editor(editor_input(
            "publish",
            &original,
            &editor,
            Some(draft_revision),
        ))
        .await
        .unwrap();
    assert!(
        !publication.requires_confirmation(),
        "ordinary Publish has no hidden confirmation dialog"
    );
    let outcome = session
        .finish_native_confirmation(publication, true)
        .await
        .unwrap();
    assert_eq!(outcome.operation.unwrap().state, "succeeded");
    let current = plan(&mut session).await;
    assert_eq!(current.desired.purpose.as_str(), editor.purpose);
    assert_eq!(current.desired.strategy, original.desired.strategy);
    assert_eq!(current.model_alias, original.model_alias);
    assert!(
        session
            .snapshot()
            .await
            .unwrap()
            .catalog
            .drafts
            .iter()
            .all(|draft| draft.draft_id != "draft/native-publication")
    );
    drop(session);
    let mut restarted = open(root.path());
    assert_eq!(
        plan(&mut restarted).await.desired.purpose.as_str(),
        editor.purpose
    );
    assert!(
        restarted
            .snapshot()
            .await
            .unwrap()
            .catalog
            .drafts
            .iter()
            .all(|draft| draft.draft_id != "draft/native-publication")
    );
}
#[tokio::test]
async fn native_default_preserves_custom_text_across_publication_and_restart() {
    use hiroute_domain::*;
    let current_guidance: serde_json::Value = serde_json::from_str(include_str!(
        "../../../../assets/decision-services/default-assessment-guidance.json"
    ))
    .unwrap();
    let root = prepare();
    let mut session = open(root.path());
    let original = plan(&mut session).await;
    let service: DecisionServiceV1 = serde_json::from_value(serde_json::json!({
        "id":"native-transition", "revision":1, "name":"Native transition fixture", "connection": {
            "kind":"system_one", "provider":"compatible", "model":"fixture", "endpoint":"https://decision.example/systemone",
            "timeout_ms":3000, "auth_header":{"name":"Authorization","value_secret_ref":"placeholder"}
        }
    })).unwrap();
    session
        .save_decision_service(
            DecisionServiceChangeV1 {
                id: service.id.clone(),
                expected_revision: 0,
                service: Some(service),
                input_slot: None,
            },
            Some(zeroize::Zeroizing::new("native-fixture".into())),
        )
        .await
        .unwrap();
    let service = session
        .decision_services()
        .await
        .unwrap()
        .services
        .remove(0);
    let mut editor = original
        .desired
        .editor(Some(original.model_alias.as_str().into()))
        .unwrap();
    editor.mode = PlanEditorMode::SmartSaving;
    editor.smart.economy = editor.candidates.clone();
    editor.smart.primary = editor.candidates.clone();
    editor.smart.classifier = ComplexityClassifierModeV1::DecisionService {
        service: Box::new(service),
    };
    let mut current = original.clone();
    for iteration in 0..2 {
        editor.purpose = format!("Native default publication {iteration}");
        let confirmation = session
            .preview_editor(editor_input("publish", &current, &editor, None))
            .await
            .unwrap();
        assert!(
            !confirmation.requires_confirmation(),
            "ordinary publish gained an extra review dialog"
        );
        session
            .finish_native_confirmation(confirmation, true)
            .await
            .unwrap();
        current = plan(&mut session).await;
        let scope = current.desired.assessment_scopes()[SMART_SAVING_SCOPE_ID];
        assert_eq!(
            scope.competence.standard,
            Some(AssessmentStandardV1::Default)
        );
        assert_eq!(
            scope.competence.criteria,
            // Editable defaults use the current guidance; stock_c_questions is frozen C v1.
            ["lowest_criterion", "middle_criterion", "highest_criterion"]
                .map(|key| current_guidance[key].as_str().unwrap().to_owned())
                .as_slice()
        );
        editor = current.desired.editor(None).unwrap();
    }
    editor.smart.judgment.competence.standard = Some(AssessmentStandardV1::Custom);
    editor.smart.judgment.competence.instructions =
        "  Preserve custom text.\nSecond line.  ".into();
    editor.smart.judgment.competence.criteria[1].push_str("  ");
    let custom = editor.smart.judgment.competence.clone();
    for iteration in 0..2 {
        editor.purpose = format!("Native custom publication {iteration}");
        let confirmation = session
            .preview_editor(editor_input("publish", &current, &editor, None))
            .await
            .unwrap();
        session
            .finish_native_confirmation(confirmation, true)
            .await
            .unwrap();
        current = plan(&mut session).await;
        assert_eq!(
            current.desired.assessment_scopes()[SMART_SAVING_SCOPE_ID].competence,
            custom
        );
        editor = current.desired.editor(None).unwrap();
    }
    let before = current.head.reference.clone();
    drop(session);
    let mut restarted = open(root.path());
    let reopened = plan(&mut restarted).await;
    assert_eq!(reopened.head.reference, before);
    assert_eq!(
        reopened.desired.assessment_scopes()[SMART_SAVING_SCOPE_ID].competence,
        custom
    );
}
