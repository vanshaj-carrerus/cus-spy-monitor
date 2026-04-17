use enigo::{Enigo, MouseControllable, MouseButton};
use scrap::{Display, Capturer};
use std::io::ErrorKind;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::Mutex;
use webrtc::api::media_engine::MediaEngine;
use webrtc::api::APIBuilder;
use webrtc::peer_connection::RTCPeerConnection;
use webrtc::peer_connection::sdp::session_description::RTCSessionDescription;
use webrtc::peer_connection::configuration::RTCConfiguration;
use webrtc::ice_transport::ice_server::RTCIceServer;
use webrtc::track::track_local::track_local_static_sample::TrackLocalStaticSample;
use webrtc::track::track_local::TrackLocal;
use webrtc::rtp_transceiver::rtp_codec::RTCRtpCodecCapability;
use webrtc::media::Sample;
use webrtc::data_channel::data_channel_message::DataChannelMessage;
use openh264::encoder::{Encoder, EncoderConfig};
use openh264::formats::YUVBuffer;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use futures_util::{StreamExt, SinkExt};
use tokio_tungstenite::{connect_async, tungstenite::protocol::Message};
use tokio::sync::mpsc;

#[derive(Serialize, Deserialize, Debug)]
struct SignalMessage {
    #[serde(rename = "type")]
    msg_type: String,
    sdp: Option<String>,
    candidate: Option<String>,
    #[serde(rename = "sdpMid")]
    sdp_mid: Option<String>,
    #[serde(rename = "sdpMLineIndex")]
    sdp_mline_index: Option<u16>,
    target_id: Option<String>,
    user_id: Option<String>,
}

pub struct RemoteControl {
    enigo: Enigo,
}

impl RemoteControl {
    pub fn new() -> Self {
        Self { enigo: Enigo::new() }
    }

    pub fn handle_command(&mut self, msg: &str) {
        if let Ok(cmd) = serde_json::from_str::<Value>(msg) {
            let cmd_type = cmd["type"].as_str().unwrap_or("");
            match cmd_type {
                "move" => {
                    let x = cmd["x"].as_f64().unwrap_or(0.0);
                    let y = cmd["y"].as_f64().unwrap_or(0.0);
                    // Assuming resolution is 1920x1080 for mapping or just use relative if enigo supports it
                    // The dashboard sends 0-1 normalized coords.
                    let (w, h) = (1920, 1080); // Default or get actual
                    self.enigo.mouse_move_to((x * w as f64) as i32, (y * h as f64) as i32);
                }
                "click" => {
                    let button = match cmd["button"].as_str().unwrap_or("left") {
                        "left" => MouseButton::Left,
                        "right" => MouseButton::Right,
                        _ => MouseButton::Left,
                    };
                    self.enigo.mouse_click(button);
                }
                _ => {}
            }
        }
    }
}

pub async fn start_fallback_webrtc(username: String) {
    let mut m = MediaEngine::default();
    m.register_default_codecs().expect("Failed to register codecs");

    let api = APIBuilder::new().with_media_engine(m).build();

    let config = RTCConfiguration {
        ice_servers: vec![RTCIceServer {
            urls: vec!["stun:stun.l.google.com:19302".to_owned()],
            ..Default::default()
        }],
        ..Default::default()
    };

    let pc = Arc::new(api.new_peer_connection(config).await.expect("Failed to create PC"));

    let video_track = Arc::new(TrackLocalStaticSample::new(
        RTCRtpCodecCapability {
            mime_type: "video/h264".to_owned(),
            ..Default::default()
        },
        "video".to_owned(),
        "webrtc-rs".to_owned(),
    ));

    pc.add_track(Arc::clone(&video_track) as Arc<dyn TrackLocal + Send + Sync>)
        .await
        .expect("Failed to add track");

    let remote_control = Arc::new(Mutex::new(RemoteControl::new()));

    pc.on_data_channel(Box::new(move |d| {
        let rc = Arc::clone(&remote_control);
        Box::pin(async move {
            let d_label = d.label().to_owned();
            if d_label == "control" {
                d.on_message(Box::new(move |msg: DataChannelMessage| {
                    let rc = Arc::clone(&rc);
                    Box::pin(async move {
                        let text = String::from_utf8_lossy(&msg.data);
                        let mut rc = rc.lock().await;
                        rc.handle_command(&text);
                    })
                }));
            }
        })
    }));

    // ICE Candidate handling
    let (cand_tx, cand_rx) = mpsc::channel(32);
    pc.on_ice_candidate(Box::new(move |c| {
        let cand_tx = cand_tx.clone();
        Box::pin(async move {
            if let Some(candidate) = c {
                let _ = cand_tx.send(candidate).await;
            }
        })
    }));

    // Screen capture loop
    let video_track_clone = Arc::clone(&video_track);
    std::thread::spawn(move || {
        let display = Display::primary().expect("Couldn't find primary display");
        let mut capturer = Capturer::new(display).expect("Couldn't begin capture");
        let (w, h) = (capturer.width(), capturer.height());

        let mut encoder = Encoder::with_config(EncoderConfig::new(w as u32, h as u32)).expect("Failed to create encoder");
        
        loop {
            std::thread::sleep(Duration::from_millis(66)); // ~15 FPS
            match capturer.frame() {
                Ok(frame) => {
                    let rgb = rgba_to_rgb(&frame);
                    let yuv_buffer = YUVBuffer::with_rgb(w, h, &rgb);
                    if let Ok(encoded_frame) = encoder.encode(&yuv_buffer) {
                        let mut data = Vec::new();
                        encoded_frame.write_vec(&mut data);
                        
                        let vt = Arc::clone(&video_track_clone);
                        if let Ok(handle) = tokio::runtime::Handle::try_current() {
                            handle.spawn(async move {
                                let _ = vt.write_sample(&Sample {
                                    data: data.into(),
                                    duration: Duration::from_millis(66),
                                    ..Default::default()
                                }).await;
                            });
                        }
                    }
                }
                Err(error) => {
                    if error.kind() != ErrorKind::WouldBlock {
                        eprintln!("Capture error: {}", error);
                        break;
                    }
                }
            }
        }
    });

    // Signaling
    let pc_clone = Arc::clone(&pc);
    tokio::spawn(async move {
        connect_mediator(pc_clone, username, cand_rx).await;
    });
}

