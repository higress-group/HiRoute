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
