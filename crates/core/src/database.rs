use crate::{Error, Result};
use rusqlite::Connection;
pub const SCHEMA_VERSION: i32 = 6;
pub fn migrate(db: &mut Connection) -> Result<()> {
    db.pragma_update(None, "foreign_keys", true)?;
    db.busy_timeout(std::time::Duration::from_secs(5))?;
    db.pragma_update(None, "journal_mode", "WAL")?;
    db.pragma_update(None, "synchronous", "FULL")?;
    let version: i32 = db.pragma_query_value(None, "user_version", |r| r.get(0))?;
    if version > SCHEMA_VERSION {
        return Err(Error::Project(
            "database schema is newer than this program".into(),
        ));
    }
    if version < 1 {
        let tx = db.transaction()?;
        tx.execute_batch(include_str!("../../../migrations/001_initial.sql"))?;
        tx.pragma_update(None, "user_version", 1)?;
        tx.commit()?;
    }
    if version < 2 {
        let tx = db.transaction()?;
        tx.execute_batch(include_str!(
            "../../../migrations/002_advanced_analysis.sql"
        ))?;
        tx.pragma_update(None, "user_version", 2)?;
        tx.commit()?;
    }
    if version < 3 {
        let tx = db.transaction()?;
        tx.execute_batch(include_str!("../../../migrations/003_ir.sql"))?;
        tx.pragma_update(None, "user_version", 3)?;
        tx.commit()?;
    }
    if version < 4 {
        let tx = db.transaction()?;
        tx.execute_batch(include_str!("../../../migrations/004_native.sql"))?;
        tx.pragma_update(None, "user_version", 4)?;
        tx.commit()?;
    }
    if version < 5 {
        let tx = db.transaction()?;
        tx.execute_batch(include_str!("../../../migrations/005_guest_runtime.sql"))?;
        tx.pragma_update(None, "user_version", 5)?;
        tx.commit()?;
    }
    if version < 6 {
        let tx = db.transaction()?;
        tx.execute_batch(include_str!("../../../migrations/006_cafe_context.sql"))?;
        tx.pragma_update(None, "user_version", 6)?;
        tx.commit()?;
    }
    Ok(())
}
#[cfg(test)]
mod tests {
    #[test]
    fn migration_preserves_version_one_projects() -> crate::Result<()> {
        let mut db = rusqlite::Connection::open_in_memory()?;
        db.execute_batch(include_str!("../../../migrations/001_initial.sql"))?;
        db.pragma_update(None, "user_version", 1)?;
        db.execute(
            "INSERT INTO files VALUES('code/test.rpx',4,'0','hash','RPX',1)",
            [],
        )?;
        db.execute("INSERT INTO analysis_runs(status) VALUES('COMPLETED')", [])?;
        super::migrate(&mut db)?;
        assert_eq!(
            db.query_row::<String, _, _>("SELECT sha256 FROM files", [], |r| r.get(0))?,
            "hash"
        );
        assert_eq!(
            db.query_row::<String, _, _>("SELECT status FROM analysis_runs", [], |r| r.get(0))?,
            "COMPLETED"
        );
        assert_eq!(
            db.pragma_query_value::<i32, _>(None, "user_version", |r| r.get(0))?,
            super::SCHEMA_VERSION
        );
        Ok(())
    }
    #[test]
    fn migration_is_idempotent() -> crate::Result<()> {
        let mut db = rusqlite::Connection::open_in_memory()?;
        super::migrate(&mut db)?;
        super::migrate(&mut db)?;
        assert_eq!(
            db.pragma_query_value::<i32, _>(None, "user_version", |r| r.get(0))?,
            super::SCHEMA_VERSION
        );
        assert_eq!(
            db.pragma_query_value::<i32, _>(None, "foreign_keys", |r| r.get(0))?,
            1
        );
        assert!(db
            .execute(
                "INSERT INTO sections VALUES('missing',0,'',1,0,0,0,0,0)",
                []
            )
            .is_err());
        Ok(())
    }
}
