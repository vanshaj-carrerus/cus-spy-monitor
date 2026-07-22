use futures_util::{SinkExt, StreamExt};
use log::{info, warn};
use serde_json::{json, Value};
use std::sync::atomic::{AtomicU64, Ordering};
use tokio::sync::{mpsc, oneshot};
use tokio_tungstenite::{connect_async, tungstenite::protocol::Message};

static MSG_REF: AtomicU64 = AtomicU64::new(1);

pub struct SupabaseSignaling {
    write_tx: mpsc::Sender<String>,
    channel_topic: String,
    join_ref: String,
}

impl SupabaseSignaling {
    pub async fn connect(
        supabase_url: &str,
        anon_key: &str,
        channel_topic: &str,
    ) -> Result<(Self, mpsc::Receiver<(String, Value)>), String> {
        let ws_base = supabase_url
            .replace("https://", "wss://")
            .replace("http://", "ws://");
        // Array-framed messages require protocol 2.0.0 (1.0.0 expects JSON objects).
        let ws_url = format!(
            "{}/realtime/v1/websocket?apikey={}&vsn=2.0.0",
            ws_base.trim_end_matches('/'),
            anon_key
        );

        info!("[P2P Signaling] Connecting WebSocket…");
        let (ws_stream, _) = connect_async(&ws_url)
            .await
            .map_err(|e| format!("WebSocket connect failed: {}", e))?;
        info!("[P2P Signaling] WebSocket connected");

        let (mut write, mut read) = ws_stream.split();
        let (write_tx, mut write_rx) = mpsc::channel::<String>(64);
        let (event_tx, event_rx) = mpsc::channel::<(String, Value)>(64);

        let topic = channel_topic.to_owned();
        let topic_for_join = topic.clone();
        let join_ref = next_ref();

        tokio::spawn(async move {
            while let Some(msg) = write_rx.recv().await {
                if write.send(Message::Text(msg)).await.is_err() {
                    break;
                }
            }
        });

        let (joined_tx, joined_rx) = oneshot::channel::<Result<(), String>>();
        let mut joined_tx = Some(joined_tx);

        let join_msg = json!([
            join_ref,
            join_ref,
            topic_for_join,
            "phx_join",
            {
                "config": {
                    "broadcast": { "ack": false, "self": false },
                    "presence": { "key": "", "enabled": false },
                    "postgres_changes": [],
                    "private": false
                },
                "access_token": anon_key
            }
        ]);
        write_tx
            .send(join_msg.to_string())
            .await
            .map_err(|e| format!("Failed to join channel: {}", e))?;
        info!("[P2P Signaling] Sent phx_join for {}", topic);

        let topic_for_read = topic.clone();
        let join_ref_for_read = join_ref.clone();
        tokio::spawn(async move {
            while let Some(msg) = read.next().await {
                let Ok(msg) = msg else { break };
                match msg {
                    Message::Text(text) => {
                        handle_text_frame(
                            &text,
                            &join_ref_for_read,
                            &topic_for_read,
                            &mut joined_tx,
                            &event_tx,
                        )
                        .await;
                    }
                    Message::Binary(bytes) => {
                        // Protocol v2 delivers broadcasts as binary USER_BROADCAST (0x04).
                        if let Some((evt_name, inner)) = parse_user_broadcast_binary(&bytes) {
                            info!("[P2P Signaling] ← broadcast {}", evt_name);
                            let _ = event_tx.send((evt_name, inner)).await;
                        } else if !bytes.is_empty() {
                            warn!(
                                "[P2P Signaling] Unparsed binary frame ({} bytes), head={:?}",
                                bytes.len(),
                                &bytes[..bytes.len().min(8)]
                            );
                        }
                    }
                    Message::Ping(_) | Message::Pong(_) => {}
                    Message::Close(_) => break,
                    _ => {}
                }
            }
            warn!("[P2P Signaling] WebSocket read loop ended");
            if let Some(tx) = joined_tx.take() {
                let _ = tx.send(Err("WebSocket closed before join ack".into()));
            }
        });

        let write_tx_for_heartbeat = write_tx.clone();
        tokio::spawn(async move {
            let mut heartbeat_ref = 0u64;
            loop {
                tokio::time::sleep(std::time::Duration::from_secs(25)).await;
                heartbeat_ref += 1;
                // Heartbeat has no join_ref (null).
                let hb = json!([
                    Value::Null,
                    heartbeat_ref.to_string(),
                    "phoenix",
                    "heartbeat",
                    {}
                ]);
                if write_tx_for_heartbeat.send(hb.to_string()).await.is_err() {
                    break;
                }
            }
        });

        tokio::time::timeout(std::time::Duration::from_secs(10), joined_rx)
            .await
            .map_err(|_| "Timed out waiting for channel join".to_string())?
            .map_err(|_| "Join acknowledgment channel closed".to_string())??;

        Ok((
            SupabaseSignaling {
                write_tx,
                channel_topic: topic,
                join_ref,
            },
            event_rx,
        ))
    }

