//! Storage-only partition of immutable input/effect bytes from journal progress.
//! References never grant authority: cold reads authenticate and reconstruct the original
//! domain value, and warm writes compare the exact retained bytes in their transaction.
use super::*;
use serde_json::value::RawValue;

const SCHEMA: &str = "hiroute.operation-payload/v1";
const EFFECT_THRESHOLD: usize = 32 * 1024;

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Reference {
    schema: String,
    digest: CanonicalDigest,
}

pub(super) fn plan(db: &Connection, operation: &OperationV1, write: bool) -> PortResult<String> {
    payload(
        db,
        operation.operation_id.as_str(),
        operation.journal_update().map_err(|_| invalid())?.plan_json,
        write,
    )
}

fn payload(db: &Connection, owner: &str, encoded: &str, write: bool) -> PortResult<String> {
    let digest = CanonicalDigest::of_bytes(encoded.as_bytes());
    if write {
        db.execute(
            "INSERT INTO operation_payloads(operation_id,digest,payload_json)
            VALUES(?1,?2,?3) ON CONFLICT DO NOTHING",
            params![owner, digest.as_str(), encoded],
        )
        .map_err(|_| unavailable())?;
    }
    // Checking only a stored digest would let damaged immutable bytes pass a warm path.
    let exact: bool = db
        .query_row(
            "SELECT EXISTS(SELECT 1 FROM operation_payloads
        WHERE operation_id=?1 AND digest=?2 AND payload_json=?3)",
            params![owner, digest.as_str(), encoded],
            |r| r.get(0),
        )
        .map_err(|_| unavailable())?;
    if !exact {
        return Err(invalid());
    }
    Ok(hiroute_domain::canonicalize_json(
        serde_json::to_value(Reference {
            schema: SCHEMA.into(),
            digest,
        })
        .map_err(|_| invalid())?,
    )
    .to_string())
}

pub(super) fn step(
    db: &Connection,
    owner: &str,
    step: &OperationStepV1,
    write: bool,
) -> PortResult<Value> {
    let mut compact = step.clone();
    for effect in &mut compact.effects {
        let bytes = serde_json::to_string(&effect.compensation).map_err(|_| invalid())?;
        if bytes.len() >= EFFECT_THRESHOLD {
            effect.compensation =
                serde_json::from_str::<Value>(&payload(db, owner, &bytes, write)?)
                    .map_err(|_| invalid())?
                    .into();
        }
    }
    serde_json::to_value(compact).map_err(|_| invalid())
}

pub(super) fn encode(db: &Connection, operation: &OperationV1) -> PortResult<String> {
    let mut value = serde_json::to_value(operation).map_err(|_| invalid())?;
    value["plan"] = serde_json::from_str(&plan(db, operation, true)?).map_err(|_| invalid())?;
    value["steps"] = Value::Array(
        operation
            .steps
            .iter()
            .map(|s| step(db, operation.operation_id.as_str(), s, true))
            .collect::<PortResult<_>>()?,
    );
    serde_json::to_string(&value).map_err(|_| invalid())
}

type Object = BTreeMap<String, Box<RawValue>>;
fn raw<T: Serialize>(value: &T) -> PortResult<Box<RawValue>> {
    serde_json::value::to_raw_value(value).map_err(|_| invalid())
}

fn expand_payload(db: &Connection, owner: &str, value: &RawValue) -> PortResult<Box<RawValue>> {
    let candidate: Value = serde_json::from_str(value.get()).map_err(|_| invalid())?;
    if candidate.get("schema").and_then(Value::as_str) != Some(SCHEMA) {
        return RawValue::from_string(value.get().to_owned()).map_err(|_| invalid());
    }
    let reference: Reference = serde_json::from_str(value.get()).map_err(|_| invalid())?;
    let bytes: String = db
        .query_row(
            "SELECT payload_json FROM operation_payloads
        WHERE operation_id=?1 AND digest=?2",
            params![owner, reference.digest.as_str()],
            |r| r.get(0),
        )
        .map_err(|_| invalid())?;
    if CanonicalDigest::of_bytes(bytes.as_bytes()) != reference.digest {
        return Err(invalid());
    }
    journal::validate_encoding(&bytes)?;
    RawValue::from_string(bytes).map_err(|_| invalid())
}

fn expand_step(db: &Connection, owner: &str, encoded: &RawValue) -> PortResult<Box<RawValue>> {
    let mut value: Object = serde_json::from_str(encoded.get()).map_err(|_| invalid())?;
    if let Some(effects) = value.get("effects") {
        let mut effects: Vec<Object> =
            serde_json::from_str(effects.get()).map_err(|_| invalid())?;
        for effect in &mut effects {
            if let Some(compensation) = effect.get("compensation") {
                let expanded = expand_payload(db, owner, compensation)?;
                effect.insert("compensation".into(), expanded);
            }
        }
        value.insert("effects".into(), raw(&effects)?);
    }
    raw(&value)
}

pub(super) fn expand(db: &Connection, encoded: &str) -> PortResult<String> {
    // Preserve duplicate-member/large-number rejection before changing object layout.
    journal::validate_encoding(encoded)?;
    let mut value: Object = serde_json::from_str(encoded).map_err(|_| invalid())?;
    let owner: String = serde_json::from_str(value.get("operation_id").ok_or_else(invalid)?.get())
        .map_err(|_| invalid())?;
    let plan = expand_payload(db, &owner, value.get("plan").ok_or_else(invalid)?)?;
    value.insert("plan".into(), plan);
    let steps: Vec<Box<RawValue>> =
        serde_json::from_str(value.get("steps").ok_or_else(invalid)?.get())
            .map_err(|_| invalid())?;
    let steps = steps
        .iter()
        .map(|s| expand_step(db, &owner, s))
        .collect::<PortResult<Vec<_>>>()?;
    value.insert("steps".into(), raw(&steps)?);
    serde_json::to_string(&value).map_err(|_| invalid())
}

pub(super) fn expand_step_record(db: &Connection, owner: &str, encoded: &str) -> PortResult<Value> {
    journal::validate_encoding(encoded)?;
    let mut record: Object = serde_json::from_str(encoded).map_err(|_| invalid())?;
    let step = expand_step(db, owner, record.get("step").ok_or_else(invalid)?)?;
    record.insert("step".into(), step);
    serde_json::to_value(&record).map_err(|_| invalid())
}

/// Older completed journals retain their original bytes. The producer format is
/// explicit in the input field; old inputs stay on their registered legacy writer
/// until their own recovery completes, rather than rewriting unrelated history.
pub(super) fn partitioned(db: &Connection, owner: &str) -> PortResult<bool> {
    db.query_row("SELECT json_extract(operation_json,'$.plan.schema')=?2 FROM operations WHERE operation_id=?1",
        params![owner,SCHEMA], |r| r.get::<_,Option<bool>>(0)).optional()
        .map(|value| value.flatten().unwrap_or(false)).map_err(|_| invalid())
}

fn invalid() -> PortError {
    port(PortErrorCode::Corrupt, "control.payload.integrity")
}
fn unavailable() -> PortError {
    port(PortErrorCode::Unavailable, "control.payload.storage")
}
