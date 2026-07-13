// spotui-daemon: a minimal librespot control daemon for the HiBy R3 Pro II.
//
// Purpose: prove the no-phone control spine. Authenticate once (cached creds),
// then listen on a local Unix socket for simple text commands and drive the
// librespot Player directly. This is the engine the on-device UI will eventually
// call instead of a phone.
//
// Built against librespot v0.8.0's API, mirroring the patterns in the
// examples/play.rs and examples/play_connect.rs that ship with librespot.
//
// Commands (newline-terminated, sent to the socket):
//   LOAD <base62_track_id>   -> load + play a single track
//   PLAY                     -> resume
//   PAUSE                    -> pause
//   STOP                     -> stop
//   STATUS                   -> report current playback state
//   QUIT                     -> shut the daemon down
//
// Audio goes out through whichever backend `audio_backend::find` selects,
// same as the stock binary (we pass --backend on the command line / via the
// SinkBuilder). For the HiBy we use the pipe backend piped to aplay, exactly
// as already proven working.

use std::{process::exit, sync::Arc};

use tokio::{
    io::{AsyncBufReadExt, AsyncWriteExt, BufReader},
    net::{TcpListener, UnixListener},
    sync::RwLock,
};

use librespot::{
    core::{
        cache::Cache, config::SessionConfig, session::Session, spotify_id::SpotifyId, SpotifyUri,
    },
    metadata::{Metadata, Playlist, Track},
    playback::{
        audio_backend,
        config::{AudioFormat, PlayerConfig},
        mixer::{self, Mixer, MixerConfig},
        player::{Player, PlayerEvent},
    },
};

const SOCKET_PATH: &str = "/tmp/spotui.sock";
const TCP_ADDR: &str = "127.0.0.1:5599";
// Credentials cache lives on the persistent userdata partition (NOT the
// removable SD card), so auth works even when no SD card is inserted -- e.g.
// right after a firmware flash, where the SD is removed before reboot.
const CACHE_DIR: &str = "/usr/data/librespot-cache";

