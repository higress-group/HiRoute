//! Native SQLite preflight must preserve the database and consume existing WAL.
use super::*;

#[test]
fn checkpointed_wal_preflight_keeps_the_database_read_only() {
    let root = tempfile::tempdir().unwrap();
    let source = root.path().join("source.db");
    let connection = Connection::open(&source).unwrap();
    connection
        .execute_batch("PRAGMA journal_mode=WAL; PRAGMA user_version=23; CREATE TABLE facts(value); INSERT INTO facts VALUES('retained');")
        .unwrap();
    drop(connection);
    let path = root.path().join("checkpointed ? % 数据.db");
    fs::copy(source, &path).unwrap();
    let before = fs::read(&path).unwrap();
    let connection = Connection::open_with_flags(
        &path,
        OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX,
    )
    .unwrap();
    assert_eq!(
        connection
            .query_row("PRAGMA user_version", [], |r| r.get::<_, u32>(0))
            .unwrap(),
        23
    );
    assert_eq!(
        connection
            .query_row("SELECT value FROM facts", [], |r| r.get::<_, String>(0))
            .unwrap(),
        "retained"
    );
    assert!(
        connection
            .execute("INSERT INTO facts VALUES('forbidden')", [])
            .is_err()
    );
    drop(connection);
    assert_eq!(fs::read(&path).unwrap(), before);
}

#[test]
fn remaining_wal_is_consumed_instead_of_reading_only_the_main_file() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("source.db");
    let writer = Connection::open(&path).unwrap();
    writer.execute_batch("PRAGMA journal_mode=WAL; CREATE TABLE facts(value); PRAGMA wal_checkpoint(TRUNCATE); INSERT INTO facts VALUES('in WAL');").unwrap();
    assert!(root.path().join("source.db-wal").metadata().unwrap().len() > 0);
    let reader = Connection::open_with_flags(
        &path,
        OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX,
    )
    .unwrap();
    assert_eq!(
        reader
            .query_row("SELECT value FROM facts", [], |r| r.get::<_, String>(0))
            .unwrap(),
        "in WAL"
    );
}
