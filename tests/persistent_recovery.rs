use raft_core::domain::Command;
use raft_core::raft::actor::{ActorMsg, RaftActor};
use raft_core::raft::state::RaftState;
use std::collections::HashMap;
use std::fs;
use std::time::{SystemTime, UNIX_EPOCH};
use tokio::sync::{mpsc, oneshot};

#[tokio::test]
async fn committed_data_survives_restart_without_a_snapshot() {
    let suffix = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let storage_dir = std::env::temp_dir().join(format!("hogyoku-recovery-{suffix}"));
    fs::create_dir_all(&storage_dir).unwrap();

    let node_id = 909;
    let (tx, rx) = mpsc::channel(100);
    let mut state = RaftState::new_with_config(
        node_id,
        HashMap::new(),
        "127.0.0.1:8909".to_string(),
        storage_dir.clone(),
    );
    state.become_leader();
    let actor = RaftActor::new(state, rx, tx.clone(), HashMap::new());
    let handle = tokio::spawn(actor.run());

    let (reply_to, response) = oneshot::channel();
    tx.send(ActorMsg::ClientRequest {
        cmd: Command::Set {
            key: "fragment".to_string(),
            value: "safe".to_string(),
        },
        reply_to,
    })
    .await
    .unwrap();
    assert_eq!(response.await.unwrap().unwrap(), "OK");
    handle.abort();
    let _ = handle.await;

    let (tx, rx) = mpsc::channel(100);
    let mut restored = RaftState::new_with_config(
        node_id,
        HashMap::new(),
        "127.0.0.1:8909".to_string(),
        storage_dir.clone(),
    );
    restored.become_leader();
    let actor = RaftActor::new(restored, rx, tx.clone(), HashMap::new());
    let handle = tokio::spawn(actor.run());

    let (reply_to, response) = oneshot::channel();
    tx.send(ActorMsg::ClientRequest {
        cmd: Command::Get {
            key: "fragment".to_string(),
        },
        reply_to,
    })
    .await
    .unwrap();
    assert_eq!(response.await.unwrap().unwrap(), "safe");

    handle.abort();
    let _ = handle.await;
    fs::remove_dir_all(storage_dir).unwrap();
}
