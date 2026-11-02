use super::MISSING_BLOBS_SQL;
use crate::{DigestAuthority, LocalObservationStore};
use hiroute_domain::WorkspaceId;
use rusqlite::{Connection, StatementStatus, params};

fn query(db: &Connection, sql: &str) -> (Vec<(String, String)>, i32) {
    let mut statement = db.prepare(sql).unwrap();
    let rows = statement
        .query_map(
            params![WorkspaceId::DEFAULT, "selected", "request_input", "fork"],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .unwrap()
        .collect::<Result<Vec<_>, _>>()
        .unwrap();
    (rows, statement.get_status(StatementStatus::VmStep))
}

fn terminal_stream_query(sql: &str, complete: bool) {
    let root = tempfile::tempdir().unwrap();
    let store = LocalObservationStore::open(root.path(), DigestAuthority::new([2; 32])).unwrap();
    let mut db = store.connection.lock();
    let tx = db.transaction().unwrap();
    // Metadata-only query fixture: a large older request and a tiny current
    // stream reproduce skewed retained history. Keep the earlier planner stats
    // when a new stream arrives, as in a live store. Native capture is separate.
    let insert = "INSERT INTO content_instances_v2(workspace_id,content_id,message_instance_id,
        request_id,direction,fork_id,part_ordinal,content_kind,canonical_media_type,
        content_blob_digest,has_content_ref,accumulated_byte_count,state,created_at_unix_nanos)
        VALUES(?1,?2,?2,?3,?4,?5,0,'text','text/plain',?6,0,0,?7,100000000)";
    for n in 0..10_000 {
        tx.execute(
            insert,
            params![
                WorkspaceId::DEFAULT,
                format!("retained-{n:05}"),
                "older",
                "request_input",
                "fork",
                format!("digest-{n}"),
                "complete"
            ],
        )
        .unwrap();
    }
    tx.execute_batch("ANALYZE content_instances_v2;").unwrap();
    for (workspace, id, request, direction, fork, state) in [
        (
            WorkspaceId::DEFAULT,
            "z",
            "selected",
            "request_input",
            "fork",
            "complete",
        ),
        (
            WorkspaceId::DEFAULT,
            "a",
            "selected",
            "request_input",
            "fork",
            "complete",
        ),
        (
            WorkspaceId::DEFAULT,
            "pending",
            "selected",
            "request_input",
            "fork",
            "installing",
        ),
        (
            WorkspaceId::DEFAULT,
            "other-direction",
            "selected",
            "response_delivered",
            "fork",
            "complete",
        ),
        (
            WorkspaceId::DEFAULT,
            "other-fork",
            "selected",
            "request_input",
            "other",
            "installing",
        ),
        (
            "workspace-other",
            "other-workspace",
            "selected",
            "request_input",
            "fork",
            "complete",
        ),
    ] {
        tx.execute(
            insert,
            params![
                workspace,
                id,
                request,
                direction,
                fork,
                format!("digest-{id}"),
                state
            ],
        )
        .unwrap();
    }
    tx.commit().unwrap();
    let expected = if complete {
        vec![
            ("a".into(), "digest-a".into()),
            ("z".into(), "digest-z".into()),
        ]
    } else {
        vec![("pending".into(), "digest-pending".into())]
    };
    let (actual, work) = query(&db, sql);
    assert_eq!(actual, expected);
    assert!(
        work < 1_000,
        "finishing a small stream scanned retained history: {work} steps"
    );

    // The observed production failure chose the workspace/content-ID ordering
    // index. Keep that negative control even if planner heuristics later change.
    let broad = sql
        .replace(" INDEXED BY content_instances_v2_request", "")
        .replace(
            "FROM content_instances_v2",
            "FROM content_instances_v2 NOT INDEXED",
        );
    let (same, broad_work) = query(&db, &broad);
    assert_eq!(same, actual);
    assert!(
        broad_work > work * 20,
        "unrelated content must not be visited: {work}/{broad_work}"
    );
}

#[test]
fn terminal_missing_blobs_only_visit_the_selected_stream() {
    terminal_stream_query(MISSING_BLOBS_SQL, false);
}
