//! Small mutable journal copies refer to separately retained immutable input/effect payloads.
use super::*;

pub(super) fn save(transaction: &Transaction<'_>, operation: &OperationV1) -> PortResult<u64> {
    let update = operation
        .journal_update()
        .map_err(|_| port(PortErrorCode::InvalidData, "control.journal.inputs"))?;
    let partitioned = payloads::partitioned(transaction, operation.operation_id.as_str())?;
    let mut expression = String::from(
        "json_set(operation_json, '$.state', ?2, '$.generation', ?3, '$.safe_error_code', ?4",
    );
    let mut values: Vec<rusqlite::types::Value> = vec![
        operation.operation_id.as_str().to_owned().into(),
        operation.state.as_str().to_owned().into(),
        i64::try_from(update.next_generation)
            .map_err(|_| {
                port(
                    PortErrorCode::InvalidData,
                    "control.journal.generation_range",
                )
            })?
            .into(),
        operation
            .safe_error_code
            .clone()
            .map(Into::into)
            .unwrap_or(rusqlite::types::Value::Null),
        i64::try_from(update.expected_generation)
            .map_err(|_| {
                port(
                    PortErrorCode::InvalidData,
                    "control.journal.generation_range",
                )
            })?
            .into(),
        (if partitioned {
            payloads::plan(transaction, operation, false)?
        } else {
            update.plan_json.to_owned()
        })
        .into(),
        operation.request_digest.as_str().to_owned().into(),
        operation.accepted_digest.as_str().to_owned().into(),
        operation.workspace_id.as_str().to_owned().into(),
        operation.idempotency.principal.clone().into(),
        operation.idempotency.operation_kind.clone().into(),
        operation.idempotency.key.clone().into(),
    ];
    for step in &update.changed_steps {
        let projection = if partitioned {
            payloads::step(transaction, operation.operation_id.as_str(), step, true)?
        } else {
            serde_json::to_value(step)
                .map_err(|_| port(PortErrorCode::InvalidData, "control.step.encode"))?
        };
        let encoded = serde_json::to_string(&projection)
            .map_err(|_| port(PortErrorCode::InvalidData, "control.step.encode"))?;
        values.push(encoded.into());
        expression.push_str(&format!(
            ", '$.steps[{}]', json(?{})",
            step.sequence,
            values.len()
        ));
        store_step(transaction, operation, step)?;
    }
    expression.push(')');
    // Same writer generation and exact immutable plan are checked in SQLite, without rebuilding
    // a Rust JSON tree or re-running typed planners. Independent recovery still fully decodes.
    let sql = format!(
        "UPDATE operations SET state=?2, generation=?3, operation_json={expression}, updated_at=unixepoch()
         WHERE operation_id=?1 AND generation=?5 AND json_extract(operation_json, '$.plan')=?6
         AND request_digest=?7 AND accepted_change_digest=?8 AND workspace_id=?9
         AND principal=?10 AND operation_kind=?11 AND idempotency_key=?12"
    );
    let updated = transaction
        .execute(&sql, rusqlite::params_from_iter(values))
        .map_err(|_| port(PortErrorCode::Unavailable, "control.journal.update"))?;
    if updated != 1 {
        return Err(port(PortErrorCode::Conflict, "control.journal.generation"));
    }
    Ok(update.next_generation)
}

pub(super) fn is_current(connection: &Connection, operation: &OperationV1) -> PortResult<bool> {
    is_current_in_scope(connection, operation, None)
}

/// Current file roots require their original input and exact file receipt. Optional
/// grant-attribution proof in another step cannot disable an otherwise valid file
/// restore dependency. Executing/recovering an operation still uses the full check.
pub(super) fn is_current_artifact_step(
    connection: &Connection,
    operation: &OperationV1,
) -> PortResult<bool> {
    let Some(step) = operation
        .steps
        .iter()
        .find(|step| step.kind == hiroute_domain::OperationStepKind::ApplyAgentArtifacts)
    else {
        return Ok(false);
    };
    is_current_in_scope(connection, operation, Some(step.sequence))
}

/// Active native model checks also consume the original grant Ensure receipt. Other
/// completed steps (including optional attribution) are not authority for this read.
pub(super) fn is_current_grant_step(
    connection: &Connection,
    operation: &OperationV1,
) -> PortResult<bool> {
    let Some(step) = operation.steps.iter().find(|step| {
        step.kind == hiroute_domain::OperationStepKind::ApplySecrets
            && step.status == hiroute_domain::OperationStepStatus::Applied
    }) else {
        return Ok(false);
    };
    is_current_in_scope(connection, operation, Some(step.sequence))
}

