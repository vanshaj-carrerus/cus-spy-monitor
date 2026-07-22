use crate::supabase_signaling::{user_room_channel, SupabaseSignaling};
use log::{error, info, warn};
use openh264::encoder::{Encoder, EncoderConfig};
use openh264::formats::YUVBuffer;
use scrap::{Capturer, Display};
use serde::Deserialize;
use serde_json::json;
use std::collections::{HashMap, HashSet};
use std::io::ErrorKind;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::{mpsc, RwLock};
use webrtc::api::media_engine::{MediaEngine, MIME_TYPE_H264};
use webrtc::api::APIBuilder;
use webrtc::data_channel::data_channel_message::DataChannelMessage;
use webrtc::ice_transport::ice_candidate::RTCIceCandidateInit;
use webrtc::ice_transport::ice_server::RTCIceServer;
use webrtc::media::Sample;
use webrtc::peer_connection::configuration::RTCConfiguration;
use webrtc::peer_connection::sdp::session_description::RTCSessionDescription;
use webrtc::peer_connection::RTCPeerConnection;
use webrtc::rtp_transceiver::rtp_codec::RTCRtpCodecCapability;
use webrtc::track::track_local::track_local_static_sample::TrackLocalStaticSample;
use webrtc::track::track_local::TrackLocal;

#[derive(Clone, Debug)]
pub struct StreamStatus {
    pub active: bool,
    pub control_enabled: bool,
    pub controller_id: String,
}

struct AdminPeer {
    pc: Arc<RTCPeerConnection>,
}

struct P2PState {
    peers: HashMap<String, AdminPeer>,
    pending_admins: HashSet<String>,
    video_track: Option<Arc<TrackLocalStaticSample>>,
    capture_running: bool,
}

pub async fn run_p2p_streaming(
    username: String,
    supabase_url: String,
    anon_key: String,
    mut status_rx: tokio::sync::watch::Receiver<StreamStatus>,
) {
    let channel_name = user_room_channel(&username);
    info!("[P2P] Starting signaling on channel: {}", channel_name);

    let (signaling, mut events) = match SupabaseSignaling::connect(
        &supabase_url,
        &anon_key,
        &channel_name,
    )
    .await
    {
        Ok(val) => val,
        Err(e) => {
            error!("[P2P] Failed to connect signaling: {}", e);
            return;
        }
    };

    let signaling = Arc::new(signaling);
    let state = Arc::new(RwLock::new(P2PState {
        peers: HashMap::new(),
        pending_admins: HashSet::new(),
        video_track: None,
        capture_running: false,
    }));

    while status_rx.borrow().active {
        tokio::select! {
            changed = status_rx.changed() => {
                if changed.is_err() || !status_rx.borrow().active {
                    break;
                }
            }
            Some((event, payload)) = events.recv() => {
                let admin_id = payload.get("adminId").and_then(|v| v.as_str()).unwrap_or("").to_string();
                if admin_id.is_empty() { continue; }

                match event.as_str() {
                    "request-stream" => {
                        info!("[P2P] Handling request-stream from {}", admin_id);
                        let stream_status = status_rx.borrow().clone();
                        if let Err(e) = handle_request_stream(
                            &username,
                            &admin_id,
                            Arc::clone(&signaling),
                            Arc::clone(&state),
                            stream_status,
                        ).await {
                            warn!("[P2P] request-stream failed for {}: {}", admin_id, e);
                        }
                    }
                    "sdp-answer" => {
                        if let Some(sdp_val) = payload.get("sdp") {
                            if let Ok(sdp) = serde_json::from_value::<SdpInit>(sdp_val.clone()) {
                                let state_read = state.read().await;
                                if let Some(peer) = state_read.peers.get(&admin_id) {
                                    if let Ok(desc) = RTCSessionDescription::answer(sdp.sdp) {
                                        if let Err(e) = peer.pc.set_remote_description(desc).await {
                                            warn!("[P2P] set_remote_description failed: {}", e);
                                        }
                                    }
                                }
                            }
                        }
                    }
                    "ice-candidate" => {
                        if let Some(cand_val) = payload.get("candidate") {
                            if let Ok(cand) = serde_json::from_value::<CandidateInit>(cand_val.clone()) {
                                let state_read = state.read().await;
                                if let Some(peer) = state_read.peers.get(&admin_id) {
                                    let _ = peer.pc.add_ice_candidate(RTCIceCandidateInit {
                                        candidate: cand.candidate.unwrap_or_default(),
                                        sdp_mid: cand.sdp_mid,
                                        sdp_mline_index: cand.sdp_mline_index,
                                        ..Default::default()
                                    }).await;
                                }
                            }
                        }
                    }
                    "stop-stream" => {
                        let mut state_write = state.write().await;
                        if let Some(peer) = state_write.peers.remove(&admin_id) {
                            let _ = peer.pc.close().await;
                            info!("[P2P] Closed peer for admin {}", admin_id);
                        }
                    }
                    _ => {}
                }
            }
        }
    }

    let mut state_write = state.write().await;
    for (admin_id, peer) in state_write.peers.drain() {
        let _ = peer.pc.close().await;
        info!("[P2P] Cleaned up peer for admin {}", admin_id);
    }
    state_write.capture_running = false;
    info!("[P2P] Streaming stopped for {}", username);
}

