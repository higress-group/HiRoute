use crate::writer::ObservationStoreError;
use rusqlite::Transaction;

pub(crate) fn migrate(transaction: &Transaction<'_>) -> Result<(), ObservationStoreError> {
    migrate_inner(transaction).map_err(|_| ObservationStoreError::ActivityUnavailable)
}

fn migrate_inner(transaction: &Transaction<'_>) -> rusqlite::Result<()> {
    let mut rebuilt = Vec::new();
    for table in [
        "observation_usage_archives_v2",
        "valuation_requests_v2",
        "valuation_contributions_v2",
        "valuation_archives_v2",
    ] {
        let requires_plan: bool = transaction.query_row(
            "SELECT EXISTS(SELECT 1 FROM pragma_table_info(?1) WHERE name='plan_id' AND \"notnull\"=1)",
            [table],
            |row| row.get(0),
        )?;
        if requires_plan {
            transaction.execute_batch(&format!(
                "ALTER TABLE {table} RENAME TO {table}_required_plan;"
            ))?;
            rebuilt.push(table);
        }
    }
    transaction.execute_batch(SCHEMA)?;
    for table in rebuilt {
        transaction.execute_batch(&format!(
            "INSERT INTO {table} SELECT * FROM {table}_required_plan;
             DROP TABLE {table}_required_plan;"
        ))?;
    }
    transaction.execute_batch(SCHEMA)
}

const SCHEMA: &str = "
CREATE TABLE IF NOT EXISTS observation_usage_archives_v2(
    workspace_id TEXT NOT NULL,day INTEGER NOT NULL,plan_id TEXT,metric TEXT NOT NULL,
    -- known_sum is the signed INTEGER encoding of the full u64 bit pattern.
    -- Negative missing_count means sticky overflow: -(actual_missing_count + 1).
    -- Readers restore negative high-bit values with an unsigned cast.
    known_sum INTEGER,missing_count INTEGER NOT NULL,
    PRIMARY KEY(workspace_id,day,plan_id,metric));
CREATE UNIQUE INDEX IF NOT EXISTS observation_usage_fixed_bucket_v2
    ON observation_usage_archives_v2(workspace_id,day,metric) WHERE plan_id IS NULL;
CREATE TABLE IF NOT EXISTS valuation_requests_v2(
    workspace_id TEXT NOT NULL,request_id TEXT NOT NULL,session_id TEXT NOT NULL,plan_id TEXT,
    started_ms INTEGER NOT NULL,input_revision INTEGER NOT NULL,input_digest TEXT NOT NULL,
    terminal INTEGER NOT NULL DEFAULT 0,accepted_ordinal INTEGER,partial INTEGER NOT NULL DEFAULT 0,
    PRIMARY KEY(workspace_id,request_id));
CREATE INDEX IF NOT EXISTS valuation_request_accepted_ordinal
    ON valuation_requests_v2(workspace_id,request_id,accepted_ordinal);
CREATE TABLE IF NOT EXISTS valuation_attempt_inputs_v2(
    workspace_id TEXT NOT NULL,request_id TEXT NOT NULL,ordinal INTEGER NOT NULL,
    pricing_json TEXT,usage_json TEXT,
    PRIMARY KEY(workspace_id,request_id,ordinal));
CREATE TABLE IF NOT EXISTS valuation_pending_v2(
    workspace_id TEXT NOT NULL,request_id TEXT NOT NULL,PRIMARY KEY(workspace_id,request_id));
CREATE TABLE IF NOT EXISTS valuation_records_v2(
    workspace_id TEXT NOT NULL,request_id TEXT NOT NULL,revision INTEGER NOT NULL,
    input_digest TEXT NOT NULL,body_json TEXT NOT NULL,
    PRIMARY KEY(workspace_id,request_id,revision));
CREATE TABLE IF NOT EXISTS valuation_contributions_v2(
    workspace_id TEXT NOT NULL,request_id TEXT NOT NULL,day INTEGER NOT NULL,plan_id TEXT,
    currency TEXT NOT NULL,valuation_kind TEXT NOT NULL,known_micros INTEGER,coverage TEXT NOT NULL,
    PRIMARY KEY(workspace_id,request_id,currency,valuation_kind));
CREATE INDEX IF NOT EXISTS valuation_contribution_bucket_v2
    ON valuation_contributions_v2(workspace_id,day,plan_id,currency,valuation_kind);
