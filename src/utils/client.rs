use crate::error::NodeError;
use crate::rpc::RaftServiceClient;
use tarpc::{client, tokio_serde::formats::Json};

pub async fn resolve_endpoint(endpoint: &str) -> anyhow::Result<std::net::SocketAddr> {
    tokio::net::lookup_host(endpoint)
        .await?
        .next()
        .ok_or_else(|| anyhow::anyhow!("No address found for {}", endpoint))
}

pub async fn connect_raft_client(endpoint: &str) -> anyhow::Result<RaftServiceClient> {
    let addr = resolve_endpoint(endpoint).await?;
    let transport = tarpc::serde_transport::tcp::connect(addr, Json::default).await?;
    Ok(RaftServiceClient::new(client::Config::default(), transport).spawn())
}

/// Executes a generic RPC call with a built-in redirection loop.
///
/// This helper function will attempt an RPC call on a given address. If the server
/// responds with a `NotLeader` error, it will automatically parse the new leader's
/// address and retry, up to a specified number of times.
pub async fn execute_with_redirect<F, T, Fut>(
    initial_addr: impl ToString,
    max_retries: u32,
    mut rpc_call: F,
) -> anyhow::Result<T>
where
    // The closure takes a client and returns a future.
    F: FnMut(RaftServiceClient) -> Fut,
    // The future resolves to the result of a tarpc RPC call.
    Fut: std::future::Future<Output = Result<Result<T, NodeError>, client::RpcError>>,
    T: Send + 'static,
{
    let mut addr = initial_addr.to_string();
    let mut retries = max_retries;

    loop {
        if retries == 0 {
            anyhow::bail!("Redirect limit reached.");
        }
        retries -= 1;

        println!("Connecting to server on {}...", addr);
        let client = match connect_raft_client(&addr).await {
            Ok(client) => client,
            Err(e) if retries > 0 => {
                eprintln!("Connection to {} failed: {}. Retrying...", addr, e);
                tokio::time::sleep(std::time::Duration::from_millis(250)).await;
                continue;
            }
            Err(e) => return Err(e),
        };

        match rpc_call(client).await {
            Ok(Ok(response)) => {
                // The RPC was successful and the server returned Ok(response).
                return Ok(response);
            }
            Ok(Err(NodeError::NotLeader { leader_addr })) => {
                // The server is a follower and told us who the leader is.
                if let Some(leader_str) = leader_addr {
                    println!("Not the leader. Redirecting to leader at {}...", leader_str);
                    addr = leader_str;
                    continue; // Retry the loop with the new address.
                } else {
                    anyhow::bail!("Not the leader, and no leader address was provided.");
                }
            }
            Ok(Err(e)) => {
                // The RPC was successful, but the server returned a non-redirect error.
                anyhow::bail!("Application error from node: {}", e);
            }
            Err(e) => {
                // The RPC itself failed (network error, etc.).
                anyhow::bail!("RPC transport error: {}", e);
            }
        }
    }
}
