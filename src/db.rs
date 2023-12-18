use rusqlite::types::Value;
use rusqlite::{params, OptionalExtension};
use tokio_rusqlite::Connection;
use tracing::{debug, instrument, trace, Level};

use crate::proto;
use crate::proto::codec::Change;
use crate::{error::Result, other_err};

pub const CRSQL_TRACKED_TAG_WHOLE_DATABASE: i32 = 0;
pub const CRSQL_TRACKED_EVENT_RECEIVE: i32 = 0;

pub struct DB {
    conn: Connection,
}

impl DB {
    #[instrument(level = Level::DEBUG, err)]
    pub async fn new(db_path: Option<&str>, ext_path: &str) -> Result<DB> {
        let conn = match db_path {
            Some(path) => Connection::open(path).await,
            None => Connection::open_in_memory().await,
        }?;
        let load_ext_path = ext_path.to_owned();
        conn.call(move |c| {
            unsafe {
                c.load_extension_enable()?;
                let r = c.load_extension(load_ext_path, Some("sqlite3_crsqlite_init"))?;
                c.load_extension_disable()?;
                r
            };
            Ok(())
        })
        .await?;
        Ok(DB { conn })
    }

    #[instrument(skip(self), level = Level::DEBUG, ret, err)]
    pub async fn tracked_peer_version(&self, site_id: Vec<u8>) -> Result<i64> {
        let version = match self
            .conn
            .call(move |conn| {
                conn.query_row(
                    "
select max(coalesce(version, 0)) from crsql_tracked_peers
where site_id = ? and event = ?",
                    params![site_id, CRSQL_TRACKED_EVENT_RECEIVE],
                    |row| row.get::<usize, Option<i64>>(0),
                )
                .optional()
            })
            .await?
        {
            Some(Some(tracked_version)) => tracked_version,
            _ => 0,
        };
        Ok(version)
    }

    #[instrument(skip(self, changes), level = Level::DEBUG, ret, err)]
    pub async fn merge(&self, site_id: Vec<u8>, changes: Vec<Change>) -> Result<i64> {
        let result = self
            .conn
            .call(move |conn| {
                let tx = conn.transaction()?;
                let mut ins_changes = tx.prepare(
                    "
insert into crsql_changes (
    \"table\", pk, cid, val, col_version, db_version, site_id, cl, seq)
values (?, ?, ?, ?, ?, ?, ?, ?, ?);",
                )?;
                let mut max_db_version = 0;
                for change in changes {
                    if change.db_version > max_db_version {
                        max_db_version = change.db_version;
                    }
                    ins_changes.execute(params![
                        change.table,
                        change.pk,
                        change.cid,
                        change.val,
                        change.col_version,
                        change.db_version,
                        &site_id,
                        change.cl,
                        change.seq,
                    ])?;
                    trace!("merge change {:?} {:?}", change, site_id);
                }
                trace!(site_id = format!("{:?}", site_id), max_db_version);
                tx.execute(
                    "
insert into crsql_tracked_peers (site_id, version, tag, event)
values (?, ?, ?, ?)
on conflict do update set version = max(version, excluded.version)",
                    params![
                        site_id,
                        max_db_version,
                        CRSQL_TRACKED_TAG_WHOLE_DATABASE,
                        CRSQL_TRACKED_EVENT_RECEIVE
                    ],
                )?;
                drop(ins_changes);
                tx.commit()?;
                Ok(max_db_version)
            })
            .await?;
        Ok(result)
    }

    #[instrument(skip(self), level = Level::DEBUG, ret, err)]
    pub async fn close(self) -> Result<()> {
        // Finalize cr-sqlite db
        self.conn
            .call(|c| {
                c.query_row("SELECT crsql_finalize()", [], |_row| Ok(()))?;
                Ok(())
            })
            .await?;
        debug!("crsql_finalized");

        Ok(())
    }

    #[instrument(skip(self), level = Level::DEBUG, ret, err)]
    pub async fn status(&self) -> Result<(Vec<u8>, i64)> {
        let (site_id, db_version) = self.conn
        .call(|c| {
            c.query_row(
                "
select crsql_site_id(), coalesce((select max(db_version) from crsql_changes where site_id is null), 0);",
                [],
                |row| Ok((row.get::<usize, Vec<u8>>(0)?, row.get::<usize, i64>(1)?)),
            )
        })
        .await?;
        Ok((site_id, db_version))
    }

    #[instrument(skip(self), level = Level::DEBUG)]
    pub async fn changes(&self, since_db_version: i64) -> Result<(Vec<u8>, Vec<Change>)> {
        let (site_id, db_version) = self.status().await?;
        if since_db_version >= db_version {
            return Ok((site_id, vec![]));
        }
        let changes = self
            .conn
            .call(move |c| {
                let mut stmt = c.prepare(
                    "
select
    \"table\",
    pk,
    cid,
    val,
    col_version,
    db_version,
    cl,
    seq
from crsql_changes
where db_version > ?
and site_id is null",
                )?;
                let mut result = vec![];
                let mut rows = stmt.query([since_db_version])?;
                while let Some(row) = rows.next()? {
                    let change = Change {
                        table: row.get::<usize, String>(0)?,
                        pk: row.get::<usize, Vec<u8>>(1)?,
                        cid: row.get::<usize, String>(2)?,
                        val: row.get::<usize, Value>(3)?,
                        col_version: row.get::<usize, i64>(4)?,
                        db_version: row.get::<usize, i64>(5)?,
                        cl: row.get::<usize, i64>(6)?,
                        seq: row.get::<usize, i64>(7)?,
                    };
                    result.push(change);
                }
                Ok(result)
            })
            .await?;
        Ok((site_id, changes))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn version_advances() {
        let test_points: Vec<(i32, i32, String)> = vec![
            (1, 2, "👾".to_string()),
            (2, 3, "🤖".to_string()),
            (3, 5, "😀".to_string()),
            (5, 8, "🪣".to_string()),
        ];

        let db = DB::new(None, "target/debug/crsqlite")
            .await
            .expect("new db");
        let (site_id, version) = db.status().await.expect("status");
        assert_eq!(site_id.len(), 16, "site_id is expected length");
        assert_ne!(site_id.as_slice(), [0; 16], "site_id is not all zeroes");
        assert_eq!(version, 0i64, "initial version");

        // create a table and insert some rows
        db.conn
            .call(move |conn| {
                conn.execute(
                    "create table canvas (x integer, y integer, value text, primary key (x, y));",
                    [],
                )?;
                conn.query_row("select crsql_as_crr('canvas');", [], |_| Ok(()))?;
                for i in 0..test_points.len() {
                    conn.execute(
                        "insert into canvas (x, y, value) values (?, ?, ?)",
                        params![test_points[i].0, test_points[i].1, test_points[i].2],
                    )?;
                }
                Ok(())
            })
            .await
            .expect("create table with rows");

        // assert new version
        let (site_id_2, version_2) = db.status().await.expect("status");
        assert_eq!(site_id, site_id_2);
        assert_eq!(version_2, 4i64);

        db.close().await.expect("close db");
    }
}