#[tokio::main]
async fn main() {
    let mut builder = env_logger::Builder::new();
    // Keep logging modest; trace is too heavy for the device.
    builder.parse_filters("librespot=info,spotui=info");
    builder.init();

    // --- Configuration -----------------------------------------------------
    let session_config = SessionConfig::default();
    let player_config = PlayerConfig::default();
    let audio_format = AudioFormat::default();

    // --- Authentication (from cache, like our working setup) ---------------
    // Cache holds the OAuth credentials + volume on the persistent partition.
    // Audio-file caching is DISABLED (3rd arg None): we don't want to fill the
    // limited userdata flash with cached audio, and enabling it without a size
    // limiter triggers "audio cache location is not configured" errors that
    // block playback. Streaming without an on-disk audio cache is fine.
    let cache = match Cache::new(Some(CACHE_DIR), Some(CACHE_DIR), None, None) {
        Ok(c) => c,
        Err(e) => {
            eprintln!("[spotui] failed to open cache at {CACHE_DIR}: {e}");
            exit(1);
        }
    };

    let credentials = match cache.credentials() {
        Some(c) => c,
        None => {
            eprintln!(
                "[spotui] no cached credentials at {CACHE_DIR}. \
                 Run the stock librespot OAuth flow once to populate the cache."
            );
            exit(1);
        }
    };

    // --- Session -----------------------------------------------------------
    eprintln!("[spotui] connecting...");
    let session = Session::new(session_config, Some(cache));
    if let Err(e) = session.connect(credentials, false).await {
        eprintln!("[spotui] error connecting: {e}");
        exit(1);
    }
    eprintln!("[spotui] connected as {:?}", session.username());

    // --- Player ------------------------------------------------------------
    // The backend is selected the same way the stock binary does it.
    // Passing None to find() yields the default-registered backend; for the
    // HiBy build we compile with the pipe backend always present (StdoutSink).
    let backend = audio_backend::find(Some("pipe".to_string()))
        .or_else(|| audio_backend::find(None))
        .expect("no audio backend available");

    // Software volume mixer (softvol). librespot attenuates the PCM by the
    // mixer's current volume before it reaches the pipe/aplay. We keep an Arc
    // to it so the command handlers can adjust volume live.
    let mixer_builder = mixer::find(Some("softvol")).expect("softvol mixer not found");
    let mixer: Arc<dyn Mixer> =
        mixer_builder(MixerConfig::default()).expect("failed to open softvol mixer");
    // Start at ~40% so it's audible but not blasting.
    mixer.set_volume((u16::MAX as f32 * 0.80) as u16);

    let player = Player::new(
        player_config,
        session.clone(),
        mixer.get_soft_volume(),
        move || backend(None, audio_format),
    );

    // Track the player's actual state from librespot events. This is shared
    // with all control connections so STATUS reports the latest known state.
    let playback_state = Arc::new(RwLock::new("STOPPED"));
    let event_state = playback_state.clone();
    let mut player_events = player.get_player_event_channel();

    tokio::spawn(async move {
        while let Some(event) = player_events.recv().await {
            let new_state = match event {
                PlayerEvent::Loading { .. } => Some("LOADING"),
                PlayerEvent::Playing { .. } => Some("PLAYING"),
                PlayerEvent::Paused { .. } => Some("PAUSED"),
                PlayerEvent::Stopped { .. } | PlayerEvent::EndOfTrack { .. } => Some("STOPPED"),
                PlayerEvent::Unavailable { .. } => Some("ERROR"),
                _ => None,
            };

            if let Some(new_state) = new_state {
                *event_state.write().await = new_state;
                eprintln!("[spotui] playback state -> {new_state}");
            }
        }
    });

    // Player::new already returns an Arc<Player> (its methods take &self and
    // it's designed to be shared), so we use it directly across connections.
    // No additional Arc wrapping needed.

    // --- Control listeners -------------------------------------------------
    // We listen on BOTH a Unix socket (for the eventual on-device UI) and a
    // TCP port on localhost (convenient for development: reach it from the
    // laptop via `adb forward tcp:5599 tcp:5599`, since the device has no nc).

    // Unix socket
    let _ = std::fs::remove_file(SOCKET_PATH); // clear any stale socket
    let unix_listener = match UnixListener::bind(SOCKET_PATH) {
        Ok(l) => l,
        Err(e) => {
            eprintln!("[spotui] failed to bind unix socket {SOCKET_PATH}: {e}");
            exit(1);
        }
    };
    eprintln!("[spotui] listening on unix:{SOCKET_PATH}");

    // TCP socket
    let tcp_listener = match TcpListener::bind(TCP_ADDR).await {
        Ok(l) => l,
        Err(e) => {
            eprintln!("[spotui] failed to bind tcp {TCP_ADDR}: {e}");
            exit(1);
        }
    };
    eprintln!("[spotui] listening on tcp:{TCP_ADDR}");

    // Accept on both concurrently. Each accepted connection is handed to
    // handle_conn, which is generic over any async read/write stream.
    // Both the player (for playback) and session (for browse/search) are shared,
    // plus the mixer (for volume control).
    let unix_player = player.clone();
    let unix_session = session.clone();
    let unix_mixer = mixer.clone();
    let unix_playback_state = playback_state.clone();
    let unix_task = tokio::spawn(async move {
        loop {
            match unix_listener.accept().await {
                Ok((stream, _addr)) => {
                    let player = unix_player.clone();
                    let session = unix_session.clone();
                    let mixer = unix_mixer.clone();
                    let playback_state = unix_playback_state.clone();
                    tokio::spawn(async move {
                        let (r, w) = stream.into_split();
                        handle_conn(r, w, player, session, mixer, playback_state).await;
                    });
                }
                Err(e) => eprintln!("[spotui] unix accept error: {e}"),
            }
        }
    });

    let tcp_player = player.clone();
    let tcp_session = session.clone();
    let tcp_mixer = mixer.clone();
    let tcp_playback_state = playback_state.clone();
    let tcp_task = tokio::spawn(async move {
        loop {
            match tcp_listener.accept().await {
                Ok((stream, _addr)) => {
                    let player = tcp_player.clone();
                    let session = tcp_session.clone();
                    let mixer = tcp_mixer.clone();
                    let playback_state = tcp_playback_state.clone();
                    tokio::spawn(async move {
                        let (r, w) = stream.into_split();
                        handle_conn(r, w, player, session, mixer, playback_state).await;
                    });
                }
                Err(e) => eprintln!("[spotui] tcp accept error: {e}"),
            }
        }
    });

    // Run until either listener task ends (they don't, normally).
    let _ = tokio::join!(unix_task, tcp_task);
}

