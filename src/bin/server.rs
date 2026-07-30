use std::net::SocketAddr;

use axum::{extract::State, http::StatusCode, response::Json as AxumJson, routing::get, Router};
use clap::Parser;
use futures::StreamExt;
use raft_core::domain::{Command, LogEntry};
use raft_core::error::NodeError;
use raft_core::raft::actor::{ActorMsg, ActorStatus, RaftActor};
use raft_core::raft::state::RaftState;
use raft_core::rpc::{
    AppendEntriesReply, ApplyMembershipResponse, InstallSnapshotReply, RaftService,
    RequestVoteReply,
};
use raft_core::utils::client::connect_raft_client;
use std::time::Duration;
use tarpc::server::{self, Channel};
use tarpc::{context, tokio_serde::formats::Json};
use tokio::sync::{mpsc, oneshot};
use tracing_subscriber::EnvFilter;

#[derive(Parser, Debug)]
struct Args {
    #[arg(long, default_value = "127.0.0.1")]
    ip: std::net::IpAddr,
    #[arg(long)]
    port: u16,
    #[arg(long)]
    id: u64,
    #[arg(long, default_value = "")]
    peers: String,
    #[arg(long)]
    contact_node_address: Option<String>,
    #[arg(long)]
    advertise_address: Option<String>,
    #[arg(long, default_value = ".")]
    data_dir: String,
    #[arg(long, default_value_t = 8081)]
    health_port: u16,
}

#[derive(Clone, Debug)]
struct RaftNodeHandle {
    tx: mpsc::Sender<ActorMsg>,
}

impl RaftService for RaftNodeHandle {
    async fn execute(self, _: context::Context, cmd: Command) -> Result<String, NodeError> {
        let (reply_to, rx) = oneshot::channel();

        let msg = ActorMsg::ClientRequest { cmd, reply_to };

        if self.tx.send(msg).await.is_err() {
            return Err(NodeError::Internal("Actor is dead".into()));
        }

        rx.await
            .map_err(|_| NodeError::Internal("Actor did not reply".into()))?
    }

    async fn request_vote(
        self,
        _: context::Context,
        term: u64,
        candidate_id: u64,
        last_log_index: u64,
        last_log_term: u64,
    ) -> RequestVoteReply {
        let (reply_to, rx) = oneshot::channel();

        let msg = ActorMsg::RequestVote {
            term,
            candidate_id,
            last_log_index,
            last_log_term,
            reply_to,
        };

        let _ = self.tx.send(msg).await;

        rx.await.unwrap_or(RequestVoteReply {
            term: 0,
            vote_granted: false,
        })
    }

    async fn append_entries(
        self,
        _: context::Context,
        term: u64,
        leader_id: u64,
        prev_log_index: u64,
        prev_log_term: u64,
        entries: Vec<LogEntry>,
        leader_commit: u64,
    ) -> AppendEntriesReply {
        let (reply_to, rx) = oneshot::channel();

        let msg = ActorMsg::AppendEntries {
            term,
            leader_id,
            prev_log_index,
            prev_log_term,
            entries,
            leader_commit,
            reply_to,
        };

        let _ = self.tx.send(msg).await;

        rx.await.unwrap_or(AppendEntriesReply {
            term: 0,
            success: false,
        })
    }

    async fn install_snapshot(
        self,
        _: context::Context,
        term: u64,
        leader_id: u64,
        last_included_index: u64,
        last_included_term: u64,
        data: Vec<u8>,
        done: bool,
    ) -> InstallSnapshotReply {
        let (reply_to, rx) = oneshot::channel();

        let msg = ActorMsg::InstallSnapshot {
            term,
            leader_id,
            last_included_index,
            last_included_term,
            data,
            done,
            reply_to,
        };

        let _ = self.tx.send(msg).await;

        rx.await.unwrap_or(InstallSnapshotReply {
            term: 0,
            success: false,
        })
    }

    async fn request_log(self, _: context::Context) -> Vec<LogEntry> {
        let (reply_to, rx) = oneshot::channel();

        let msg = ActorMsg::RequestLog { reply_to };

        let _ = self.tx.send(msg).await;

        rx.await.unwrap_or_default()
    }

    async fn apply_membership(
        self,
        _: context::Context,
        node_id: u64,
        node_addr: String,
    ) -> Result<ApplyMembershipResponse, NodeError> {
        let (reply_to, rx) = oneshot::channel();

        let msg = ActorMsg::ApplyMembership {
            node_id,
            node_addr,
            reply_to,
        };

        if self.tx.send(msg).await.is_err() {
            return Err(NodeError::Internal("Actor is dead".into()));
        }

        rx.await
            .map_err(|_| NodeError::Internal("Actor did not reply".into()))?
    }

    async fn remove_membership(self, _: context::Context, node_id: u64) -> Result<(), NodeError> {
        let (reply_to, rx) = oneshot::channel();

        let msg = ActorMsg::RemoveMembership { node_id, reply_to };

        if self.tx.send(msg).await.is_err() {
            return Err(NodeError::Internal("Actor is dead".into()));
        }

        rx.await
            .map_err(|_| NodeError::Internal("Actor did not reply".into()))?
    }
}

