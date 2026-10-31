//! Reuse SQLite programs in the writer connection's bounded cache. Callers
//! retain their original parameter evaluation, error mapping and transaction.
use rusqlite::{Connection, Params, Result, Row};

pub(crate) fn execute(connection: &Connection, sql: &str, params: impl Params) -> Result<usize> {
    connection.prepare_cached(sql)?.execute(params)
}

pub(crate) fn query_row<T>(
    connection: &Connection,
    sql: &str,
    params: impl Params,
    map: impl FnOnce(&Row<'_>) -> Result<T>,
) -> Result<T> {
    connection.prepare_cached(sql)?.query_row(params, map)
}

#[cfg(test)]
mod tests {
    use super::*;
    use rusqlite::{StatementStatus, params};

    #[test]
    fn cached_programs_release_large_bindings_between_requests() {
        let connection = Connection::open_in_memory().unwrap();
        connection.set_prepared_statement_cache_capacity(64);
        connection
            .execute_batch("CREATE TABLE payloads(scope INTEGER PRIMARY KEY, body BLOB)")
            .unwrap();
        let insert = "INSERT INTO payloads(scope, body) VALUES(?1, ?2)";
        let baseline = connection
            .prepare_cached(insert)
            .unwrap()
            .get_status(StatementStatus::MemUsed);
        for (scope, size) in [(1, 4 * 1024 * 1024), (2, 16 * 1024 * 1024)] {
            let body = vec![b'x'; size];
            assert_eq!(
                execute(&connection, insert, params![scope, &body]).unwrap(),
                1
            );
            drop(body);
            let stored: usize = query_row(
                &connection,
                "SELECT length(body) FROM payloads WHERE scope=?1",
                [scope],
                |row| row.get(0),
            )
            .unwrap();
            assert_eq!(stored, size);
            execute(&connection, "DELETE FROM payloads WHERE scope=?1", [scope]).unwrap();
            let retained = connection
                .prepare_cached(insert)
                .unwrap()
                .get_status(StatementStatus::MemUsed);
            assert!(
                retained <= baseline + 64 * 1024,
                "a cached program retained request payload: {baseline} -> {retained}"
            );
        }
    }
}