    pub async fn broadcast(&self, event: &str, payload: Value) -> Result<(), String> {
        let msg_ref = next_ref();
        // Broadcast requires the channel join_ref as the first array element.
        let msg = json!([
            self.join_ref,
            msg_ref,
            self.channel_topic,
            "broadcast",
            {
                "type": "broadcast",
                "event": event,
                "payload": payload
            }
        ]);
        self.write_tx
            .send(msg.to_string())
            .await
            .map_err(|e| format!("Broadcast failed: {}", e))?;
        info!("[P2P Signaling] → broadcast {}", event);
        Ok(())
    }
}

fn value_as_ref_string(v: &Value) -> String {
    v.as_str()
        .map(|s| s.to_owned())
        .or_else(|| v.as_u64().map(|n| n.to_string()))
        .or_else(|| v.as_i64().map(|n| n.to_string()))
        .unwrap_or_default()
}

async fn handle_text_frame(
    text: &str,
    join_ref_for_read: &str,
    topic_for_read: &str,
    joined_tx: &mut Option<oneshot::Sender<Result<(), String>>>,
    event_tx: &mpsc::Sender<(String, Value)>,
) {
    let Ok(parsed) = serde_json::from_str::<Value>(text) else {
        return;
    };
    let Some(arr) = parsed.as_array() else {
        return;
    };
    if arr.len() < 5 {
        return;
    }

    let msg_ref = value_as_ref_string(&arr[1]);
    let event = arr[3].as_str().unwrap_or("");

    if event == "phx_reply" && msg_ref == join_ref_for_read {
        let status = arr[4]
            .get("status")
            .and_then(|v| v.as_str())
            .unwrap_or("error");
        if let Some(tx) = joined_tx.take() {
            let _ = if status == "ok" {
                info!("[P2P Signaling] Channel joined: {}", topic_for_read);
                tx.send(Ok(()))
            } else {
                tx.send(Err(format!("Channel join rejected: {}", arr[4])))
            };
        }
        return;
    }

    if event == "broadcast" {
        if let Some(payload) = arr[4].as_object() {
            let evt_name = payload
                .get("event")
                .and_then(|v| v.as_str())
                .unwrap_or("");
            if let Some(inner) = payload.get("payload") {
                info!("[P2P Signaling] ← broadcast {}", evt_name);
                let _ = event_tx
                    .send((evt_name.to_owned(), inner.clone()))
                    .await;
            }
        }
    }
}

/// Parse Supabase Realtime protocol v2 USER_BROADCAST binary frame (type 0x04).
/// Layout: type | topic_len | event_len | meta_len | payload_enc | topic | event | meta | payload
fn parse_user_broadcast_binary(bytes: &[u8]) -> Option<(String, Value)> {
    if let Some(parsed) = parse_user_broadcast_structured(bytes) {
        return Some(parsed);
    }
    // Fallback: some frames may omit/alter the header; recover event + JSON payload.
    parse_user_broadcast_heuristic(bytes)
}

fn parse_user_broadcast_structured(bytes: &[u8]) -> Option<(String, Value)> {
    if bytes.len() < 5 || bytes[0] != 0x04 {
        return None;
    }

    let topic_len = bytes[1] as usize;
    let event_len = bytes[2] as usize;
    let meta_len = bytes[3] as usize;
    let payload_enc = bytes[4];
    let mut offset = 5;

    if offset + topic_len + event_len + meta_len > bytes.len() {
        return None;
    }

    offset += topic_len; // topic
    let event = String::from_utf8_lossy(&bytes[offset..offset + event_len]).into_owned();
    offset += event_len;
    offset += meta_len; // metadata

    let payload_bytes = &bytes[offset..];
    let inner = if payload_enc == 1 || looks_like_json(payload_bytes) {
        serde_json::from_slice::<Value>(payload_bytes).ok()?
    } else {
        Value::String(String::from_utf8_lossy(payload_bytes).into_owned())
    };

    if event.is_empty() {
        return None;
    }
    Some((event, inner))
}

fn parse_user_broadcast_heuristic(bytes: &[u8]) -> Option<(String, Value)> {
    let text = String::from_utf8_lossy(bytes);
    let json_start = text.find('{')?;
    let inner = serde_json::from_str::<Value>(&text[json_start..]).ok()?;

    // Event name sits between topic and JSON, e.g. "...9084request-stream{...}"
    let before = &text[..json_start];
    const EVENTS: &[&str] = &[
        "request-stream",
        "sdp-offer",
        "sdp-answer",
        "ice-candidate",
        "stop-stream",
    ];
    let event = EVENTS
        .iter()
        .find(|e| before.ends_with(*e))
        .map(|e| (*e).to_owned())?;

    Some((event, inner))
}

fn looks_like_json(bytes: &[u8]) -> bool {
    matches!(bytes.first(), Some(b'{') | Some(b'['))
}

fn next_ref() -> String {
    MSG_REF.fetch_add(1, Ordering::Relaxed).to_string()
}

pub fn user_room_channel(username: &str) -> String {
    // Must match supabase-js: channel("room:user_X") → topic "realtime:room:user_X"
    format!("realtime:room:user_{}", username)
}
