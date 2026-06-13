//! Virtioz Core Daemon (Swarm Router)
//!
//! A shim daemon that:
//! 1. Receives tasks over a local WebSocket (ws://127.0.0.1:9002) from the React UI
//! 2. Auto-discovers ALL Host executors via the Mesh Signaling Server
//! 3. Establishes WebRTC Data Channels to multiple hosts simultaneously
//! 4. Load-balances tasks across active, IDLE hosts and streams outputs back to UI

use futures_util::{SinkExt, StreamExt};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use std::time::Duration;
use tokio::signal;
use tokio::sync::{broadcast, mpsc, Mutex};
use tokio_tungstenite::connect_async;
use tokio_tungstenite::tungstenite::Message;
use webrtc::api::APIBuilder;
use webrtc::data_channel::data_channel_message::DataChannelMessage;
use webrtc::ice_transport::ice_candidate::{RTCIceCandidate, RTCIceCandidateInit};
use webrtc::peer_connection::configuration::RTCConfiguration;
use webrtc::peer_connection::sdp::session_description::RTCSessionDescription;
use webrtc::ice_transport::ice_server::RTCIceServer;
use webrtc::peer_connection::RTCPeerConnection;
use webrtc::data_channel::RTCDataChannel;

// ---------------------------------------------------------------------------
// Configuration
// ---------------------------------------------------------------------------

const MAX_QUEUE_WAIT_SECS: u64 = 300; // 5 minutes

// ---------------------------------------------------------------------------
// Types
// ---------------------------------------------------------------------------

#[derive(Debug, Serialize, Deserialize)]
struct Task {
    task_id: String,
    #[serde(rename = "type")]
    task_type: String,
    payload: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    token: Option<String>,
}

type SwarmChannels = Arc<Mutex<HashMap<String, Arc<RTCDataChannel>>>>;
type PeerConnections = Arc<Mutex<HashMap<String, Arc<RTCPeerConnection>>>>;
type BusyStates = Arc<Mutex<HashSet<String>>>;
type TaskMap = Arc<Mutex<HashMap<String, String>>>;