CREATE TABLE IF NOT EXISTS valuation_archives_v2(
    workspace_id TEXT NOT NULL,day INTEGER NOT NULL,plan_id TEXT,currency TEXT NOT NULL,valuation_kind TEXT NOT NULL,
    known_micros INTEGER,partial_count INTEGER NOT NULL,
    PRIMARY KEY(workspace_id,day,plan_id,currency,valuation_kind));
CREATE UNIQUE INDEX IF NOT EXISTS valuation_fixed_bucket_v2
    ON valuation_archives_v2(workspace_id,day,currency,valuation_kind) WHERE plan_id IS NULL;
";

#[cfg(test)]
mod tests {
    use super::*;
    use rusqlite::Connection;

    #[test]
    fn plan_rows_survive_nullable_migration_and_fixed_archives_remain_unique() {
        let mut db = Connection::open_in_memory().unwrap();
        db.execute_batch(&SCHEMA.replace("plan_id TEXT,", "plan_id TEXT NOT NULL,"))
            .unwrap();
        db.execute_batch(
            "INSERT INTO valuation_requests_v2 VALUES('w','r','s','plan/p',1,2,'digest',1,0,0);
             INSERT INTO valuation_contributions_v2 VALUES('w','r',0,'plan/p','USD','usage',7,'complete');
             INSERT INTO valuation_archives_v2 VALUES('w',0,'plan/p','USD','usage',11,0);
             INSERT INTO observation_usage_archives_v2 VALUES('w',0,'plan/p','input',13,0);",
        ).unwrap();
        let tx = db.transaction().unwrap();
        migrate(&tx).unwrap();
        migrate(&tx).unwrap();
        let plan: (String, u64) = tx
            .query_row(
                "SELECT plan_id,input_revision FROM valuation_requests_v2",
                [],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .unwrap();
        assert_eq!(plan, ("plan/p".into(), 2));
        for (table, amount, expected) in [
            ("valuation_contributions_v2", "known_micros", 7),
            ("valuation_archives_v2", "known_micros", 11),
            ("observation_usage_archives_v2", "known_sum", 13),
        ] {
            let actual: i64 = tx
                .query_row(
                    &format!("SELECT {amount} FROM {table} WHERE plan_id='plan/p'"),
                    [],
                    |row| row.get(0),
                )
                .unwrap();
            assert_eq!(actual, expected);
        }
        tx.execute_batch(
            "INSERT INTO valuation_requests_v2 VALUES('w','fixed','s',NULL,1,1,'fixed-digest',1,0,0);
             INSERT INTO valuation_contributions_v2 VALUES('w','fixed',0,NULL,'USD','usage',17,'complete');
             INSERT INTO valuation_archives_v2 VALUES('w',0,NULL,'USD','usage',17,0);
             INSERT INTO valuation_archives_v2 VALUES('w',0,NULL,'USD','usage',19,0)
                ON CONFLICT DO UPDATE SET known_micros=known_micros+excluded.known_micros;
             INSERT INTO observation_usage_archives_v2 VALUES('w',0,NULL,'input',23,0);
             INSERT INTO observation_usage_archives_v2 VALUES('w',0,NULL,'input',29,0)
                ON CONFLICT DO UPDATE SET known_sum=known_sum+excluded.known_sum;",
        ).unwrap();
        for (table, amount, expected) in [
            ("valuation_archives_v2", "known_micros", 36),
            ("observation_usage_archives_v2", "known_sum", 52),
        ] {
            let actual: (u64, i64) = tx
                .query_row(
                    &format!("SELECT COUNT(*),SUM({amount}) FROM {table} WHERE plan_id IS NULL"),
                    [],
                    |row| Ok((row.get(0)?, row.get(1)?)),
                )
                .unwrap();
            assert_eq!(actual, (1, expected));
        }
        tx.commit().unwrap();
    }

    #[test]
    fn nullable_migration_rolls_back_with_its_owner_transaction() {
        let mut db = Connection::open_in_memory().unwrap();
        db.execute_batch(&SCHEMA.replace("plan_id TEXT,", "plan_id TEXT NOT NULL,"))
            .unwrap();
        {
            let tx = db.transaction().unwrap();
            migrate(&tx).unwrap();
        }
        let required: bool = db.query_row(
            "SELECT \"notnull\" FROM pragma_table_info('valuation_requests_v2') WHERE name='plan_id'",
            [], |row| row.get(0),
        ).unwrap();
        assert!(required);
    }
}