async fn connect_mediator(pc: Arc<RTCPeerConnection>, username: String, mut cand_rx: mpsc::Receiver<webrtc::ice_transport::ice_candidate::RTCIceCandidate>) {
    let url = "wss://cus-monitor-mediator.onrender.com";
    let (ws_stream, _) = match connect_async(url).await {
        Ok(val) => val,
        Err(e) => {
            eprintln!("Failed to connect to mediator: {}", e);
            return;
        }
    };

    let (mut write, mut read) = ws_stream.split();

    // 1. Join the mediator as this user
    let join = SignalMessage {
        msg_type: "join".to_owned(),
        sdp: None,
        candidate: None,
        sdp_mid: None,
        sdp_mline_index: None,
        target_id: None,
        user_id: Some(username.clone()),
    };
    let _ = write.send(Message::Text(serde_json::to_string(&join).unwrap())).await;

    // 2. Main loop: handle messages from dashboard and send candidates
    loop {
        tokio::select! {
            Some(Ok(msg)) = read.next() => {
                if let Message::Text(text) = msg {
                    if let Ok(sig) = serde_json::from_str::<SignalMessage>(&text) {
                        match sig.msg_type.as_str() {
                            "offer" => {
                                if let Some(sdp) = sig.sdp {
                                    pc.set_remote_description(RTCSessionDescription::offer(sdp).unwrap()).await.unwrap();
                                    let answer = pc.create_answer(None).await.unwrap();
                                    pc.set_local_description(answer.clone()).await.unwrap();
                                    
                                    let response = SignalMessage {
                                        msg_type: "answer".to_owned(),
                                        sdp: Some(answer.sdp),
                                        candidate: None,
                                        sdp_mid: None,
                                        sdp_mline_index: None,
                                        target_id: None,
                                        user_id: Some(username.clone()),
                                    };
                                    let _ = write.send(Message::Text(serde_json::to_string(&response).unwrap())).await;
                                }
                            }
                            "candidate" => {
                                if let Some(candidate) = sig.candidate {
                                    pc.add_ice_candidate(webrtc::ice_transport::ice_candidate::RTCIceCandidateInit {
                                        candidate,
                                        sdp_mid: sig.sdp_mid,
                                        sdp_mline_index: sig.sdp_mline_index,
                                        ..Default::default()
                                    }).await.unwrap();
                                }
                            }
                            _ => {}
                        }
                    }
                }
            }
            Some(cand) = cand_rx.recv() => {
                let stats = cand.to_json().unwrap();
                let sig = SignalMessage {
                    msg_type: "candidate".to_owned(),
                    sdp: None,
                    candidate: Some(stats.candidate),
                    sdp_mid: stats.sdp_mid,
                    sdp_mline_index: stats.sdp_mline_index,
                    target_id: None,
                    user_id: Some(username.clone()),
                };
                let _ = write.send(Message::Text(serde_json::to_string(&sig).unwrap())).await;
            }
        }
    }
}

fn rgba_to_rgb(rgba: &[u8]) -> Vec<u8> {
    let mut rgb = Vec::with_capacity(rgba.len() * 3 / 4);
    for chunk in rgba.chunks_exact(4) {
        rgb.push(chunk[2]); // R
        rgb.push(chunk[1]); // G
        rgb.push(chunk[0]); // B
    }
    rgb
}