// ---------------------------------------------------------------------------
// Main
// ---------------------------------------------------------------------------

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    eprintln!("[virtioz-core] Swarm Router starting...");
    
    let (tx, _rx) = broadcast::channel::<String>(100);
    let (task_tx, mut task_rx) = mpsc::channel::<(String, String)>(100);
    
    let dashboard_tx = tx.clone();
    let dashboard_task_tx = task_tx.clone();
    
    // UI WebSocket Server Loop
    tokio::spawn(async move {
        if let Ok(listener) = tokio::net::TcpListener::bind("127.0.0.1:9002").await {
            eprintln!("[virtioz-core] Dashboard broadcaster listening on ws://127.0.0.1:9002");
            while let Ok((stream, _)) = listener.accept().await {
                let mut rx = dashboard_tx.subscribe();
                let task_tx_clone = dashboard_task_tx.clone();
                let tx_clone = dashboard_tx.clone();
                
                tokio::spawn(async move {
                    if let Ok(mut ws_stream) = tokio_tungstenite::accept_async(stream).await {
                        // Wait for first auth message
                        if let Some(Ok(Message::Text(text))) = ws_stream.next().await {
                            if let Ok(json_val) = serde_json::from_str::<Value>(&text) {
                                if json_val["type"] == "auth" && json_val["token"] == "virtioz_local_ipc_admin" {
                                    let _ = ws_stream.send(Message::Text(serde_json::json!({"type": "status", "message": "IPC Authenticated"}).to_string().into())).await;
                                    
                                    let (mut write, mut read) = ws_stream.split();
                                    
                                    // Start Writer
                                    tokio::spawn(async move {
                                        loop {
                                            match rx.recv().await {
                                                Ok(json_str) => {
                                                    let _ = write.send(Message::Text(json_str.into())).await;
                                                }
                                                Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => {
                                                    continue;
                                                }
                                                Err(_) => {
                                                    break;
                                                }
                                            }
                                        }
                                    });
                                    
                                    // Reader
                                    while let Some(msg) = read.next().await {
                                        if let Ok(Message::Text(text)) = msg {
                                            if let Ok(json_val) = serde_json::from_str::<Value>(&text) {
                                                if json_val["type"] == "task" {
                                                    let task_id = uuid::Uuid::new_v4().as_simple().to_string();
                                                    let payload = json_val["payload"].as_str().unwrap_or("").to_string();
                                                    
                                                    let task = Task {
                                                        task_id: task_id.clone(),
                                                        task_type: "terminal".to_string(),
                                                        payload,
                                                        token: None,
                                                    };
                                                    
                                                    if let Ok(task_json) = serde_json::to_string(&task) {
                                                        let status_msg = serde_json::json!({
                                                            "type": "status",
                                                            "message": format!("Task queued: {}", task_id)
                                                        });
                                                        let _ = tx_clone.send(status_msg.to_string());
                                                        
                                                        let _ = task_tx_clone.send((task_id, task_json)).await;
                                                    }
                                                } else if json_val["type"] == "stdin" {
                                                     let target_task_id = json_val["task_id"].as_str().unwrap_or("").to_string();
                                                     let _ = task_tx_clone.send((target_task_id, text.to_string())).await;
                                                }
                                            }
                                        }
                                    }
                                } else {
                                    let _ = ws_stream.send(Message::Text(serde_json::json!({"type": "status", "message": "IPC Auth Failed"}).to_string().into())).await;
                                }
                            }
                        }
                    }
                });
            }
        }
    });

    let swarm_channels: SwarmChannels = Arc::new(Mutex::new(HashMap::new()));
    let busy_states: BusyStates = Arc::new(Mutex::new(HashSet::new()));
    let task_map: TaskMap = Arc::new(Mutex::new(HashMap::new())); // Map task_id -> peer_id

    // Task Dispatcher (Intelligent Load Balancer)
    let swarm_channels_clone = Arc::clone(&swarm_channels);
    let busy_states_clone = Arc::clone(&busy_states);
    let task_map_clone = Arc::clone(&task_map);
    let tx_dispatch = tx.clone();
    tokio::spawn(async move {
        let mut round_robin_keys: Vec<String>;
        let mut rr_idx = 0;
        
        while let Some((task_id, task_json)) = task_rx.recv().await {
            
            // Fast-path routing for STDIN chunks
            if task_json.contains("\"type\":\"stdin\"") {
                let map = task_map_clone.lock().await;
                if let Some(peer_id) = map.get(&task_id) {
                    let channels = swarm_channels_clone.lock().await;
                    if let Some(dc) = channels.get(peer_id) {
                        let _ = dc.send_text(task_json.clone()).await;
                    }
                }
                continue; // Stdin routed. Skip load balancing.
            }

            let mut dispatched = false;
            let mut wait_secs = 0;
            
            while !dispatched && wait_secs < MAX_QUEUE_WAIT_SECS {
                let mut channels = swarm_channels_clone.lock().await;
                // Clean up closed channels
                channels.retain(|_, dc| dc.ready_state() == webrtc::data_channel::data_channel_state::RTCDataChannelState::Open);
                
                if channels.is_empty() {
                    eprintln!("[virtioz-core] Warning: No active agents in swarm to process task");
                    let err_json = serde_json::json!({"type": "result", "error": "No active nodes in swarm", "task_id": task_id});
                    let _ = tx_dispatch.send(err_json.to_string());
                    dispatched = true;
                    break;
                }
                
                // Update round-robin keys
                round_robin_keys = channels.keys().cloned().collect();
                round_robin_keys.sort(); // Stable ordering
                
                let mut selected_peer = None;
                let mut busy = busy_states_clone.lock().await;
                
                // Find an idle node
                for _ in 0..round_robin_keys.len() {
                    rr_idx = (rr_idx + 1) % round_robin_keys.len();
                    let peer_id = &round_robin_keys[rr_idx];
                    
                    if !busy.contains(peer_id) {
                        selected_peer = Some(peer_id.clone());
                        break;
                    }
                }
                
                if let Some(peer_id) = selected_peer {
                    if let Some(dc) = channels.get(&peer_id) {
                        // Mark as busy and record task mapping
                        busy.insert(peer_id.clone());
                        task_map_clone.lock().await.insert(task_id.clone(), peer_id.clone());
                        
                        if let Err(e) = dc.send_text(task_json.clone()).await {
                            eprintln!("[virtioz-core] Failed to send task to agent: {}", e);
                            busy.remove(&peer_id);
                            task_map_clone.lock().await.remove(&task_id);
                            let err_json = serde_json::json!({"type": "result", "error": format!("Send error: {}", e), "task_id": task_id});
                            let _ = tx_dispatch.send(err_json.to_string());
                        } else {
                            eprintln!("[virtioz-core] Task {} dispatched to swarm node {}", task_id, peer_id);
                            let status_msg = serde_json::json!({
                                "type": "status",
                                "message": format!("Task {} running on {}", task_id, peer_id)
                            });
                            let _ = tx_dispatch.send(status_msg.to_string());
                        }
                        dispatched = true;
                    }
                } else {
                    // All nodes busy, wait 1 second
                    drop(busy);
                    drop(channels);
                    tokio::time::sleep(Duration::from_secs(1)).await;
                    wait_secs += 1;
                }
            }
            
            if !dispatched {
                let err_json = serde_json::json!({"type": "result", "error": "Swarm is fully saturated (Queue timeout)", "task_id": task_id});
                let _ = tx_dispatch.send(err_json.to_string());
            }
        }
    });

    // Telemetry Loop
    let swarm_channels_clone_tel = Arc::clone(&swarm_channels);
    let tx_clone_tel = tx.clone();
    tokio::spawn(async move {
        loop {
            tokio::time::sleep(Duration::from_secs(1)).await;
            let mut channels = swarm_channels_clone_tel.lock().await;
            channels.retain(|_, dc| dc.ready_state() == webrtc::data_channel::data_channel_state::RTCDataChannelState::Open);
            let active_nodes = channels.len();
            
            let tel_json = serde_json::json!({
                "type": "telemetry",
                "active_nodes": active_nodes
            });
            let _ = tx_clone_tel.send(tel_json.to_string());
        }
    });

    // Run the WebRTC signaling logic
    loop {
        tokio::select! {
            _ = signal::ctrl_c() => {
                eprintln!("\n[virtioz-core] Shutting down gracefully.");
                break;
            }
            res = run_webrtc_swarm(Arc::clone(&swarm_channels), tx.clone(), Arc::clone(&busy_states), Arc::clone(&task_map)) => {
                if let Err(e) = res {
                    eprintln!("[virtioz-core] Swarm Router error: {e}. Reconnecting in 5s...");
                }
                tokio::time::sleep(Duration::from_secs(5)).await;
            }
        }
    }
    
    Ok(())
}