/// Handle one control connection: read newline-terminated commands and drive
/// the player. Generic over the concrete stream type (Unix or TCP) so both
/// listeners share one implementation.
async fn handle_conn<R, W>(
    read_half: R,
    mut write_half: W,
    player: Arc<Player>,
    session: Session,
    mixer: Arc<dyn Mixer>,
    playback_state: Arc<RwLock<&'static str>>,
) where
    R: tokio::io::AsyncRead + Unpin,
    W: tokio::io::AsyncWrite + Unpin,
{
    let mut lines = BufReader::new(read_half).lines();

    while let Ok(Some(line)) = lines.next_line().await {
        let line = line.trim().to_string();
        if line.is_empty() {
            continue;
        }
        let mut parts = line.splitn(2, ' ');
        let cmd = parts.next().unwrap_or("").to_uppercase();
        let arg = parts.next().unwrap_or("").trim();

        let reply: String = match cmd.as_str() {
            "LOAD" => match SpotifyId::from_base62(arg) {
                Ok(id) => {
                    let track = SpotifyUri::Track { id };
                    player.load(track, true, 0);
                    format!("OK loading {arg}\n")
                }
                Err(e) => format!("ERR bad track id '{arg}': {e}\n"),
            },
            "PLAY" => {
                player.play();
                "OK play\n".to_string()
            }
            "PAUSE" => {
                player.pause();
                "OK pause\n".to_string()
            }
            "STOP" => {
                player.stop();
                "OK stop\n".to_string()
            }
            "STATUS" => {
                let state = *playback_state.read().await;
                format!("STATUS {state}\n")
            }
            "SEARCH" => search_tracks(&session, arg).await,
            "LIKED" => liked_tracks(&session).await,
            "PLAYLIST" => playlist_tracks(&session, arg).await,
            "VOL_UP" => {
                // Step ~6% of full range per press (~16 steps floor to ceiling).
                let step = (u16::MAX as u32 * 6 / 100) as u16;
                let cur = mixer.volume();
                let next = cur.saturating_add(step);
                mixer.set_volume(next);
                format!("OK vol {}\n", vol_percent(next))
            }
            "VOL_DOWN" => {
                let step = (u16::MAX as u32 * 6 / 100) as u16;
                let cur = mixer.volume();
                let next = cur.saturating_sub(step);
                mixer.set_volume(next);
                format!("OK vol {}\n", vol_percent(next))
            }
            "SETVOL" => match arg.parse::<u32>() {
                Ok(pct) if pct <= 100 => {
                    let v = (u16::MAX as u32 * pct / 100) as u16;
                    mixer.set_volume(v);
                    format!("OK vol {pct}\n")
                }
                _ => "ERR SETVOL needs 0-100\n".to_string(),
            },
            "GETVOL" => format!("VOL {}\n", vol_percent(mixer.volume())),
            "QUIT" => {
                let _ = write_half.write_all(b"OK bye\n").await;
                eprintln!("[spotui] quit requested");
                exit(0);
            }
            other => format!("ERR unknown command '{other}'\n"),
        };

        if write_half.write_all(reply.as_bytes()).await.is_err() {
            break;
        }
    }
}

/// Convert a u16 volume (0..=65535) to a 0..=100 percentage for replies.
fn vol_percent(v: u16) -> u32 {
    (v as u32 * 100) / u16::MAX as u32
}

/// Search via librespot's internal context-resolve (NOT the restricted Web API).
/// Resolves `spotify:search:<query>` into a Context, takes the first page's
/// tracks, enriches the top results with names via Track::get, and returns
/// tab-separated lines the UI can parse:
///
///   RESULT <track_id>\t<track_name>\t<artists>
///   ...
///   END <count>
///
/// On error or no results, returns a single status line.
async fn search_tracks(session: &Session, query: &str) -> String {
    if query.is_empty() {
        return "ERR empty query\n".to_string();
    }

    // Build the search URI: whitespace -> '+'
    let q = query.split_whitespace().collect::<Vec<_>>().join("+");
    let uri = format!("spotify:search:{q}");

    context_to_results(session, &uri).await
}

