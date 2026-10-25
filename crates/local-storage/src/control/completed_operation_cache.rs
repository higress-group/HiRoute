//! A bounded decoder memo for completed provenance, not an execution/currentness proof.
//! Every lookup compares the exact journal and all retained payload bytes in one SQL
//! snapshot. Mutable/recoverable operations keep the cold reader. Indexed receipts,
//! live grants, file ownership and writer/current-generation checks stay with callers.
use super::*;
use std::collections::VecDeque;

const MAX_ENTRIES: usize = 32;
const MAX_BYTES: usize = 128 * 1024 * 1024;

#[derive(Default)]
pub(super) struct CompletedOperationCache {
    entries: VecDeque<Entry>,
    bytes: usize,
    #[cfg(test)]
    pub(super) decodes: usize,
}

struct Entry {
    id: OperationId,
    journal: String,
    payloads: Vec<(String, String)>,
    operation: OperationV1,
    bytes: usize,
}

impl CompletedOperationCache {
    pub(super) fn read(
        &mut self,
        diagnostics: &hiroute_diagnostics::DiagnosticsPort,
        db: &Connection,
        id: &OperationId,
    ) -> PortResult<Option<OperationV1>> {
        let snapshot = if db.is_autocommit() {
            Some(db.unchecked_transaction().map_err(|_| unavailable())?)
        } else {
            None
        };
        let row: Option<(String, String)> = db
            .query_row(
                "SELECT state,operation_json FROM operations WHERE operation_id=?1",
                [id.as_str()],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .optional()
            .map_err(|_| unavailable())?;
        let result = match row {
            None => {
                self.remove(id);
                None
            }
            Some((state, journal)) if state == "succeeded" => {
                let mut query = db.prepare("SELECT digest,payload_json FROM operation_payloads WHERE operation_id=?1 ORDER BY digest")
                    .map_err(|_| unavailable())?;
                let payloads = query
                    .query_map([id.as_str()], |r| Ok((r.get(0)?, r.get(1)?)))
                    .map_err(|_| unavailable())?
                    .collect::<rusqlite::Result<Vec<(String, String)>>>()
                    .map_err(|_| unavailable())?;
                drop(query);
                if let Some(position) = self.entries.iter().position(|entry| {
                    entry.id == *id && entry.journal == journal && entry.payloads == payloads
                }) {
                    let entry = self.entries.remove(position).expect("located entry");
                    let operation = entry.operation.clone();
                    self.entries.push_back(entry);
                    Some(operation)
                } else {
                    self.remove(id);
                    #[cfg(test)]
                    {
                        self.decodes += 1;
                    }
                    let operation = decode_stored_operation(diagnostics, db, &journal)?;
                    if operation.operation_id != *id || operation.state != OperationState::Succeeded
                    {
                        return Err(port(PortErrorCode::Corrupt, "control.operation.identity"));
                    }
                    let bytes = payloads
                        .iter()
                        .fold(journal.len(), |n, (digest, payload)| {
                            n.saturating_add(digest.len()).saturating_add(payload.len())
                        })
                        .saturating_mul(8)
                        .saturating_add(64 * 1024);
                    if bytes <= MAX_BYTES {
                        while self.entries.len() >= MAX_ENTRIES || self.bytes > MAX_BYTES - bytes {
                            let old = self.entries.pop_front().expect("bounded occupied cache");
                            self.bytes -= old.bytes;
                        }
                        self.entries.push_back(Entry {
                            id: id.clone(),
                            journal,
                            payloads,
                            operation: operation.clone(),
                            bytes,
                        });
                        self.bytes += bytes;
                    }
                    Some(operation)
                }
            }
            Some((_, journal)) => {
                self.remove(id);
                Some(decode_stored_operation(diagnostics, db, &journal)?)
            }
        };
        if let Some(snapshot) = snapshot {
            snapshot.commit().map_err(|_| unavailable())?;
        }
        Ok(result)
    }

    fn remove(&mut self, id: &OperationId) {
        if let Some(position) = self.entries.iter().position(|entry| entry.id == *id) {
            self.bytes -= self.entries.remove(position).expect("located entry").bytes;
        }
    }
}

fn unavailable() -> PortError {
    port(PortErrorCode::Unavailable, "control.operation.read")
}