async fn run_webrtc_swarm(
    swarm_channels: SwarmChannels,
    tx_broadcast: broadcast::Sender<String>,
    busy_states: BusyStates,
    task_map: TaskMap,
) -> Result<(), Box<dyn std::error::Error>> {
    let signaling_server = std::env::var("VIRTIOZ_SIGNALING_URL").unwrap_or_else(|_| "ws://localhost:8080".to_string());
    let peer_id = format!("core_{}", uuid::Uuid::new_v4().as_simple().to_string()[..8].to_string());
    
    eprintln!("[virtioz-core] Connecting to mesh at {}", signaling_server);
    let (ws_stream, _) = connect_async(&signaling_server).await?;
    let (mut write, mut read) = ws_stream.split();
    
    let cluster_key = std::env::var("VIRTIOZ_CLUSTER_KEY").unwrap_or_else(|_| "default".to_string());
    
    // Register
    let reg_msg = serde_json::json!({
        "type": "register",
        "peer_id": peer_id,
        "cluster_key": cluster_key
    });
    write.send(Message::Text(reg_msg.to_string().into())).await?;
    
    let peer_connections: PeerConnections = Arc::new(Mutex::new(HashMap::new()));
    let (ws_tx, mut ws_rx) = mpsc::channel::<String>(32);
    
    // Write loop for signaling
    tokio::spawn(async move {
        while let Some(msg) = ws_rx.recv().await {
            let _ = write.send(Message::Text(msg.into())).await;
        }
    });

    // Discovery Loop
    let ws_tx_clone = ws_tx.clone();
    let peer_id_clone = peer_id.clone();
    tokio::spawn(async move {
        loop {
            let disc_msg = serde_json::json!({
                "type": "discover_hosts",
                "peer_id": peer_id_clone
            });
            let _ = ws_tx_clone.send(disc_msg.to_string()).await;
            tokio::time::sleep(Duration::from_secs(10)).await;
        }
    });

    let mut known_hosts = HashSet::new();

    while let Some(msg) = read.next().await {
        let msg = msg?;
        if let Message::Text(text) = msg {
            let data: Value = serde_json::from_str(&text).unwrap_or_default();
            
            if data["type"] == "host_list" {
                if let Some(hosts) = data["hosts"].as_array() {
                    for host in hosts {
                        let target_peer_id = host["peer_id"].as_str().unwrap_or("").to_string();
                        if target_peer_id.is_empty() || known_hosts.contains(&target_peer_id) {
                            continue;
                        }
                        
                        eprintln!("[virtioz-core] Found new agent: {}", target_peer_id);
                        known_hosts.insert(target_peer_id.clone());
                        
                        let api = APIBuilder::new().build();
                        let config = RTCConfiguration {
                            ice_servers: vec![RTCIceServer {
                                urls: vec!["stun:stun.l.google.com:19302".to_owned()],
                                ..Default::default()
                            }],
                            ..Default::default()
                        };
                        
                        if let Ok(pc) = api.new_peer_connection(config).await {
                            let pc = Arc::new(pc);
                            peer_connections.lock().await.insert(target_peer_id.clone(), Arc::clone(&pc));
                            
                            // Create DataChannel
                            if let Ok(dc) = pc.create_data_channel("virtioz_data", None).await {
                                let swarm_channels_clone = Arc::clone(&swarm_channels);
                                let dc_clone = Arc::clone(&dc);
                                let target_peer_id_clone = target_peer_id.clone();
                                
                                dc.on_open(Box::new(move || {
                                    eprintln!("[virtioz-core] Data channel opened with agent {}", target_peer_id_clone);
                                    let dc_clone = Arc::clone(&dc_clone);
                                    let swarm_channels_clone = Arc::clone(&swarm_channels_clone);
                                    let target_peer_id_clone = target_peer_id_clone.clone();
                                    Box::pin(async move {
                                        swarm_channels_clone.lock().await.insert(target_peer_id_clone, dc_clone);
                                    })
                                }));
                                
                                let busy_states_clone = Arc::clone(&busy_states);
                                let target_peer_id_clone = target_peer_id.clone();
                                let tx_broadcast_clone = tx_broadcast.clone();
                                let task_map_clone = Arc::clone(&task_map);
                                dc.on_message(Box::new(move |msg: DataChannelMessage| {
                                    let busy_states_clone = Arc::clone(&busy_states_clone);
                                    let target_peer_id_clone = target_peer_id_clone.clone();
                                    let tx_broadcast_clone = tx_broadcast_clone.clone();
                                    let task_map_clone = Arc::clone(&task_map_clone);
                                    Box::pin(async move {
                                        let msg_str = String::from_utf8(msg.data.to_vec()).unwrap_or_default();
                                        if let Ok(json_val) = serde_json::from_str::<Value>(&msg_str) {
                                            // Decorate message with source peer ID
                                            let mut final_msg = json_val.clone();
                                            if let Some(obj) = final_msg.as_object_mut() {
                                                obj.insert("source_peer".to_string(), Value::String(target_peer_id_clone.clone()));
                                            }
                                            
                                            eprintln!("[virtioz-core] Routing message from {}: {:?}", target_peer_id_clone, final_msg);
                                            
                                            // Broadcast to all UI websockets
                                            let _ = tx_broadcast_clone.send(final_msg.to_string());
                                            
                                            // Clear busy state ONLY on final result
                                            if json_val["type"] == "result" {
                                                busy_states_clone.lock().await.remove(&target_peer_id_clone);
                                                if let Some(t_id) = json_val["task_id"].as_str() {
                                                    task_map_clone.lock().await.remove(t_id);
                                                }
                                            }
                                        }
                                    })
                                }));
                                
                                // ICE Candidate gathering
                                let ws_tx_ice = ws_tx.clone();
                                let p_id = peer_id.clone();
                                let t_id = target_peer_id.clone();
                                pc.on_ice_candidate(Box::new(move |c: Option<RTCIceCandidate>| {
                                    let ws_tx_ice = ws_tx_ice.clone();
                                    let p_id = p_id.clone();
                                    let t_id = t_id.clone();
                                    Box::pin(async move {
                                        if let Some(c) = c {
                                            if let Ok(json) = c.to_json() {
                                                let msg = serde_json::json!({
                                                    "type": "ice-candidate",
                                                    "source_peer_id": p_id,
                                                    "target_peer_id": t_id,
                                                    "candidate": json
                                                });
                                                let _ = ws_tx_ice.send(msg.to_string()).await;
                                            }
                                        }
                                    })
                                }));
                                
                                // Create Offer
                                if let Ok(offer) = pc.create_offer(None).await {
                                    if let Ok(_) = pc.set_local_description(offer.clone()).await {
                                        let offer_msg = serde_json::json!({
                                            "type": "offer",
                                            "source_peer_id": peer_id,
                                            "target_peer_id": target_peer_id,
                                            "sdp": {
                                                "type": "offer",
                                                "sdp": offer.sdp
                                            }
                                        });
                                        let _ = ws_tx.send(offer_msg.to_string()).await;
                                    }
                                }
                            }
                        }
                    }
                }
            } else if data["type"] == "answer" {
                if let Some(source_peer_id) = data["source_peer_id"].as_str() {
                    let pcs = peer_connections.lock().await;
                    if let Some(pc) = pcs.get(source_peer_id) {
                        if let Some(sdp) = data["sdp"]["sdp"].as_str() {
                            let answer = RTCSessionDescription::answer(sdp.to_string()).unwrap_or_default();
                            let _ = pc.set_remote_description(answer).await;
                        }
                    }
                }
            } else if data["type"] == "ice-candidate" {
                if let Some(source_peer_id) = data["source_peer_id"].as_str() {
                    let pcs = peer_connections.lock().await;
                    if let Some(pc) = pcs.get(source_peer_id) {
                        if let Some(c) = data["candidate"].as_object() {
                            let init = RTCIceCandidateInit {
                                candidate: c["candidate"].as_str().unwrap_or("").to_string(),
                                sdp_mid: c.get("sdpMid").and_then(|v| v.as_str()).map(|s| s.to_string()),
                                sdp_mline_index: c.get("sdpMLineIndex").and_then(|v| v.as_u64()).map(|v| v as u16),
                                username_fragment: None,
                            };
                            let _ = pc.add_ice_candidate(init).await;
                        }
                    }
                }
            }
        }
    }
    
    Ok(())
}