#[derive(Deserialize)]
struct SdpInit {
    #[serde(rename = "type")]
    _sdp_type: String,
    sdp: String,
}

#[derive(Deserialize)]
struct CandidateInit {
    candidate: Option<String>,
    #[serde(rename = "sdpMid")]
    sdp_mid: Option<String>,
    #[serde(rename = "sdpMLineIndex")]
    sdp_mline_index: Option<u16>,
}

async fn handle_request_stream(
    username: &str,
    admin_id: &str,
    signaling: Arc<SupabaseSignaling>,
    state: Arc<RwLock<P2PState>>,
    stream_status: StreamStatus,
) -> Result<(), String> {
    {
        let mut state_write = state.write().await;
        if state_write.peers.contains_key(admin_id)
            || state_write.pending_admins.contains(admin_id)
        {
            return Ok(());
        }
        state_write.pending_admins.insert(admin_id.to_owned());
    }

    let result = handle_request_stream_inner(
        username,
        admin_id,
        signaling,
        Arc::clone(&state),
        stream_status,
    )
    .await;

    let peer_to_close = {
        let mut state_write = state.write().await;
        state_write.pending_admins.remove(admin_id);
        if result.is_err() {
            state_write.peers.remove(admin_id)
        } else {
            None
        }
    };
    if let Some(peer) = peer_to_close {
        let _ = peer.pc.close().await;
    }

    result
}

