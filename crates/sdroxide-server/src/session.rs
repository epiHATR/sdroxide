//! Remote WebSocket sessions: Hello handshake, sign-in, codec negotiation,
//! three-lane sender, and the command/mic receive loop.
//!
//! Several clients may share one radio. On IQ front ends each client gets an
//! independent software VFO inside the shared capture bandwidth; TX is exclusive.

use std::sync::Arc;
use std::time::Duration;

use axum::extract::ws::{Message, WebSocket, WebSocketUpgrade};
use axum::extract::{Path, State};
use axum::response::{IntoResponse, Response};
use futures_util::{SinkExt, StreamExt};
use tokio::sync::mpsc;
use tracing::{info, warn};

use sdroxide_proto::{AudioCaps, AudioCodec, ClientMsg, PROTO_VERSION, ServerMsg, decode, encode};
use sdroxide_types::Command;

use crate::auth;
use crate::{Shared, Station};

/// `/ws` — the station's first radio, which is the whole of what a station
/// with one radio has. Every client that predates the roster arrives here, so
/// this address must never mean anything else.
pub async fn ws_route(State(station): State<Arc<Station>>, upgrade: WebSocketUpgrade) -> Response {
    let shared = station.first();
    upgrade.on_upgrade(|socket| session(socket, shared, station))
}

/// `/ws/<id>` — one named radio out of the station's roster. Unknown ids are
/// refused rather than rounded to a neighbour: a client that asked for the
/// Pluto and silently got the RTL-SDR would be operating the wrong radio.
pub async fn ws_route_for(
    State(station): State<Arc<Station>>,
    Path(id): Path<u32>,
    upgrade: WebSocketUpgrade,
) -> Response {
    match station.radio(id) {
        Some(shared) => upgrade.on_upgrade(|socket| session(socket, shared, station)),
        None => {
            let known: Vec<String> = station.list().iter().map(|r| r.id.to_string()).collect();
            (
                axum::http::StatusCode::NOT_FOUND,
                format!("this station has no radio {id}; it serves {}", known.join(", ")),
            )
                .into_response()
        }
    }
}

fn msg(m: &ServerMsg) -> Message {
    Message::Binary(encode(m).expect("encode").into())
}

async fn session(mut socket: WebSocket, shared: Arc<Shared>, station: Arc<Station>) {
    // Hello and the sign-in first, and only then a seat on the radio. Claiming
    // a seat before knowing who this is would let anyone who can open a socket
    // fill the client cap without ever proving they may touch it.
    let Some(audio_caps) = handshake(&mut socket, &shared).await else {
        let _ = socket.close().await;
        return;
    };

    let Some(session_id) = run_session(&mut socket, &shared, &station, audio_caps).await else {
        return;
    };

    // Cleanup — drop this client's listener seat and clear TX if it held the key.
    let _ = shared.cmd_tx.send(Command::DetachListener { id: session_id });
    let held_ptt = shared.remove_session(session_id);
    if held_ptt {
        let _ = shared.cmd_tx.send(Command::SetPtt(false));
        let _ = shared.cmd_tx.send(Command::SetTune(false));
        let _ = shared.cmd_tx.send(Command::CwKey(false));
    }
    info!(radio = shared.id, session = session_id, "remote session ended");
}

/// `Hello`, then the sign-in challenge if this server has one. `None` means the
/// socket is finished with — the caller closes it and claims nothing.
async fn handshake(socket: &mut WebSocket, shared: &Arc<Shared>) -> Option<AudioCaps> {
    let hello = tokio::time::timeout(Duration::from_secs(5), socket.recv()).await;
    let audio_caps = match hello {
        Ok(Some(Ok(Message::Binary(bytes)))) => match decode::<ClientMsg>(&bytes) {
            Ok(ClientMsg::Hello { proto, audio }) if proto == PROTO_VERSION => audio,
            Ok(ClientMsg::Hello { proto, .. }) => {
                let _ = socket
                    .send(msg(&ServerMsg::Error(format!(
                        "protocol mismatch: server {PROTO_VERSION}, client {proto}"
                    ))))
                    .await;
                return None;
            }
            _ => {
                let _ = socket.send(msg(&ServerMsg::Error("expected Hello".into()))).await;
                return None;
            }
        },
        _ => return None,
    };

    let signed_in = auth::challenge(
        socket,
        &shared.auth,
        auth::Frames {
            what: "/ws",
            required: encode(&ServerMsg::AuthRequired).expect("encode"),
            rejected: &|why| encode(&ServerMsg::AuthRejected(why.into())).expect("encode"),
            credentials: &|bytes| match decode::<ClientMsg>(bytes) {
                Ok(ClientMsg::Auth { username, password }) => Some((username, password)),
                _ => None,
            },
        },
    )
    .await;
    signed_in.then_some(audio_caps)
}

