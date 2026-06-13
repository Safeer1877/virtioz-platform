use futures_util::{SinkExt, StreamExt};
use serde_json::Value;
use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::process::Command;
use tokio::sync::{mpsc, Mutex};
use tokio_tungstenite::connect_async;
use tokio_tungstenite::tungstenite::Message;
use webrtc::api::APIBuilder;
use webrtc::data_channel::data_channel_message::DataChannelMessage;
use webrtc::ice_transport::ice_candidate::{RTCIceCandidate, RTCIceCandidateInit};
use webrtc::ice_transport::ice_server::RTCIceServer;
use webrtc::peer_connection::configuration::RTCConfiguration;
use webrtc::peer_connection::sdp::session_description::RTCSessionDescription;
use webrtc::peer_connection::RTCPeerConnection;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    eprintln!("[virtioz-agent] Swarm Agent starting...");
    
    let signaling_server = std::env::var("VIRTIOZ_SIGNALING_URL").unwrap_or_else(|_| "ws://localhost:8080".to_string());
    let peer_id = format!("agent_{}", uuid::Uuid::new_v4().as_simple().to_string()[..8].to_string());
    let cluster_key = std::env::var("VIRTIOZ_CLUSTER_KEY").unwrap_or_else(|_| "default".to_string());
    
    eprintln!("[virtioz-agent] Connecting to Mesh Signaling at {} as {}", signaling_server, peer_id);
    let (ws_stream, _) = connect_async(&signaling_server).await?;
    let (mut write, mut read) = ws_stream.split();
    
    // Register
    let reg_msg = serde_json::json!({
        "type": "register",
        "peer_id": peer_id,
        "cluster_key": cluster_key
    });
    write.send(Message::Text(reg_msg.to_string().into())).await?;
    
    let (ws_tx, mut ws_rx) = mpsc::channel::<String>(32);
    
    tokio::spawn(async move {
        while let Some(msg) = ws_rx.recv().await {
            let _ = write.send(Message::Text(msg.into())).await;
        }
    });

    let peer_connections: Arc<Mutex<HashMap<String, Arc<RTCPeerConnection>>>> = Arc::new(Mutex::new(HashMap::new()));
    
    while let Some(Ok(Message::Text(text))) = read.next().await {
        if let Ok(data) = serde_json::from_str::<Value>(&text) {
            let msg_type = data["type"].as_str().unwrap_or("");
            
            match msg_type {
                "offer" => {
                    if let Some(source_peer_id) = data["source_peer_id"].as_str() {
                        let source_peer_id = source_peer_id.to_string();
                        eprintln!("[virtioz-agent] Received WebRTC offer from Router: {}", source_peer_id);
                        
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
                            peer_connections.lock().await.insert(source_peer_id.clone(), Arc::clone(&pc));
                            
                            // Handle ICE Candidates
                            let ws_tx_ice = ws_tx.clone();
                            let p_id = peer_id.clone();
                            let t_id = source_peer_id.clone();
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
                            
                            // Handle Data Channel
                            pc.on_data_channel(Box::new(move |dc| {
                                eprintln!("[virtioz-agent] Data channel opened by Router");
                                let dc_clone = Arc::clone(&dc);
                                
                                dc.on_message(Box::new(move |msg: DataChannelMessage| {
                                    let dc_clone = Arc::clone(&dc_clone);
                                    Box::pin(async move {
                                        let msg_str = String::from_utf8(msg.data.to_vec()).unwrap_or_default();
                                        if let Ok(task) = serde_json::from_str::<Value>(&msg_str) {
                                            if task["type"] == "terminal" {
                                                let payload = task["payload"].as_str().unwrap_or("").to_string();
                                                let task_id = task["task_id"].as_str().unwrap_or("").to_string();
                                                
                                                eprintln!("[virtioz-agent] Executing task {}: {}", task_id, payload);
                                                
                                                let dc_exec = Arc::clone(&dc_clone);
                                                tokio::spawn(async move {
                                                    let mut cmd = Command::new("powershell.exe");
                                                    cmd.arg("-NoProfile").arg("-Command").arg(&payload);
                                                    cmd.stdout(std::process::Stdio::piped());
                                                    cmd.stderr(std::process::Stdio::piped());
                                                    
                                                    match cmd.spawn() {
                                                        Ok(mut child) => {
                                                            let stdout = child.stdout.take().unwrap();
                                                            let stderr = child.stderr.take().unwrap();
                                                            
                                                            let mut stdout_reader = BufReader::new(stdout).lines();
                                                            let mut stderr_reader = BufReader::new(stderr).lines();
                                                            
                                                            let dc_out = Arc::clone(&dc_exec);
                                                            let tid_out = task_id.clone();
                                                            let out_task = tokio::spawn(async move {
                                                                while let Ok(Some(line)) = stdout_reader.next_line().await {
                                                                    let msg = serde_json::json!({
                                                                        "type": "stream_chunk",
                                                                        "task_id": tid_out,
                                                                        "data": format!("{}\r\n", line)
                                                                    });
                                                                    let _ = dc_out.send_text(msg.to_string()).await;
                                                                }
                                                            });
                                                            
                                                            let dc_err = Arc::clone(&dc_exec);
                                                            let tid_err = task_id.clone();
                                                            let err_task = tokio::spawn(async move {
                                                                while let Ok(Some(line)) = stderr_reader.next_line().await {
                                                                    let msg = serde_json::json!({
                                                                        "type": "stream_chunk",
                                                                        "task_id": tid_err,
                                                                        "data": format!("{}\r\n", line)
                                                                    });
                                                                    let _ = dc_err.send_text(msg.to_string()).await;
                                                                }
                                                            });
                                                            
                                                            let _ = out_task.await;
                                                            let _ = err_task.await;
                                                            let status = child.wait().await.unwrap();
                                                            
                                                            let finish_msg = serde_json::json!({
                                                                "type": "result",
                                                                "task_id": task_id,
                                                                "exit_code": status.code().unwrap_or(0),
                                                                "data": "\r\n[Process completed]\r\n"
                                                            });
                                                            let _ = dc_exec.send_text(finish_msg.to_string()).await;
                                                        }
                                                        Err(e) => {
                                                            let err_msg = serde_json::json!({
                                                                "type": "result",
                                                                "task_id": task_id,
                                                                "error": e.to_string()
                                                            });
                                                            let _ = dc_exec.send_text(err_msg.to_string()).await;
                                                        }
                                                    }
                                                });
                                            }
                                        }
                                    })
                                }));
                                Box::pin(async {})
                            }));
                            
                            // Set Remote Description
                            if let Some(sdp) = data["sdp"]["sdp"].as_str() {
                                let offer = RTCSessionDescription::offer(sdp.to_string()).unwrap_or_default();
                                if let Ok(_) = pc.set_remote_description(offer).await {
                                    // Create Answer
                                    if let Ok(answer) = pc.create_answer(None).await {
                                        if let Ok(_) = pc.set_local_description(answer.clone()).await {
                                            let answer_msg = serde_json::json!({
                                                "type": "answer",
                                                "source_peer_id": peer_id,
                                                "target_peer_id": source_peer_id,
                                                "sdp": {
                                                    "type": "answer",
                                                    "sdp": answer.sdp
                                                }
                                            });
                                            let _ = ws_tx.send(answer_msg.to_string()).await;
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
                "ice-candidate" => {
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
                _ => {}
            }
        }
    }
    
    Ok(())
}