async fn handle_request_stream_inner(
    username: &str,
    admin_id: &str,
    signaling: Arc<SupabaseSignaling>,
    state: Arc<RwLock<P2PState>>,
    stream_status: StreamStatus,
) -> Result<(), String> {
    info!(
        "[P2P] Building peer for admin {} (control_enabled={}, controller_id={})",
        admin_id, stream_status.control_enabled, stream_status.controller_id
    );

    let mut m = MediaEngine::default();
    m.register_default_codecs()
        .map_err(|e| format!("codec registration: {}", e))?;
    let api = APIBuilder::new().with_media_engine(m).build();

    let config = RTCConfiguration {
        ice_servers: vec![RTCIceServer {
            urls: vec![
                "stun:stun.l.google.com:19302".to_owned(),
                "stun:stun1.l.google.com:19302".to_owned(),
            ],
            ..Default::default()
        }],
        ..Default::default()
    };

    let pc = Arc::new(
        api.new_peer_connection(config)
            .await
            .map_err(|e| format!("create PC: {}", e))?,
    );

    let video_track = {
        let mut state_write = state.write().await;
        if state_write.video_track.is_none() {
            let track = Arc::new(TrackLocalStaticSample::new(
                RTCRtpCodecCapability {
                    mime_type: MIME_TYPE_H264.to_owned(),
                    clock_rate: 90000,
                    channels: 0,
                    sdp_fmtp_line:
                        "level-asymmetry-allowed=1;packetization-mode=1;profile-level-id=42e01f"
                            .to_owned(),
                    rtcp_feedback: vec![],
                },
                "video".to_owned(),
                "p2p-screen".to_owned(),
            ));
            state_write.video_track = Some(Arc::clone(&track));

            if !state_write.capture_running {
                state_write.capture_running = true;
                let capture_track = Arc::clone(&track);
                let capture_state = Arc::clone(&state);
                let rt_handle = tokio::runtime::Handle::current();
                std::thread::spawn(move || {
                    run_screen_capture(capture_track, capture_state, rt_handle);
                });
            }
            track
        } else {
            Arc::clone(state_write.video_track.as_ref().unwrap())
        }
    };

    pc.add_track(Arc::clone(&video_track) as Arc<dyn TrackLocal + Send + Sync>)
        .await
        .map_err(|e| format!("add track: {}", e))?;

    let admin_id_owned = admin_id.to_owned();

    pc.on_data_channel(Box::new(move |d| {
        let label = d.label().to_owned();
        Box::pin(async move {
            if label == "remote-control" {
                info!("[P2P] Remote-control data channel opened");
                d.on_message(Box::new(move |msg: DataChannelMessage| {
                    Box::pin(async move {
                        if !super::remote_input::is_control_active() {
                            return;
                        }
                        let text = String::from_utf8_lossy(&msg.data);
                        match serde_json::from_str::<super::RemoteControlCommand>(&text) {
                            Ok(cmd) => {
                                #[cfg(target_os = "windows")]
                                super::remote_input::execute(cmd);
                            }
                            Err(e) => warn!("[P2P] Bad remote-control command: {}", e),
                        }
                    })
                }));
            }
        })
    }));

    let (cand_tx, mut cand_rx) = mpsc::channel(32);
    let signaling_for_ice = Arc::clone(&signaling);
    let admin_for_ice = admin_id_owned.clone();
    pc.on_ice_candidate(Box::new(move |c| {
        let cand_tx = cand_tx.clone();
        let admin = admin_for_ice.clone();
        let sig = Arc::clone(&signaling_for_ice);
        Box::pin(async move {
            if let Some(candidate) = c {
                if let Ok(json_cand) = candidate.to_json() {
                    let payload = json!({
                        "adminId": admin,
                        "candidate": {
                            "candidate": json_cand.candidate,
                            "sdpMid": json_cand.sdp_mid,
                            "sdpMLineIndex": json_cand.sdp_mline_index
                        }
                    });
                    let _ = sig.broadcast("ice-candidate", payload).await;
                }
            }
            let _ = cand_tx.send(()).await;
        })
    }));

    let offer = pc
        .create_offer(None)
        .await
        .map_err(|e| format!("create offer: {}", e))?;
    pc.set_local_description(offer.clone())
        .await
        .map_err(|e| format!("set local: {}", e))?;

    let offer_payload = json!({
        "adminId": admin_id_owned,
        "sdp": {
            "type": "offer",
            "sdp": offer.sdp
        }
    });
    signaling.broadcast("sdp-offer", offer_payload).await?;

    {
        let mut state_write = state.write().await;
        state_write.peers.insert(
            admin_id_owned.clone(),
            AdminPeer {
                pc: Arc::clone(&pc),
            },
        );
    }

    info!(
        "[P2P] Sent offer to admin {} for user {}",
        admin_id_owned, username
    );

    tokio::spawn(async move {
        while cand_rx.recv().await.is_some() {}
    });

    Ok(())
}

