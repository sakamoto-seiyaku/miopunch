use std::sync::{Arc, Mutex};
use std::time::Duration;
use iroh::{Endpoint, RelayMode};
use iroh::endpoint::presets;
use iroh::endpoint::transports::{PathSelector, PathSelection, PathSelectionContext};

#[derive(Debug, Default)]
struct Records { rows: Vec<(usize, Option<String>, Vec<String>)>, stopped: bool }
#[derive(Debug)]
struct RecordingSelector(Arc<Mutex<Records>>);
impl PathSelector for RecordingSelector {
    fn select(&self, ctx: &PathSelectionContext<'_>) -> PathSelection {
        let paths: Vec<_> = ctx.paths().collect();
        let mut state = self.0.lock().unwrap();
        if !state.stopped {
            state.rows.push((paths.len(), ctx.current().map(ToString::to_string),
                paths.iter().map(|p| p.network_path().to_string()).collect()));
        }
        let mut selection = PathSelection::none();
        if let Some(first) = paths.first() { selection.set(first); }
        selection
    }
}
async fn ping(conn: &iroh::endpoint::Connection) {
    let (mut send, mut recv) = conn.open_bi().await.unwrap();
    send.write_all(b"PING").await.unwrap(); send.finish().unwrap();
    assert_eq!(recv.read_to_end(16).await.unwrap(), b"PONG");
}
#[tokio::main]
async fn main() {
    let records = Arc::new(Mutex::new(Records::default()));
    let server = Endpoint::builder(presets::Minimal).clear_ip_transports().relay_mode(RelayMode::Disabled)
        .alpns(vec![b"m101/legacy".to_vec()]).bind_addr("127.0.0.1:0").unwrap().bind().await.unwrap();
    let client = Endpoint::builder(presets::Minimal).clear_ip_transports().relay_mode(RelayMode::Disabled)
        .path_selector(Arc::new(RecordingSelector(records.clone())))
        .bind_addr("127.0.0.1:0").unwrap().bind().await.unwrap();
    let addr = server.addr();
    let task = tokio::spawn({ let server = server.clone(); async move {
        let mut workers = Vec::new();
        for _ in 0..2 {
            let conn = server.accept().await.unwrap().await.unwrap();
            workers.push(tokio::spawn(async move {
                while let Ok((mut send, mut recv)) = conn.accept_bi().await {
                    assert_eq!(recv.read_to_end(16).await.unwrap(), b"PING");
                    send.write_all(b"PONG").await.unwrap(); send.finish().unwrap();
                }
            }));
        }
        for worker in workers { worker.await.unwrap(); }
    }});
    let a = client.connect(addr.clone(), b"m101/legacy").await.unwrap(); ping(&a).await;
    let b = client.connect(addr, b"m101/legacy").await.unwrap();
    assert_ne!(a.stable_id(), b.stable_id());
    for _ in 0..4 { ping(&a).await; ping(&b).await; tokio::time::sleep(Duration::from_millis(30)).await; }
    let rows = { let mut r = records.lock().unwrap(); r.stopped = true; r.rows.clone() };
    for (index, row) in rows.iter().enumerate() { println!("CALLBACK {index} count={} current={:?} paths={:?}", row.0, row.1, row.2); }
    let multi = rows.iter().position(|r| r.0 >= 2);
    let extra_single = multi.map(|start| rows[start..].iter().filter(|r| r.0 < 2).count()).unwrap_or(0);
    let selected: Vec<_> = [&a, &b].iter().map(|c| c.paths().iter()
        .filter(|p| p.is_selected()).map(|p| format!("{:?}",p.remote_addr())).collect::<Vec<_>>()).collect();
    println!("ACTUAL stable_ids={:?},{:?} selected={selected:?} extra_single={extra_single}",a.stable_id(),b.stable_id());
    a.close(0u32.into(), b"done"); b.close(0u32.into(), b"done");
    task.await.unwrap(); client.close().await; server.close().await;
    println!("ENDPOINT_CLOSE_RETURNED");
    assert!(multi.is_some(), "must actually observe both live same-peer connections");
    assert_eq!(extra_single, 0, "legacy peer selector received extra connection-local callbacks");
    assert!(selected.iter().all(|p| p.len()==1), "both real connections must select a path");
}