fn parse_peers(peers_str: &str) -> std::collections::HashMap<u64, String> {
    let mut peer_map = std::collections::HashMap::new();
    if peers_str.is_empty() {
        return peer_map;
    }

    for s in peers_str.split(',') {
        if let Some((id_str, addr)) = s.split_once('=') {
            if let Ok(id) = id_str.parse::<u64>() {
                peer_map.insert(id, addr.to_string());
            }
        }
    }
    peer_map
}

async fn get_actor_status(tx: &mpsc::Sender<ActorMsg>) -> Option<ActorStatus> {
    let (reply_to, rx) = oneshot::channel();
    match tokio::time::timeout(
        Duration::from_secs(1),
        tx.send(ActorMsg::Status { reply_to }),
    )
    .await
    {
        Ok(Ok(())) => {}
        _ => return None,
    }

    match tokio::time::timeout(Duration::from_secs(1), rx).await {
        Ok(Ok(status)) => Some(status),
        _ => None,
    }
}

async fn live(State(tx): State<mpsc::Sender<ActorMsg>>) -> StatusCode {
    if get_actor_status(&tx).await.is_some() {
        StatusCode::OK
    } else {
        StatusCode::SERVICE_UNAVAILABLE
    }
}

async fn ready(State(tx): State<mpsc::Sender<ActorMsg>>) -> StatusCode {
    match get_actor_status(&tx).await {
        Some(status) if status.ready => StatusCode::OK,
        _ => StatusCode::SERVICE_UNAVAILABLE,
    }
}

async fn status(
    State(tx): State<mpsc::Sender<ActorMsg>>,
) -> Result<AxumJson<ActorStatus>, StatusCode> {
    get_actor_status(&tx)
        .await
        .map(AxumJson)
        .ok_or(StatusCode::SERVICE_UNAVAILABLE)
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    // --- 1. Basic Setup ---
    let args = Args::parse();
    let filter = EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| EnvFilter::new("info"))
        .add_directive("tarpc=warn".parse()?);
    tracing_subscriber::fmt().with_env_filter(filter).init();
    tracing::info!("Node {} starting on port {}", args.id, args.port);

    // Channel for RPCs to talk to the Actor
    let (tx, rx) = mpsc::channel::<ActorMsg>(100);

    // --- 2. Start RPC Server Listener (in background) ---
    let server_addr = SocketAddr::new(args.ip, args.port);
    let mut listener = tarpc::serde_transport::tcp::listen(server_addr, Json::default).await?;
    listener.config_mut().max_frame_length(usize::MAX);

    let listener_tx = tx.clone();
    tokio::spawn(async move {
        tracing::info!("RPC Server listening on {}", server_addr);
        // Accept connections and spawn a handler for each one
        while let Some(accept_result) = listener.next().await {
            match accept_result {
                Ok(transport) => {
                    let handle = RaftNodeHandle {
                        tx: listener_tx.clone(),
                    };
                    tokio::spawn(async move {
                        server::BaseChannel::with_defaults(transport)
                            .execute(handle.serve())
                            .for_each(|response_future| async move {
                                tokio::spawn(response_future);
                            })
                            .await;
                    });
                }
                Err(e) => {
                    tracing::error!("Failed to accept connection: {}", e);
                }
            }
        }
    });

    // Give the listener a moment to bind before we try to bootstrap.
    tokio::time::sleep(Duration::from_millis(100)).await;

    // --- 3. Connect to Peers ---
    let mut peers_map = parse_peers(&args.peers);
    peers_map.remove(&args.id);
    tracing::info!("Peers Config: {:?}", peers_map);
    let mut rpc_clients = std::collections::HashMap::new();
    for (id, addr_str) in &peers_map {
        tracing::info!("Connecting to peer {} at {}...", id, addr_str);
        if let Ok(client) = connect_raft_client(addr_str).await {
            rpc_clients.insert(*id, client);
            tracing::info!("Connected to peer {}", id);
        } else {
            tracing::warn!("Failed to connect to peer {} initially: {}", id, addr_str);
        }
    }

    // --- 4. Create Actor and Bootstrap (in foreground) ---
    let advertise_address = args
        .advertise_address
        .unwrap_or_else(|| format!("{}:{}", args.ip, args.port));
    let state =
        RaftState::new_with_config(args.id, peers_map.clone(), advertise_address, args.data_dir);
    let health_tx = tx.clone();
    let mut actor = RaftActor::new(state, rx, tx, rpc_clients);

    if let Some(contact_node_address) = args.contact_node_address {
        tracing::info!("Bootstrapping to {}", contact_node_address);
        if let Err(e) = actor.bootstrap(contact_node_address).await {
            tracing::error!("Bootstrap failed: {}. Shutting down.", e);
            return Ok(());
        }
        tracing::info!("Bootstrap successful.");
    }

    let health_addr = SocketAddr::from(([0, 0, 0, 0], args.health_port));
    let health_app = Router::new()
        .route("/live", get(live))
        .route("/ready", get(ready))
        .route("/status", get(status))
        .with_state(health_tx);
    let health_listener = tokio::net::TcpListener::bind(health_addr).await?;
    tokio::spawn(async move {
        tracing::info!("Health server listening on {}", health_addr);
        if let Err(e) = axum::serve(health_listener, health_app).await {
            tracing::error!("Health server stopped: {}", e);
        }
    });

    // --- 5. Run Actor's main loop (blocks forever) ---
    tracing::info!("Starting actor main loop.");
    actor.run().await;

    Ok(())
}