fn run_screen_capture(
    video_track: Arc<TrackLocalStaticSample>,
    state: Arc<RwLock<P2PState>>,
    rt_handle: tokio::runtime::Handle,
) {
    let display = match Display::primary() {
        Ok(d) => d,
        Err(e) => {
            error!("[P2P] No primary display: {}", e);
            return;
        }
    };
    let mut capturer = match Capturer::new(display) {
        Ok(c) => c,
        Err(e) => {
            error!("[P2P] Capture init failed: {}", e);
            return;
        }
    };
    let (w, h) = (capturer.width(), capturer.height());
    // H264 requires even dimensions.
    let enc_w = (w & !1) as u32;
    let enc_h = (h & !1) as u32;
    let mut encoder = match Encoder::with_config(
        EncoderConfig::new(enc_w, enc_h)
            .max_frame_rate(15.0)
            .set_bitrate_bps(2_500_000)
            .enable_skip_frame(false),
    ) {
        Ok(e) => e,
        Err(e) => {
            error!("[P2P] Encoder init failed: {}", e);
            return;
        }
    };

    info!("[P2P] Screen capture started ({}x{})", enc_w, enc_h);

    let frame_duration = Duration::from_millis(66);
    let mut frames_sent: u64 = 0;
    let mut last_keyframe = Instant::now() - Duration::from_secs(10);
    let mut was_sending = false;

    loop {
        let should_run = {
            let s = state.try_read();
            match s {
                Ok(st) => st.capture_running && !st.peers.is_empty(),
                Err(_) => true,
            }
        };
        if !should_run {
            was_sending = false;
            std::thread::sleep(Duration::from_millis(100));
            let s = state.try_read();
            if let Ok(st) = s {
                if !st.capture_running {
                    break;
                }
            }
            continue;
        }

        // First frame after peers appear (or every 2s) must be an IDR with SPS/PPS.
        // Samples written before the track is bound are silently dropped by webrtc-rs.
        if !was_sending {
            // Give DTLS / track bind a moment so the first keyframe isn't dropped.
            std::thread::sleep(Duration::from_millis(800));
            was_sending = true;
            last_keyframe = Instant::now() - Duration::from_secs(10);
        }
        let need_keyframe = last_keyframe.elapsed() >= Duration::from_secs(2);
        if need_keyframe {
            unsafe {
                let _ = encoder.raw_api().force_intra_frame(true);
            }
            last_keyframe = Instant::now();
        }

        std::thread::sleep(frame_duration);
        match capturer.frame() {
            Ok(frame) => {
                let mut rgb = match bgra_frame_to_rgb(&frame, w, h, enc_w as usize, enc_h as usize) {
                    Some(rgb) => rgb,
                    None => {
                        warn!("[P2P] Skipping frame with unexpected size");
                        continue;
                    }
                };
                #[cfg(target_os = "windows")]
                if let Some((cx, cy)) = cursor_position() {
                    draw_cursor_on_rgb(&mut rgb, enc_w as usize, enc_h as usize, cx, cy);
                }
                let yuv_buffer = YUVBuffer::with_rgb(enc_w as usize, enc_h as usize, &rgb);
                match encoder.encode(&yuv_buffer) {
                    Ok(encoded_frame) => {
                        let mut data = Vec::new();
                        // Ensure Annex-B start codes so the H264 RTP payloader can split NALs.
                        append_annex_b(&encoded_frame, &mut data);
                        if data.is_empty() {
                            continue;
                        }
                        let vt = Arc::clone(&video_track);
                        let dur = frame_duration;
                        frames_sent += 1;
                        if frames_sent == 1 || frames_sent % 30 == 0 {
                            info!(
                                "[P2P] Encoded frame #{} ({} bytes)",
                                frames_sent,
                                data.len()
                            );
                        }
                        rt_handle.spawn(async move {
                            if let Err(e) = vt
                                .write_sample(&Sample {
                                    data: data.into(),
                                    duration: dur,
                                    ..Default::default()
                                })
                                .await
                            {
                                warn!("[P2P] write_sample failed: {}", e);
                            }
                        });
                    }
                    Err(e) => warn!("[P2P] Encode failed: {}", e),
                }
            }
            Err(error) => {
                if error.kind() != ErrorKind::WouldBlock {
                    error!("[P2P] Capture error: {}", error);
                    break;
                }
            }
        }
    }

    info!("[P2P] Screen capture stopped");
}

#[cfg(target_os = "windows")]
fn cursor_position() -> Option<(i32, i32)> {
    use windows::Win32::Foundation::POINT;
    use windows::Win32::UI::WindowsAndMessaging::GetCursorPos;
    unsafe {
        let mut pt = POINT::default();
        GetCursorPos(&mut pt).ok()?;
        Some((pt.x, pt.y))
    }
}

