//! Journal copies may differ in object order; content and admission identity may not drift.
use super::*;
use hiroute_domain::OwnedEffectV1;
use rusqlite::params;

fn journal_copies(control: &ControlStore) -> (String, String) {
    let connection = control.connection.borrow();
    (
        connection
            .query_row("SELECT operation_json FROM operations", [], |r| r.get(0))
            .unwrap(),
        connection
            .query_row(
                "SELECT step_json FROM operation_steps WHERE step_no=0",
                [],
                |r| r.get(0),
            )
            .unwrap(),
    )
}

fn restore_copies(control: &ControlStore, copies: &(String, String)) {
    let connection = control.connection.borrow();
    connection
        .execute("UPDATE operations SET operation_json=?1", [&copies.0])
        .unwrap();
    connection
        .execute(
            "UPDATE operation_steps SET step_json=?1 WHERE step_no=0",
            [&copies.1],
        )
        .unwrap();
}

#[test]
fn journal_member_order_survives_recovery_but_never_hides_changed_or_duplicate_content() {
    let root = tempdir().unwrap();
    let control = ControlStore::open(
        &crate::test_storage_authority(),
        root.path().join("control.db"),
        root.path().join("backups"),
    )
    .unwrap();
    let mut op = operation(&WorkspaceId::default(), "journal-object-order", 0);
    let authorization = authorize(&control, &op, "journal-order-capability", i64::MAX);
    control.begin_operation(&op, &authorization).unwrap();
    op.steps[0].effects.push(OwnedEffectV1 {
        effect_id: "journal-order".into(),
        kind: OwnedEffectKind::Control,
        target: "journal-order".into(),
        before_fingerprint: None,
        after_fingerprint: None,
        compensation: json!({"a":1,"large":9_223_372_036_854_775_808_u64,"z":[1,2]}).into(),
    });
    control.save_operation(&mut op).unwrap();
    assert!(control.operation_is_current(&op).unwrap());
    let immutable_plan: String = control
        .connection
        .borrow()
        .query_row(
            "SELECT json_extract(operation_json, '$.plan') FROM operations",
            [],
            |r| r.get(0),
        )
        .unwrap();
    // SQLite constructs an explicit alternative producer order in both journal copies,
    // independently of whichever serde_json feature set this test binary resolved.
    control.connection.borrow().execute_batch(
        "UPDATE operation_steps SET step_json=json_set(step_json, '$.step', json_object(
            'effects', json_extract(step_json, '$.step.effects'),
            'attempts', json_extract(step_json, '$.step.attempts'),
            'status', json_extract(step_json, '$.step.status'),
            'kind', json_extract(step_json, '$.step.kind'),
            'deterministic_input_digest', json_extract(step_json, '$.step.deterministic_input_digest'),
            'sequence', json_extract(step_json, '$.step.sequence'))) WHERE step_no=0;
         UPDATE operation_steps SET step_json=json_set(step_json,
            '$.step.effects[0].compensation', json('{\"z\":[1,2],\"large\":9223372036854775808,\"a\":1}')) WHERE step_no=0;
         UPDATE operations SET operation_json=json_set(operation_json, '$.steps[0]',
            json((SELECT json_extract(step_json, '$.step') FROM operation_steps WHERE step_no=0)));"
    ).unwrap();
    let reordered = journal_copies(&control);
    let mut recovered = control.load_operation(&op.operation_id).unwrap().unwrap();
    assert_eq!(recovered, op);
    assert!(control.operation_is_current(&recovered).unwrap());
    assert_eq!(
        journal_copies(&control),
        reordered,
        "read-only validation must retain old bytes"
    );

    let compensation_locations = [
        (
            "operations",
            "operation_json",
            "",
            "$.steps[0].effects[0].compensation",
        ),
        (
            "operation_steps",
            "step_json",
            " WHERE step_no=0",
            "$.step.effects[0].compensation",
        ),
    ];
    for (table, column, scope, path) in compensation_locations {
        for corrupted in [
            // Identical duplicate values must still fail.
            r#"{"a":1,"a":1,"large":9223372036854775808,"z":[1,2]}"#,
            r#"{"a":1,"\u0061":1,"large":9223372036854775808,"z":[1,2]}"#,
            r#"{"a":1,"large":9223372036854775808,"z":[1,2],"unexpected":null}"#,
            r#"{"a":1,"large":9223372036854775808,"z":[2,1]}"#,
            r#"{"a":1.0,"large":9223372036854775808,"z":[1,2]}"#,
            r#"{"a":1,"large":9223372036854775808}"#,
            r#"{"a":1,"large":9223372036854775808,"z":null}"#,
            r#"{"a":1,"large":9223372036854775809,"z":[1,2]}"#,
        ] {
            restore_copies(&control, &reordered);
            control
                .connection
                .borrow()
                .execute(
                    &format!(
                        "UPDATE {table} SET {column}=json_set({column}, '{path}', json(?1)){scope}"
                    ),
                    params![corrupted],
                )
                .unwrap();
            assert!(
                !control.operation_is_current(&recovered).unwrap(),
                "{table}: {corrupted}"
            );
        }
    }
    for statement in [
        "UPDATE operations SET operation_json=json_set(operation_json, '$.steps[0].unexpected', 1)",
        "UPDATE operation_steps SET step_json=json_set(step_json, '$.step.unexpected', 1) WHERE step_no=0",
    ] {
        restore_copies(&control, &reordered);
        control.connection.borrow().execute(statement, []).unwrap();
        assert!(!control.operation_is_current(&recovered).unwrap());
    }
    restore_copies(&control, &reordered);
    // SQLite and ordinary Value decoding can round these different numeric tokens alike.
    // A quoted key also proves that comparison does not depend on SQLite's JSON-path parser.
    for (number, changed_token) in [
        (json!(u64::MAX), "18446744073709551614"),
        (
            json!(18_446_744_073_709_551_616.0_f64),
            "18446744073709551616",
        ),
        (json!(1.0_f64), "1.00000000000000001"),
    ] {
        recovered.steps[0].effects[0].compensation = json!({"quoted\"number": number}).into();
        control.save_operation(&mut recovered).unwrap();
        assert!(control.operation_is_current(&recovered).unwrap());
        let current = journal_copies(&control);
        for (table, column, scope, path) in compensation_locations {
            let corrupted = format!(r#"{{"quoted\"number":{changed_token}}}"#);
            control
                .connection
                .borrow()
                .execute(
                    &format!(
                        "UPDATE {table} SET {column}=json_set({column}, '{path}', json(?1)){scope}"
                    ),
                    params![corrupted],
                )
                .unwrap();
            assert!(!control.operation_is_current(&recovered).unwrap());
            restore_copies(&control, &current);
        }
    }
    recovered.safe_error_code = Some("ORDER_RECOVERED".into());
    control.save_operation(&mut recovered).unwrap();
    assert!(control.operation_is_current(&recovered).unwrap());
    let current_plan: String = control
        .connection
        .borrow()
        .query_row(
            "SELECT json_extract(operation_json, '$.plan') FROM operations",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(current_plan, immutable_plan);
}
