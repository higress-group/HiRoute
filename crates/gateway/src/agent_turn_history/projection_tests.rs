use super::*;
use crate::agent_turn_history::projection::analyze_request;

#[test]
fn long_history_projects_only_latest_real_user_without_losing_boundaries_or_tool_status() {
    let (_directory, replay) = replay();
    let old = replay.store_content(b"old retained text").unwrap();
    let current_text = "latest untruncated 路由";
    let current = replay.store_content(current_text.as_bytes()).unwrap();
    let mut messages = (0..2_000)
        .map(|_| text(MessageRole::User, old.wire_marker()))
        .collect::<Vec<_>>();
    messages.push(text(MessageRole::User, current.wire_marker()));
    messages.push(tool_result("completed-tool", false));
    messages.push(tool_result("failed-tool", true));
    messages.push(text(MessageRole::Assistant, "later assistant output"));
    let request = request(messages);

    // The projection has no consumer for historical user text. Removing that
    // stream makes any accidental read fail; canonical reference validation
    // remains separate and must still reject the missing backing.
    replay.release_stream(&old).unwrap();
    let analyzed = analyze_request(&request, &replay).unwrap();
    assert_eq!(analyzed.message_count, 2_004);
    assert_eq!(analyzed.turns.len(), 2_001);
    let latest = analyzed.turns.last().unwrap();
    assert!(
        analyzed.turns[..2_000]
            .iter()
            .all(|turn| turn.user.is_empty())
    );
    assert_eq!(latest.user_index, 2_000);
    assert_eq!(
        latest.user,
        vec![VisibleContentPart::Text {
            text: current_text.into()
        }]
    );
    assert_eq!(
        analyzed.tool_results,
        vec![
            (2_001, "completed-tool".into(), ToolStatus::Completed),
            (2_002, "failed-tool".into(), ToolStatus::Failed),
        ]
    );
    assert!(replay.prevalidate(&[old]).is_err());
    replay.release_stream(&current).unwrap();
    assert!(matches!(
        analyze_request(&request, &replay),
        Err(AgentTurnHistoryError::Integrity)
    ));
}

#[test]
fn image_user_input_and_tool_only_messages_keep_existing_user_definition() {
    let (_directory, replay) = replay();
    let mut image = text(MessageRole::User, "current");
    image.content.push(ContentPart::Image {
        source: crate::server::core_runtime::model_ir::ImageSource::Url {
            url: "https://image.invalid/example.png".into(),
        },
    });
    let analyzed = analyze_request(
        &request(vec![image, tool_result("tool-only", false)]),
        &replay,
    )
    .unwrap();
    assert_eq!(analyzed.turns.len(), 1);
    assert_eq!(
        analyzed.turns.last().unwrap().user,
        vec![
            VisibleContentPart::Text {
                text: "current".into()
            },
            VisibleContentPart::Unavailable {
                source_kind: "image_url".into()
            },
        ]
    );
    let analyzed = analyze_request(&request(vec![tool_result("only", false)]), &replay).unwrap();
    assert_eq!(analyzed.turns.len(), 0);
    assert!(analyzed.turns.is_empty());
}