#[inline]
fn put_rgb_pixel(rgb: &mut [u8], width: usize, height: usize, x: i32, y: i32, color: [u8; 3]) {
    if x < 0 || y < 0 || x >= width as i32 || y >= height as i32 {
        return;
    }
    let idx = ((y as u32 * width as u32 + x as u32) * 3) as usize;
    if idx + 2 >= rgb.len() {
        return;
    }
    rgb[idx] = color[0];
    rgb[idx + 1] = color[1];
    rgb[idx + 2] = color[2];
}

#[cfg(target_os = "windows")]
fn draw_cursor_on_rgb(rgb: &mut [u8], width: usize, height: usize, x: i32, y: i32) {
    let cursor_h = 24i32;
    let cursor_w = 16i32;

    for dy in 0..cursor_h {
        let row_w = ((dy as f32 / cursor_h as f32) * cursor_w as f32).max(1.0) as i32;
        for dx in 0..=row_w {
            put_rgb_pixel(rgb, width, height, x + dx + 2, y + dy + 2, [0, 0, 0]);
        }
    }
    for dy in 0..cursor_h {
        let row_w = ((dy as f32 / cursor_h as f32) * cursor_w as f32).max(1.0) as i32;
        put_rgb_pixel(rgb, width, height, x, y + dy, [0, 0, 0]);
        put_rgb_pixel(rgb, width, height, x + row_w, y + dy, [0, 0, 0]);
    }
    for dx in 0..cursor_w {
        put_rgb_pixel(rgb, width, height, x + dx, y, [0, 0, 0]);
    }
    put_rgb_pixel(rgb, width, height, x + 3, y + 18, [0, 0, 0]);
    put_rgb_pixel(rgb, width, height, x + 4, y + 19, [0, 0, 0]);
    put_rgb_pixel(rgb, width, height, x + 5, y + 20, [0, 0, 0]);

    for dy in 1..cursor_h - 1 {
        let row_w = ((dy as f32 / cursor_h as f32) * cursor_w as f32).max(1.0) as i32;
        for dx in 1..row_w {
            put_rgb_pixel(rgb, width, height, x + dx, y + dy, [245, 245, 245]);
        }
    }
    put_rgb_pixel(rgb, width, height, x + 4, y + 17, [170, 170, 170]);
    put_rgb_pixel(rgb, width, height, x + 5, y + 18, [170, 170, 170]);
}

/// Convert scrap BGRA frame (possibly with row stride padding) to tightly-packed RGB.
fn bgra_frame_to_rgb(
    frame: &[u8],
    width: usize,
    height: usize,
    enc_w: usize,
    enc_h: usize,
) -> Option<Vec<u8>> {
    if height == 0 || frame.len() < width * 4 {
        return None;
    }
    let stride = frame.len() / height;
    let row_bytes = width * 4;
    if stride < row_bytes {
        return None;
    }

    let mut rgb = Vec::with_capacity(enc_w * enc_h * 3);
    for y in 0..enc_h {
        let row = &frame[y * stride..y * stride + row_bytes];
        for x in 0..enc_w {
            let px = &row[x * 4..x * 4 + 4];
            // scrap is BGRA / bgr0
            rgb.push(px[2]);
            rgb.push(px[1]);
            rgb.push(px[0]);
        }
    }
    Some(rgb)
}

fn append_annex_b(encoded: &openh264::encoder::EncodedBitStream<'_>, dst: &mut Vec<u8>) {
    const START: [u8; 4] = [0, 0, 0, 1];
    for layer_idx in 0..encoded.num_layers() {
        let Some(layer) = encoded.layer(layer_idx) else {
            continue;
        };
        for nal_idx in 0..layer.nal_count() {
            let Some(nal) = layer.nal_unit(nal_idx) else {
                continue;
            };
            if nal.is_empty() {
                continue;
            }
            // OpenH264 may or may not include start codes; normalize to Annex-B.
            if nal.starts_with(&[0, 0, 0, 1]) || nal.starts_with(&[0, 0, 1]) {
                dst.extend_from_slice(nal);
            } else {
                dst.extend_from_slice(&START);
                dst.extend_from_slice(nal);
            }
        }
    }
}

#[derive(Deserialize)]
pub struct SignalingConfig {
    #[serde(rename = "supabaseUrl")]
    pub supabase_url: String,
    #[serde(rename = "supabaseAnonKey")]
    pub supabase_anon_key: String,
}
