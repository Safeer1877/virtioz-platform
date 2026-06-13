use futures_util::{SinkExt, StreamExt};
use serde_json::Value;
use std::collections::HashMap;
use std::sync::Arc;
use tokio::net::TcpListener;
use tokio::sync::{mpsc, Mutex};
use tokio_tungstenite::accept_async;
use tokio_tungstenite::tungstenite::Message;

type PeerMap = Arc<Mutex<HashMap<String, mpsc::UnboundedSender<Message>>>>;
type ClusterMap = Arc<Mutex<HashMap<String, String>>>; // peer_id -> cluster_key

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    eprintln!("[virtioz-signaling] Starting Mesh Signaling Server on 0.0.0.0:8080...");
    let listener = TcpListener::bind("0.0.0.0:8080").await?;
    
    let peers: PeerMap = Arc::new(Mutex::new(HashMap::new()));
    let clusters: ClusterMap = Arc::new(Mutex::new(HashMap::new()));
    
    while let Ok((stream, addr)) = listener.accept().await {
        eprintln!("[virtioz-signaling] New connection from {}", addr);
        let peers_clone = Arc::clone(&peers);
        let clusters_clone = Arc::clone(&clusters);
        
        tokio::spawn(async move {
            if let Ok(ws_stream) = accept_async(stream).await {
                let (mut write, mut read) = ws_stream.split();
                let (tx, mut rx) = mpsc::unbounded_channel::<Message>();
                
                let mut registered_peer_id = None;
                
                // Writer task
                let writer_task = tokio::spawn(async move {
                    while let Some(msg) = rx.recv().await {
                        if write.send(msg).await.is_err() {
                            break;
                        }
                    }
                });
                
                while let Some(Ok(msg)) = read.next().await {
                    if let Message::Text(text) = msg {
                        if let Ok(json) = serde_json::from_str::<Value>(&text) {
                            let msg_type = json["type"].as_str().unwrap_or("");
                            
                            match msg_type {
                                "register" => {
                                    if let (Some(peer_id), Some(cluster_key)) = (json["peer_id"].as_str(), json["cluster_key"].as_str()) {
                                        let p_id = peer_id.to_string();
                                        registered_peer_id = Some(p_id.clone());
                                        peers_clone.lock().await.insert(p_id.clone(), tx.clone());
                                        clusters_clone.lock().await.insert(p_id.clone(), cluster_key.to_string());
                                        eprintln!("[virtioz-signaling] Registered peer: {} in cluster: {}", p_id, cluster_key);
                                    }
                                }
                                "discover_hosts" => {
                                    if let (Some(source_id), Some(cluster)) = (&registered_peer_id, {
                                        let c = clusters_clone.lock().await;
                                        if let Some(r) = &registered_peer_id { c.get(r).cloned() } else { None }
                                    }) {
                                        let peers_lock = peers_clone.lock().await;
                                        let clusters_lock = clusters_clone.lock().await;
                                        
                                        let mut hosts = Vec::new();
                                        for (peer_id, peer_cluster) in clusters_lock.iter() {
                                            // Find peers in the same cluster that are NOT routers (routers start with "core_")
                                            // and are not the requesting peer
                                            if peer_cluster == &cluster && peer_id != source_id && !peer_id.starts_with("core_") {
                                                hosts.push(serde_json::json!({ "peer_id": peer_id }));
                                            }
                                        }
                                        
                                        let reply = serde_json::json!({
                                            "type": "host_list",
                                            "hosts": hosts
                                        });
                                        let _ = tx.send(Message::Text(reply.to_string().into()));
                                    }
                                }
                                "offer" | "answer" | "ice-candidate" => {
                                    if let Some(target_peer_id) = json["target_peer_id"].as_str() {
                                        let peers_lock = peers_clone.lock().await;
                                        if let Some(target_tx) = peers_lock.get(target_peer_id) {
                                            eprintln!("[virtioz-signaling] Routing {} from {:?} to {}", msg_type, registered_peer_id, target_peer_id);
                                            let _ = target_tx.send(Message::Text(text.into()));
                                        }
                                    }
                                }
                                _ => {}
                            }
                        }
                    }
                }
                
                // Cleanup on disconnect
                if let Some(peer_id) = registered_peer_id {
                    eprintln!("[virtioz-signaling] Peer disconnected: {}", peer_id);
                    peers_clone.lock().await.remove(&peer_id);
                    clusters_clone.lock().await.remove(&peer_id);
                }
                writer_task.abort();
            }
        });
    }
    
    Ok(())
}