/// Fetch the current user's liked songs ("collection") as track results.
async fn liked_tracks(session: &Session) -> String {
    let user = session.username();
    let uri = format!("spotify:user:{user}:collection");
    context_to_results(session, &uri).await
}

/// Resolve any context URI (search, collection, artist, etc.) into enriched
/// track result lines. Takes the first non-empty page, caps the count, and
/// enriches names via Track::get.
async fn context_to_results(session: &Session, uri: &str) -> String {
    // Cap results. 50 gives enough to scroll while keeping on-device
    // enrichment (sequential Track::get calls) responsive.
    const MAX_RESULTS: usize = 50;

    let ctx = match session.spclient().get_context(uri).await {
        Ok(c) => c,
        Err(e) => return format!("ERR context failed: {e}\n"),
    };

    // Collect track URIs from the first page that has inline tracks.
    let mut track_uris: Vec<String> = Vec::new();
    for page in &ctx.pages {
        for t in &page.tracks {
            if let Some(u) = t.uri.as_ref() {
                if u.starts_with("spotify:track:") {
                    track_uris.push(u.clone());
                }
                if track_uris.len() >= MAX_RESULTS {
                    break;
                }
            }
        }
        if !track_uris.is_empty() {
            break; // first non-empty page is enough for v1
        }
    }

    enrich_track_uris(session, &track_uris).await
}

/// Fetch a specific playlist's tracks. Accepts either a bare base62 id or a
/// full `spotify:playlist:<id>` URI.
async fn playlist_tracks(session: &Session, playlist_arg: &str) -> String {
    if playlist_arg.is_empty() {
        return "ERR empty playlist id\n".to_string();
    }
    const MAX_RESULTS: usize = 50;

    // Build a playlist URI string, then parse it (avoids hand-constructing the
    // SpotifyUri::Playlist variant, which is user-scoped).
    let uri_str = if playlist_arg.starts_with("spotify:") {
        playlist_arg.to_string()
    } else {
        format!("spotify:playlist:{playlist_arg}")
    };

    let uri = match SpotifyUri::from_uri(&uri_str) {
        Ok(u) => u,
        Err(e) => return format!("ERR bad playlist uri '{uri_str}': {e}\n"),
    };

    let plist = match Playlist::get(session, &uri).await {
        Ok(p) => p,
        Err(e) => return format!("ERR playlist fetch failed: {e}\n"),
    };

    // Collect track URIs from the playlist contents.
    let mut track_uris: Vec<String> = Vec::new();
    for t in plist.tracks() {
        if let SpotifyUri::Track { id } = t {
            track_uris.push(format!(
                "spotify:track:{}",
                id.to_base62().unwrap_or_default()
            ));
        }
        if track_uris.len() >= MAX_RESULTS {
            break;
        }
    }

    enrich_track_uris(session, &track_uris).await
}

/// Shared enrichment: given a list of `spotify:track:<id>` URIs, fetch each
/// track's name + artists and format as RESULT lines, ending with END <count>.
async fn enrich_track_uris(session: &Session, track_uris: &[String]) -> String {
    if track_uris.is_empty() {
        return "END 0\n".to_string();
    }

    let mut out = String::new();
    let mut count = 0;
    for uri in track_uris {
        let base62 = uri.rsplit(':').next().unwrap_or("");
        let id = match SpotifyId::from_base62(base62) {
            Ok(id) => id,
            Err(_) => continue,
        };
        let track_uri = SpotifyUri::Track { id };
        match Track::get(session, &track_uri).await {
            Ok(track) => {
                let artists = track
                    .artists
                    .0
                    .iter()
                    .map(|a| a.name.clone())
                    .collect::<Vec<_>>()
                    .join(", ");
                out.push_str(&format!("RESULT {base62}\t{}\t{}\n", track.name, artists));
                count += 1;
            }
            Err(_) => {
                out.push_str(&format!("RESULT {base62}\t(unknown)\t\n"));
                count += 1;
            }
        }
    }

    out.push_str(&format!("END {count}\n"));
    out
}