/// Tell the client that asked why a roster edit did not happen.
fn report<T>(
    shared: &Shared,
    outcome: Result<Result<T, String>, tokio::task::JoinError>,
    what: &str,
) {
    let why = match outcome {
        Ok(Ok(_)) => return,
        Ok(Err(e)) => e,
        Err(e) => format!("the station could not answer ({e})"),
    };
    warn!(radio = shared.id, "{what}: {why}");
    shared.broadcast_reliable(ServerMsg::Notice(Some(format!("{what}: {why}"))));
}

/// Run until the socket dies. Returns the session id that was registered, or
/// `None` if registration / greeting failed (Busy or early close).
async fn run_session(
    socket: &mut WebSocket,
    shared: &Arc<Shared>,
    roster: &Arc<Station>,
    audio_caps: AudioCaps,
) -> Option<u64> {
    let mut rx_codec =
        if audio_caps.opus_decode { AudioCodec::Opus48kMono } else { AudioCodec::Pcm16_48k };
    let tx_codec =
        if audio_caps.opus_encode { AudioCodec::Opus48kMono } else { AudioCodec::Pcm16_48k };

    // Match the radio's live codec before HelloAck so the client decodes what
    // the pump will actually send.
    if let Some(live) = shared.rx_codec() {
        rx_codec = live;
    }

    let (
        caps,
        state,
        memories,
        mem_folders,
        scanner,
        background_tasks,
        digi,
        voice,
        images,
        notice,
        station,
        tle_subs,
        sat_tle_listing,
        sat_track,
        radio,
        rds,
        ism_reports,
        ism_status,
        adsb_status,
        vdl2_status,
        ais_status,
        rs41_status,
        meshtastic_status,
        drm,
        hd,
        relay,
        profiles,
    ) = {
        let latest = shared.latest.lock().unwrap();
        (
            latest.caps.clone(),
            latest.state.clone(),
            latest.memories.clone(),
            latest.mem_folders.clone(),
            latest.scanner.clone(),
            latest.background_tasks.clone(),
            latest.digi.clone(),
            latest.voice.clone(),
            latest.images.clone(),
            latest.notice.clone(),
            latest.station.clone(),
            latest.tle_subs.clone(),
            latest.sat_tle_listing.clone(),
            latest.sat_track.clone(),
            latest.radio.clone(),
            latest.rds.clone(),
            latest.ism_reports.clone(),
            latest.ism_status.clone(),
            latest.adsb_status.clone(),
            latest.vdl2_status.clone(),
            latest.ais_status.clone(),
            latest.rs41_status.clone(),
            latest.meshtastic_status.clone(),
            latest.drm.clone(),
            latest.hd.clone(),
            latest.relay.clone(),
            latest.profiles.clone(),
        )
    };
    let ack = ServerMsg::HelloAck {
        proto: PROTO_VERSION,
        caps: caps.clone(),
        state: state.clone(),
        rx_codec,
        tx_codec,
    };
    if socket.send(msg(&ack)).await.is_err() {
        return None;
    }
    let _ = socket
        .send(msg(&ServerMsg::Radios {
            me: shared.id,
            radios: roster.roster(),
            editable: roster.editable(),
        }))
        .await;
    let _ = socket.send(msg(&ServerMsg::Memories(memories))).await;
    let _ = socket.send(msg(&ServerMsg::MemoryFolders(mem_folders))).await;
    let _ = socket.send(msg(&ServerMsg::Scanner(scanner))).await;
    let _ = socket.send(msg(&ServerMsg::BackgroundTasks(background_tasks))).await;
    let _ = socket.send(msg(&ServerMsg::Profiles(profiles))).await;
    if let Some(d) = digi {
        let _ = socket.send(msg(&ServerMsg::Ft8Status(d))).await;
    }
    if let Some(v) = voice {
        let _ = socket.send(msg(&ServerMsg::VoiceStatus(v))).await;
    }
    if let Some(d) = rds {
        let _ = socket.send(msg(&ServerMsg::Rds(d))).await;
    }
    if let Some(d) = drm {
        let _ = socket.send(msg(&ServerMsg::Drm(d))).await;
    }
    if let Some(d) = hd {
        let _ = socket.send(msg(&ServerMsg::Hd(d))).await;
    }
    if let Some(st) = ism_status {
        let _ = socket.send(msg(&ServerMsg::IsmStatus(st))).await;
    }
    if !ism_reports.is_empty() {
        let _ = socket.send(msg(&ServerMsg::IsmReports(ism_reports))).await;
    }
    if let Some(st) = adsb_status {
        let _ = socket.send(msg(&ServerMsg::AdsbStatus(st))).await;
    }
    if let Some(st) = vdl2_status {
        let _ = socket.send(msg(&ServerMsg::Vdl2Status(st))).await;
    }
    if let Some(st) = ais_status {
        let _ = socket.send(msg(&ServerMsg::AisStatus(st))).await;
    }
    if let Some(st) = rs41_status {
        let _ = socket.send(msg(&ServerMsg::Rs41Status(st))).await;
    }
    if let Some(st) = meshtastic_status {
        let _ = socket.send(msg(&ServerMsg::MeshtasticStatus(st))).await;
    }
    if let Some(p) = images {
        let _ = socket.send(msg(&ServerMsg::ImagePresets(p))).await;
    }
    if let Some(s) = station {
        let _ = socket.send(msg(&ServerMsg::StationConfig(s))).await;
        let _ = socket.send(msg(&ServerMsg::TleSubStatus(tle_subs))).await;
    }
    if let Some(text) = sat_tle_listing {
        let _ = socket.send(msg(&ServerMsg::SatTleListing(text))).await;
    }
    if let Some(r) = radio {
        let _ = socket.send(msg(&ServerMsg::RadioConfig(r))).await;
    }
    if sat_track.is_some() {
        let _ = socket.send(msg(&ServerMsg::SatTrack(sat_track))).await;
    }
    if let Some(r) = relay {
        let _ = socket.send(msg(&ServerMsg::RelayStatus(r))).await;
    }
    if notice.is_some() {
        let _ = socket.send(msg(&ServerMsg::Notice(notice))).await;
    }

    // --- register lanes -----------------------------------------------
    let (rel_tx, mut rel_rx) = mpsc::channel::<ServerMsg>(256);
    let (aud_tx, mut aud_rx) = mpsc::channel::<ServerMsg>(8);
    let session_id = match shared.try_add_session(rel_tx.clone(), aud_tx, rx_codec) {
        Ok(id) => id,
        Err(()) => {
            let _ = socket.send(msg(&ServerMsg::Busy)).await;
            let _ = socket.close().await;
            return None;
        }
    };
    // Independent VFO seat on IQ radios (no-op on CAT/audio-mode). Seed the
    // overlay immediately so dial commands rewrite before the engine answers.
    if !caps.audio_mode {
        let view = crate::ListenerView { freq_hz: state.rx_freq_hz(), rx: state.rx[0] };
        shared.listener_views.lock().unwrap().insert(session_id, view);
        let _ = shared.cmd_tx.send(Command::AttachListener { id: session_id });
    }
    info!(radio = shared.id, session = session_id, ?rx_codec, ?tx_codec, "remote client connected");

    let (mut ws_tx, mut ws_rx) = futures_util::StreamExt::split(socket);

    let mut spectrum_rx = shared.spectrum_rx.clone();
    let mut wide_rx = shared.wide_spectrum_rx.clone();
    let sender = async {
        let mut last_spectrum_seq = 0u32;
        let mut last_wide_seq = 0u32;
        loop {
            tokio::select! {
                biased;
                m = rel_rx.recv() => {
                    let Some(m) = m else { break };
                    if ws_tx.send(msg(&m)).await.is_err() { break; }
                }
                m = aud_rx.recv() => {
                    let Some(m) = m else { break };
                    if ws_tx.send(msg(&m)).await.is_err() { break; }
                }
                changed = wide_rx.changed() => {
                    if changed.is_err() { break; }
                    let frame = wide_rx.borrow_and_update().clone();
                    if let Some(f) = frame {
                        if f.seq != last_wide_seq {
                            last_wide_seq = f.seq;
                            if ws_tx.send(msg(&ServerMsg::WideSpectrum(f))).await.is_err() { break; }
                        }
                    }
                }
                changed = spectrum_rx.changed() => {
                    if changed.is_err() { break; }
                    let frame = spectrum_rx.borrow_and_update().clone();
                    if let Some(f) = frame {
                        if f.seq != last_spectrum_seq {
                            last_spectrum_seq = f.seq;
                            if ws_tx.send(msg(&ServerMsg::Spectrum(f))).await.is_err() { break; }
                        }
                    }
                }
            }
        }
    };

    let receiver = async {
        let mut opus_dec: Option<opus::Decoder> = None;
        let mut pcm = vec![0.0f32; 5760];
        while let Some(Ok(m)) = ws_rx.next().await {
            let Message::Binary(bytes) = m else {
                if matches!(m, Message::Close(_)) {
                    break;
                }
                continue;
            };
            match decode::<ClientMsg>(&bytes) {
                Ok(ClientMsg::Command(cmd)) => {
                    if shared.accept_tx_command(session_id, &cmd) {
                        let cmd = shared.route_command(session_id, cmd);
                        let _ = shared.cmd_tx.send(cmd);
                    }
                }
                Ok(ClientMsg::MicFrame { payload, .. }) => {
                    if !shared.holds_ptt(session_id) {
                        continue;
                    }
                    let n = match tx_codec {
                        AudioCodec::Opus48kMono => {
                            let dec = opus_dec.get_or_insert_with(|| {
                                opus::Decoder::new(48_000, opus::Channels::Mono)
                                    .expect("opus decoder")
                            });
                            match dec.decode_float(&payload, &mut pcm, false) {
                                Ok(n) => n,
                                Err(e) => {
                                    warn!("opus decode: {e}");
                                    continue;
                                }
                            }
                        }
                        AudioCodec::Pcm16_48k => {
                            let mut n = 0;
                            for (i, c) in payload.chunks_exact(2).enumerate() {
                                if i >= pcm.len() {
                                    break;
                                }
                                pcm[i] = i16::from_le_bytes([c[0], c[1]]) as f32 / 32768.0;
                                n += 1;
                            }
                            n
                        }
                    };
                    let mut mic = shared.mic_tx.lock().unwrap();
                    for &s in &pcm[..n] {
                        if mic.push(s).is_err() {
                            break;
                        }
                    }
                }
                Ok(ClientMsg::Probe(req)) => crate::probe::ask(shared, req),
                Ok(ClientMsg::AddRadio { name }) => {
                    let station = roster.clone();
                    let done = tokio::task::spawn_blocking(move || station.add_radio(&name)).await;
                    report(shared, done, "adding a radio");
                }
                Ok(ClientMsg::RemoveRadio { id }) => {
                    let station = roster.clone();
                    let done = tokio::task::spawn_blocking(move || station.remove_radio(id)).await;
                    report(shared, done, "closing a radio");
                }
                Ok(ClientMsg::RenameRadio { id, name }) => {
                    let station = roster.clone();
                    let done =
                        tokio::task::spawn_blocking(move || station.rename_radio(id, &name)).await;
                    report(shared, done, "renaming a radio");
                }
                Ok(ClientMsg::SetRadioEnabled { id, on }) => {
                    let station = roster.clone();
                    let done =
                        tokio::task::spawn_blocking(move || station.set_radio_power(id, on)).await;
                    report(shared, done, "switching a radio");
                }
                Ok(ClientMsg::Ping(t)) => {
                    // Answer on this client's lane only — not every peer.
                    let _ = rel_tx.try_send(ServerMsg::Pong(t));
                }
                Ok(ClientMsg::Hello { .. }) => {}
                Ok(ClientMsg::Auth { .. }) => {}
                Err(e) => warn!("bad client message: {e}"),
            }
        }
    };

    tokio::select! {
        _ = sender => {}
        _ = receiver => {}
    }
    Some(session_id)
}
