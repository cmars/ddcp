use rusqlite::params;
use tokio_rusqlite::Connection;

use crate::{changes, status, Result};

use super::mocks::setup;

#[tokio::test]
async fn test_status_changes() {
    let (mut alice, alice_conn, _) = setup().await;

    let addr = alice.init().await.expect("init");
    assert_eq!(addr, "VLD0:AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA");

    let (push_addr, push_site_id, version) = alice.push().await.expect("push");
    assert_eq!(addr, push_addr);
    assert_eq!(version, 0);

    // No changes yet
    let (changes_site_id, empty_changes) = changes(&alice_conn, -1).await.expect("changes");
    assert_eq!(push_site_id, changes_site_id);
    assert!(empty_changes.is_empty());

    let points = vec![
        (1, 2, "👾".to_string()),
        (2, 3, "🤖".to_string()),
        (3, 5, "😀".to_string()),
        (5, 8, "🪣".to_string()),
    ];
    scenario_create_canvas(&alice_conn, points.clone())
        .await
        .expect("canvas scenario");

    let (_, changes) = changes(&alice_conn, 0).await.expect("changes");
    assert_eq!(changes.len(), 4);
}

#[tokio::test]
async fn test_sync_changes() {
    let (mut alice, alice_conn, _) = setup().await;
    let (mut bob, bob_conn, _) = setup().await;

    let _ = alice.init().await.expect("init");
    let _ = bob.init().await.expect("init");

    let alice_points = vec![
        (1, 2, "👾".to_string()),
        (2, 3, "🤖".to_string()),
        (3, 5, "😀".to_string()),
        (5, 8, "🪣".to_string()),
    ];
    scenario_create_canvas(&alice_conn, alice_points.clone())
        .await
        .expect("alice canvas scenario");
    scenario_create_canvas(&bob_conn, vec![(8, 13, "🐙".to_string())])
        .await
        .expect("bob canvas scenario");

    let (alice_site_id, alice_changes) = changes(&alice_conn, -1).await.expect("changes");
    assert_eq!(alice_changes.len(), 4);
    eprintln!("{:?}", alice_changes);

    let (_, alice_version) = status(&alice_conn).await.expect("alice status");
    assert_eq!(alice_version, 4); // 4 transactions

    let (bob_site_id, bob_changes) = changes(&bob_conn, -1).await.expect("changes");
    assert_eq!(bob_changes.len(), 1);
    eprintln!("{:?}", bob_changes);

    assert_ne!(alice_site_id, bob_site_id);

    let (_, bob_version) = status(&bob_conn).await.expect("bob status");
    assert_eq!(bob_version, 1); // 1 transaction

    // Bob merge changes from Alice
    let bob_stage_result = bob
        .merge(alice_site_id.clone(), alice_changes)
        .await
        .expect("bob stage");
    assert_eq!(bob_stage_result, alice_version);

    // Alice merges latest changes from Bob
    let alice_stage_result = alice
        .merge(bob_site_id.clone(), bob_changes)
        .await
        .expect("alice stage");
    assert_eq!(alice_stage_result, 1);

    alice_conn
        .call(|conn| {
            let mut stmt = conn.prepare("select * from canvas")?;
            let mut rows = stmt.query([])?;
            while let Some(row) = rows.next()? {
                eprintln!("alice {:?}", row);
            }
            Ok(())
        })
        .await
        .unwrap();

    bob_conn
        .call(|conn| {
            let mut stmt = conn.prepare("select * from canvas")?;
            let mut rows = stmt.query([])?;
            while let Some(row) = rows.next()? {
                eprintln!("bob {:?}", row);
            }
            Ok(())
        })
        .await
        .unwrap();

    let bob_expect = bob_conn
        .call(|conn| {
            conn.query_row(
                "select value from canvas where x = 1 and y = 2",
                [],
                |row| row.get::<usize, String>(0),
            )
        })
        .await
        .expect("bob conn query");
    assert_eq!(bob_expect, "👾");

    let alice_expect = alice_conn
        .call(|conn| {
            conn.query_row(
                "select value from canvas where x = 8 and y = 13",
                [],
                |row| row.get::<usize, String>(0),
            )
        })
        .await
        .expect("alice conn query");
    assert_eq!(alice_expect, "🐙");
}

async fn scenario_create_canvas(
    async_conn: &Connection,
    points: Vec<(i32, i32, String)>,
) -> Result<()> {
    async_conn
        .call(move |conn| {
            conn.execute(
                "create table canvas (x integer, y integer, value text, primary key (x, y));",
                [],
            )
            .expect("create table");
            conn.query_row("select crsql_as_crr('canvas');", [], |_| Ok(()))
                .expect("crsql_as_crr");

            for i in 0..points.len() {
                conn.execute(
                    "insert into canvas (x, y, value) values (?, ?, ?)",
                    params![points[i].0, points[i].1, points[i].2],
                )
                .expect("insert");
            }
            Ok(())
        })
        .await?;
    Ok(())
}