fn is_current_in_scope(
    connection: &Connection,
    operation: &OperationV1,
    selected: Option<u16>,
) -> PortResult<bool> {
    if !operation
        .journal_is_committed()
        .map_err(|_| port(PortErrorCode::InvalidData, "control.current.checkpoint"))?
    {
        return Ok(false);
    }
    let checkpoint = operation
        .journal_update()
        .map_err(|_| port(PortErrorCode::InvalidData, "control.current.inputs"))?;
    // A supported retained inline journal keeps its original immutable bytes.
    let partitioned = payloads::partitioned(connection, operation.operation_id.as_str())?;
    let projection = if partitioned {
        (|| {
            Ok::<_, PortError>((
                payloads::plan(connection, operation, false)?,
                operation
                    .steps
                    .iter()
                    .filter(|s| selected.is_none_or(|sequence| s.sequence == sequence))
                    .map(|s| payloads::step(connection, operation.operation_id.as_str(), s, false))
                    .collect::<PortResult<Vec<_>>>()?,
            ))
        })()
    } else {
        Ok((
            checkpoint.plan_json.to_owned(),
            serde_json::to_value(
                operation
                    .steps
                    .iter()
                    .filter(|s| selected.is_none_or(|sequence| s.sequence == sequence))
                    .collect::<Vec<_>>(),
            )
            .map_err(|_| port(PortErrorCode::InvalidData, "control.current.steps"))?
            .as_array()
            .expect("steps array")
            .clone(),
        ))
    };
    let (plan_json, projected_steps) = match projection {
        Ok(value) => value,
        Err(error) if error.code == PortErrorCode::Corrupt => return Ok(false),
        Err(error) => return Err(error),
    };
    let steps = serde_json::to_string(&projected_steps)
        .map_err(|_| port(PortErrorCode::InvalidData, "control.current.steps"))?;
    let revisions = serde_json::to_string(&operation.expected_revisions)
        .map_err(|_| port(PortErrorCode::InvalidData, "control.current.revisions"))?;
    let step_rows = hiroute_domain::canonicalize_json(Value::Array(projected_steps)).to_string();
    // A sealed settings service releases its writer claim while the native-file tail waits for
    // explicit retry. The coordinator verifies the receipt before doing any tail work and must
    // still be able to observe this exact committed journal during startup recovery.
    let parked_settings_tail = operation.state == OperationState::Activating
        && operation
            .step(hiroute_domain::OperationStepKind::Activate)
            .terminal_result
            .as_deref()
            .and_then(hiroute_domain::SettingsServiceCompletionV1::parse)
            .is_some();
    // Fetch guards and both mutable copies in one SQLite snapshot. A second query could
    // accidentally approve an old generation after another connection commits a new one.
    let copies: Option<(String, String)> = connection
        .query_row(
            "SELECT CASE WHEN ?16 IS NULL THEN json_extract(operation_json, '$.steps')
                ELSE json_array(json_extract(operation_json, '$.steps['||?16||']')) END,
                (SELECT json_group_array(json(step)) FROM (
                    SELECT json_extract(step_json, '$.step') AS step FROM operation_steps
                    WHERE operation_id=?1 AND (?16 IS NULL OR step_no=?16) ORDER BY step_no))
         FROM operations WHERE operation_id=?1 AND generation=?2
         AND json_extract(operation_json, '$.plan')=?3 AND request_digest=?4
         AND accepted_change_digest=?5 AND workspace_id=?6 AND principal=?7
         AND operation_kind=?8 AND idempotency_key=?9 AND state=?10
         AND json_extract(operation_json, '$.operation_id')=?1
         AND json_extract(operation_json, '$.workspace_id')=?6
         AND json_extract(operation_json, '$.request_digest')=?4
         AND json_extract(operation_json, '$.accepted_digest')=?5
         AND json_extract(operation_json, '$.idempotency.principal')=?7
         AND json_extract(operation_json, '$.idempotency.operation_kind')=?8
         AND json_extract(operation_json, '$.idempotency.key')=?9
         AND json_type(operation_json, '$.steps')='array'
         AND json_extract(operation_json, '$.safe_error_code') IS ?11
         AND json_extract(operation_json, '$.expected_revisions')=json(?12)
         AND json_extract(operation_json, '$.schema_version')=?13
         AND json_extract(operation_json, '$.generation')=?2
         AND json_extract(operation_json, '$.state')=?10
         AND (state IN ('succeeded', 'rolled_back', 'needs_attention')
              OR EXISTS(SELECT 1 FROM writer_claim WHERE operation_id=?1)
              OR (?15 AND NOT EXISTS(SELECT 1 FROM writer_claim)))
         AND (SELECT count(*) FROM operation_steps WHERE operation_id=?1
              AND (?16 IS NULL OR step_no=?16))=CASE WHEN ?16 IS NULL THEN ?14 ELSE 1 END
         AND NOT EXISTS(SELECT 1 FROM operation_steps s WHERE s.operation_id=?1 AND (?16 IS NULL OR s.step_no=?16)
             AND (s.step_no IS NOT json_extract(s.step_json, '$.step.sequence')
                  OR s.step_kind IS NOT json_extract(s.step_json, '$.step.kind')
                  OR s.state IS NOT json_extract(s.step_json, '$.step.status')))",
            params![
                operation.operation_id.as_str(),
                operation.generation,
                plan_json,
                operation.request_digest.as_str(),
                operation.accepted_digest.as_str(),
                operation.workspace_id.as_str(),
                operation.idempotency.principal,
                operation.idempotency.operation_kind,
                operation.idempotency.key,
                operation.state.as_str(),
                operation.safe_error_code,
                revisions,
                operation.schema_version,
                operation.steps.len() as u64,
                parked_settings_tail,
                selected,
            ],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .optional()
        .map_err(|_| port(PortErrorCode::Unavailable, "control.current.read"))?;
    let Some((journal_steps, row_steps)) = copies else {
        return Ok(false);
    };
    if journal_steps == steps && row_steps == step_rows {
        return Ok(true);
    }
    let expected = read_journal_json(&steps)
        .map_err(|_| port(PortErrorCode::InvalidData, "control.current.expected-steps"))?;
    Ok(journal_json_matches(&journal_steps, &expected)
        && journal_json_matches(&row_steps, &expected))
}

// Only mutable journal copies use this reader. Immutable plans and admission identities
// retain exact SQL predicates. Typed OperationStep decoding would discard unknown fields;
// ordinary Value decoding would discard duplicate members and round oversized numbers.
fn journal_json_matches(encoded: &str, expected: &JournalJson) -> bool {
    read_journal_json(encoded).is_ok_and(|actual| actual == *expected)
}

#[derive(PartialEq)]
enum JournalJson {
    Scalar(Value),
    Number(String),
    Array(Vec<Self>),
    Object(BTreeMap<String, Self>),
}

fn read_journal_json(encoded: &str) -> serde_json::Result<JournalJson> {
    JournalJson::read(serde_json::from_str(encoded)?, 0)
}

impl JournalJson {
    fn read(raw: &serde_json::value::RawValue, depth: usize) -> serde_json::Result<Self> {
        // RawValue starts a fresh deserializer for each container. Retain serde's usual
        // whole-document depth bound instead of letting nested corrupt input exhaust the stack.
        if depth >= 128 {
            return Err(serde::de::Error::custom("journal nesting limit exceeded"));
        }
        match raw.get().as_bytes()[0] {
            b'{' => {
                let JournalObject(values) = serde_json::from_str(raw.get())?;
                values
                    .into_iter()
                    .map(|(key, value)| Ok((key, Self::read(value, depth + 1)?)))
                    .collect::<serde_json::Result<_>>()
                    .map(Self::Object)
            }
            b'[' => serde_json::from_str::<Vec<&serde_json::value::RawValue>>(raw.get())?
                .into_iter()
                .map(|value| Self::read(value, depth + 1))
                .collect::<serde_json::Result<_>>()
                .map(Self::Array),
            b'"' | b'n' | b't' | b'f' => serde_json::from_str(raw.get()).map(Self::Scalar),
            // Current producers serialize numeric tokens deterministically. Retain their
            // spelling to reject changed integers beyond u64 and floats that round alike.
            _ => Ok(Self::Number(raw.get().to_owned())),
        }
    }
}

struct JournalObject<'a>(BTreeMap<String, &'a serde_json::value::RawValue>);

impl<'de> Deserialize<'de> for JournalObject<'de> {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        struct JournalVisitor;
        impl<'de> serde::de::Visitor<'de> for JournalVisitor {
            type Value = JournalObject<'de>;

            fn expecting(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                formatter.write_str("journal object without duplicate members")
            }

            fn visit_map<A: serde::de::MapAccess<'de>>(
                self,
                mut map: A,
            ) -> Result<Self::Value, A::Error> {
                let mut values = BTreeMap::new();
                while let Some(key) = map.next_key::<String>()? {
                    if values.contains_key(&key) {
                        return Err(serde::de::Error::custom("duplicate journal member"));
                    }
                    values.insert(key, map.next_value()?);
                }
                Ok(JournalObject(values))
            }
        }
        deserializer.deserialize_map(JournalVisitor)
    }
}

/// Strict structure check shared by the versioned storage codec; no typed projection
/// may silently discard duplicate fields or turn oversized numbers into rounded values.
pub(super) fn validate_encoding(encoded: &str) -> PortResult<()> {
    read_journal_json(encoded)
        .map(|_| ())
        .map_err(|_| port(PortErrorCode::Corrupt, "control.journal.encoding"))
}
