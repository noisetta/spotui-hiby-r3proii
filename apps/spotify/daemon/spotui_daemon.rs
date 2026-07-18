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
//   LOAD_LIKED <track_id>     -> load within the Liked Songs queue
//   LOAD_PLAYLIST <playlist_id> <track_id>
//                            -> load within a playlist queue
//   PLAY                     -> resume
//   PAUSE                    -> pause
//   STOP                     -> stop
//   STATUS                   -> report current playback state
//   QUEUE_STATUS             -> report the active queue position
//   NOW_PLAYING              -> report current track metadata
//   POSITION                 -> report playback position in milliseconds
//   SEEK <milliseconds>      -> seek within the loaded track
//   QUIT                     -> shut the daemon down
//
// Audio goes out through whichever backend `audio_backend::find` selects,
// same as the stock binary (we pass --backend on the command line / via the
// SinkBuilder). For the HiBy we use the pipe backend piped to aplay, exactly
// as already proven working.

use std::{process::exit, sync::Arc, time::Duration};

use tokio::{
    io::{AsyncBufReadExt, AsyncWriteExt, BufReader},
    net::{TcpListener, UnixListener},
    sync::RwLock,
};

use librespot::{
    core::{
        cache::Cache, config::SessionConfig, session::Session, spotify_id::SpotifyId, SpotifyUri,
    },
    metadata::{audio::UniqueFields, Metadata, Playlist, Track},
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

struct NowPlaying {
    id: String,
    title: String,
    artist: String,
    duration_ms: u32,
}

enum QueueSource {
    Liked,
    Playlist(String),
}

#[derive(Default)]
struct PlaybackQueue {
    source: Option<QueueSource>,
    track_ids: Vec<String>,
    current_index: Option<usize>,
}

/// Keep line-oriented protocol fields from containing separators.
fn sanitize_field(value: &str) -> String {
    value
        .replace("\t", " ")
        .replace("\r", " ")
        .replace("\n", " ")
}

#[tokio::main]
async fn main() {
    let mut builder = env_logger::Builder::new();
    // Keep logging modest; trace is too heavy for the device.
    builder.parse_filters("librespot=info,spotui=info");
    builder.init();

    // --- Configuration -----------------------------------------------------
    let session_config = SessionConfig::default();
    let mut player_config = PlayerConfig::default();
    player_config.position_update_interval = Some(Duration::from_secs(1));
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

    // Track actual player state and current metadata from librespot events.
    let playback_state = Arc::new(RwLock::new("STOPPED"));
    let now_playing = Arc::new(RwLock::new(None::<NowPlaying>));
    let playback_position = Arc::new(RwLock::new(None::<u32>));
    let playback_queue =
        Arc::new(RwLock::new(PlaybackQueue::default()));
    let event_player = player.clone();
    let event_state = playback_state.clone();
    let event_now_playing = now_playing.clone();
    let event_position = playback_position.clone();
    let event_playback_queue = playback_queue.clone();
    let mut player_events = player.get_player_event_channel();

    tokio::spawn(async move {
        while let Some(event) = player_events.recv().await {
            if let PlayerEvent::EndOfTrack { track_id, .. } = &event {
                let ended_id = spotify_track_base62(track_id);
                let mut next_track = None;

                {
                    let mut queue = event_playback_queue.write().await;

                    if let (Some(current_index), Some(ended_id)) =
                        (queue.current_index, ended_id.as_deref())
                    {
                        let source_label = match queue.source.as_ref() {
                            Some(QueueSource::Liked) => {
                                "liked".to_string()
                            }
                            Some(QueueSource::Playlist(playlist_id)) => {
                                format!("playlist {playlist_id}")
                            }
                            None => "unknown".to_string(),
                        };

                        let current_matches = queue
                            .track_ids
                            .get(current_index)
                            .map(String::as_str)
                            == Some(ended_id);

                        if current_matches {
                            let next_index = current_index + 1;
                            let next_id =
                                queue.track_ids.get(next_index).cloned();

                            if let Some(next_id) = next_id {
                                let queue_len = queue.track_ids.len();
                                queue.current_index = Some(next_index);
                                next_track = Some((
                                    source_label,
                                    next_index,
                                    queue_len,
                                    next_id,
                                ));
                            } else {
                                eprintln!(
                                    "[spotui] {} queue finished at {}/{}",
                                    source_label,
                                    current_index + 1,
                                    queue.track_ids.len()
                                );
                                *queue = PlaybackQueue::default();
                            }
                        } else {
                            eprintln!(
                                "[spotui] ignored stale {} queue end event for {}",
                                source_label,
                                ended_id
                            );
                        }
                    }
                }

                if let Some((
                    source_label,
                    next_index,
                    queue_len,
                    next_id,
                )) = next_track
                {
                    match SpotifyId::from_base62(&next_id) {
                        Ok(id) => {
                            eprintln!(
                                "[spotui] {} queue advance -> {}/{} ({})",
                                source_label,
                                next_index + 1,
                                queue_len,
                                next_id
                            );
                            event_player.load(
                                SpotifyUri::Track { id },
                                true,
                                0,
                            );
                            continue;
                        }
                        Err(e) => {
                            eprintln!(
                                "[spotui] {} queue next id invalid '{}': {}",
                                source_label,
                                next_id,
                                e
                            );
                            *event_playback_queue.write().await =
                                PlaybackQueue::default();
                        }
                    }
                }
            }

            if matches!(&event, PlayerEvent::Unavailable { .. }) {
                *event_playback_queue.write().await =
                    PlaybackQueue::default();
                event_player.stop();
                eprintln!(
                    "[spotui] unavailable track; cleared queue and reset player"
                );
            }

            let new_state = match &event {
                PlayerEvent::Loading { .. } => Some("LOADING"),
                PlayerEvent::Playing { .. } => Some("PLAYING"),
                PlayerEvent::Paused { .. } => Some("PAUSED"),
                PlayerEvent::Stopped { .. } | PlayerEvent::EndOfTrack { .. } => Some("STOPPED"),
                PlayerEvent::Unavailable { .. } => Some("STOPPED"),
                _ => None,
            };

            if let Some(new_state) = new_state {
                *event_state.write().await = new_state;
                eprintln!("[spotui] playback state -> {new_state}");
            }

            let position_update = match &event {
                PlayerEvent::Loading { position_ms, .. }
                | PlayerEvent::Playing { position_ms, .. }
                | PlayerEvent::Paused { position_ms, .. }
                | PlayerEvent::PositionCorrection { position_ms, .. }
                | PlayerEvent::PositionChanged { position_ms, .. }
                | PlayerEvent::Seeked { position_ms, .. } => {
                    Some(Some(*position_ms))
                }
                PlayerEvent::Stopped { .. }
                | PlayerEvent::EndOfTrack { .. }
                | PlayerEvent::Unavailable { .. } => Some(None),
                _ => None,
            };

            if let Some(updated_position) = position_update {
                *event_position.write().await = updated_position;
            }

            match event {
                PlayerEvent::TrackChanged { audio_item } => {
                    let artist = match &audio_item.unique_fields {
                        UniqueFields::Track { artists, .. } => artists
                            .0
                            .iter()
                            .map(|artist| artist.name.clone())
                            .collect::<Vec<_>>()
                            .join(", "),
                        UniqueFields::Local { artists, .. } => {
                            artists.clone().unwrap_or_default()
                        }
                        UniqueFields::Episode { show_name, .. } => show_name.clone(),
                    };

                    let id = audio_item
                        .uri
                        .rsplit(":")
                        .next()
                        .unwrap_or(&audio_item.uri);

                    let item = NowPlaying {
                        id: sanitize_field(id),
                        title: sanitize_field(&audio_item.name),
                        artist: sanitize_field(&artist),
                        duration_ms: audio_item.duration_ms,
                    };

                    eprintln!(
                        "[spotui] now playing -> {} - {}",
                        item.title, item.artist
                    );
                    *event_now_playing.write().await = Some(item);
                }
                PlayerEvent::Stopped { .. }
                | PlayerEvent::EndOfTrack { .. }
                | PlayerEvent::Unavailable { .. } => {
                    *event_now_playing.write().await = None;
                }
                _ => {}
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
    let unix_now_playing = now_playing.clone();
    let unix_playback_position = playback_position.clone();
    let unix_playback_queue = playback_queue.clone();
    let unix_task = tokio::spawn(async move {
        loop {
            match unix_listener.accept().await {
                Ok((stream, _addr)) => {
                    let player = unix_player.clone();
                    let session = unix_session.clone();
                    let mixer = unix_mixer.clone();
                    let playback_state = unix_playback_state.clone();
                    let now_playing = unix_now_playing.clone();
                    let playback_position = unix_playback_position.clone();
                    let playback_queue = unix_playback_queue.clone();
                    tokio::spawn(async move {
                        let (r, w) = stream.into_split();
                        handle_conn(
                            r,
                            w,
                            player,
                            session,
                            mixer,
                            playback_state,
                            now_playing,
                            playback_position,
                            playback_queue,
                        )
                        .await;
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
    let tcp_now_playing = now_playing.clone();
    let tcp_playback_position = playback_position.clone();
    let tcp_playback_queue = playback_queue.clone();
    let tcp_task = tokio::spawn(async move {
        loop {
            match tcp_listener.accept().await {
                Ok((stream, _addr)) => {
                    let player = tcp_player.clone();
                    let session = tcp_session.clone();
                    let mixer = tcp_mixer.clone();
                    let playback_state = tcp_playback_state.clone();
                    let now_playing = tcp_now_playing.clone();
                    let playback_position = tcp_playback_position.clone();
                    let playback_queue = tcp_playback_queue.clone();
                    tokio::spawn(async move {
                        let (r, w) = stream.into_split();
                        handle_conn(
                            r,
                            w,
                            player,
                            session,
                            mixer,
                            playback_state,
                            now_playing,
                            playback_position,
                            playback_queue,
                        )
                        .await;
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
    now_playing: Arc<RwLock<Option<NowPlaying>>>,
    playback_position: Arc<RwLock<Option<u32>>>,
    playback_queue: Arc<RwLock<PlaybackQueue>>,
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
                    *playback_queue.write().await =
                        PlaybackQueue::default();
                    let track = SpotifyUri::Track { id };
                    player.load(track, true, 0);
                    format!("OK loading {arg}\n")
                }
                Err(e) => format!("ERR bad track id '{arg}': {e}\n"),
            },
            "LOAD_LIKED" => match liked_track_ids(&session).await {
                Ok(track_ids) => {
                    match track_ids.iter().position(|id| id == arg) {
                        Some(index) => match SpotifyId::from_base62(arg) {
                            Ok(id) => {
                                let queue_len = track_ids.len();
                                *playback_queue.write().await =
                                    PlaybackQueue {
                                        source: Some(QueueSource::Liked),
                                        track_ids,
                                        current_index: Some(index),
                                    };
                                player.load(
                                    SpotifyUri::Track { id },
                                    true,
                                    0,
                                );
                                eprintln!(
                                    "[spotui] liked queue loaded -> {}/{} ({})",
                                    index + 1,
                                    queue_len,
                                    arg
                                );
                                format!("OK loading liked {arg}\n")
                            }
                            Err(e) => format!(
                                "ERR bad track id '{arg}': {e}\n"
                            ),
                        },
                        None => format!(
                            "ERR track '{arg}' is not in Liked Songs\n"
                        ),
                    }
                }
                Err(e) => format!("ERR liked queue fetch failed: {e}\n"),
            },
            "LOAD_PLAYLIST" => {
                let mut args = arg.split_whitespace();
                let playlist_id = args.next().unwrap_or("");
                let track_id = args.next().unwrap_or("");
                let has_extra_args = args.next().is_some();

                if playlist_id.is_empty()
                    || track_id.is_empty()
                    || has_extra_args
                {
                    concat!(
                        "ERR LOAD_PLAYLIST needs playlist_id ",
                        "and track_id\n"
                    )
                    .to_string()
                } else {
                    match playlist_track_ids(
                        &session,
                        playlist_id,
                    )
                    .await
                    {
                        Ok(track_ids) => {
                            match track_ids
                                .iter()
                                .position(|id| id == track_id)
                            {
                                Some(index) => {
                                    match SpotifyId::from_base62(
                                        track_id,
                                    ) {
                                        Ok(id) => {
                                            let queue_len =
                                                track_ids.len();
                                            *playback_queue.write().await =
                                                PlaybackQueue {
                                                    source: Some(
                                                        QueueSource::Playlist(
                                                            playlist_id
                                                                .to_string(),
                                                        ),
                                                    ),
                                                    track_ids,
                                                    current_index:
                                                        Some(index),
                                                };
                                            player.load(
                                                SpotifyUri::Track {
                                                    id,
                                                },
                                                true,
                                                0,
                                            );
                                            eprintln!(
                                                "[spotui] playlist {} queue loaded -> {}/{} ({})",
                                                playlist_id,
                                                index + 1,
                                                queue_len,
                                                track_id
                                            );
                                            format!(
                                                "OK loading playlist {} {}\n",
                                                playlist_id,
                                                track_id
                                            )
                                        }
                                        Err(e) => format!(
                                            "ERR bad track id '{}': {}\n",
                                            track_id,
                                            e
                                        ),
                                    }
                                }
                                None => format!(
                                    "ERR track '{}' is not in playlist '{}'\n",
                                    track_id,
                                    playlist_id
                                ),
                            }
                        }
                        Err(e) => format!(
                            "ERR playlist queue fetch failed: {}\n",
                            e
                        ),
                    }
                }
            }
            "PLAY" => {
                player.play();
                "OK play\n".to_string()
            }
            "PAUSE" => {
                player.pause();
                "OK pause\n".to_string()
            }
            "STOP" => {
                *playback_queue.write().await =
                    PlaybackQueue::default();
                player.stop();
                "OK stop\n".to_string()
            }
            "STATUS" => {
                let state = *playback_state.read().await;
                format!("STATUS {state}\n")
            }
            "QUEUE_STATUS" => {
                let queue = playback_queue.read().await;

                match (
                    queue.source.as_ref(),
                    queue.current_index.and_then(|index| {
                        queue
                            .track_ids
                            .get(index)
                            .map(|track_id| (index, track_id))
                    }),
                ) {
                    (
                        Some(QueueSource::Liked),
                        Some((index, track_id)),
                    ) => format!(
                        "QUEUE LIKED {} {} {}\n",
                        index,
                        queue.track_ids.len(),
                        track_id
                    ),
                    (
                        Some(QueueSource::Playlist(playlist_id)),
                        Some((index, track_id)),
                    ) => format!(
                        "QUEUE PLAYLIST {} {} {} {}\n",
                        playlist_id,
                        index,
                        queue.track_ids.len(),
                        track_id
                    ),
                    _ => "QUEUE NONE\n".to_string(),
                }
            }
            "NOW_PLAYING" => {
                let current = now_playing.read().await;
                match current.as_ref() {
                    Some(item) => format!(
                        "NOW_PLAYING {}\t{}\t{}\t{}\n",
                        item.id, item.title, item.artist, item.duration_ms
                    ),
                    None => "NOW_PLAYING NONE\n".to_string(),
                }
            }
            "POSITION" => {
                let position = *playback_position.read().await;
                match position {
                    Some(position) => format!("POSITION {position}\n"),
                    None => "POSITION NONE\n".to_string(),
                }
            }
            "SEEK" => match arg.parse::<u32>() {
                Ok(requested_ms) => {
                    let duration_ms = now_playing
                        .read()
                        .await
                        .as_ref()
                        .map(|item| item.duration_ms);

                    match duration_ms {
                        Some(duration_ms) => {
                            let target_ms = requested_ms.min(duration_ms);
                            player.seek(target_ms);
                            format!("OK seek {target_ms}\n")
                        }
                        None => "ERR no track loaded\n".to_string(),
                    }
                }
                Err(_) => "ERR SEEK needs milliseconds\n".to_string(),
            },
            "SEARCH" => search_tracks(&session, arg).await,
            "LIKED" => liked_tracks(&session).await,
            "PLAYLIST" => playlist_tracks(&session, arg).await,
            "PLAYLISTS" => public_playlists(&session).await,
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

/// Fetch playlists exposed through the current users Spotify profile.
///
/// This endpoint includes public and followed profile playlists. It does not
/// expose private playlists or rootlist folder organization.
async fn public_playlists(session: &Session) -> String {
    const MAX_PLAYLISTS: usize = 100;

    let username = session.username();

    let response = match session
        .spclient()
        .get_user_profile(
            &username,
            Some(MAX_PLAYLISTS as u32),
            Some(0),
        )
        .await
    {
        Ok(bytes) => bytes,
        Err(e) => {
            return format!(
                "ERR playlist profile fetch failed: {e}\n"
            );
        }
    };

    let profile: serde_json::Value =
        match serde_json::from_slice(response.as_ref()) {
            Ok(value) => value,
            Err(e) => {
                return format!(
                    "ERR playlist profile parse failed: {e}\n"
                );
            }
        };

    let Some(playlists) = profile
        .get("public_playlists")
        .and_then(serde_json::Value::as_array)
    else {
        return concat!(
            "ERR profile has no public_playlists field",
            "\n"
        )
        .to_string();
    };

    let mut reply = String::new();
    let mut count = 0usize;

    for playlist in playlists.iter().take(MAX_PLAYLISTS) {
        let Some(uri) = playlist
            .get("uri")
            .and_then(serde_json::Value::as_str)
        else {
            continue;
        };

        let Some(id) = uri.strip_prefix("spotify:playlist:") else {
            continue;
        };

        let name = playlist
            .get("name")
            .and_then(serde_json::Value::as_str)
            .unwrap_or("Untitled Playlist");

        let owner = playlist
            .get("owner_name")
            .and_then(serde_json::Value::as_str)
            .unwrap_or("");

        reply.push_str("PLAYLIST ");
        reply.push_str(&sanitize_field(id));
        reply.push_str("\t");
        reply.push_str(&sanitize_field(name));
        reply.push_str("\t");
        reply.push_str(&sanitize_field(owner));
        reply.push_str("\n");

        count += 1;
    }

    reply.push_str(&format!("END {count}\n"));
    reply
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

/// Fetch the same ordered Liked Songs IDs exposed by `LIKED`.
async fn liked_track_ids(session: &Session) -> Result<Vec<String>, String> {
    const MAX_RESULTS: usize = 50;

    let user = session.username();
    let uri = format!("spotify:user:{user}:collection");
    let context = session
        .spclient()
        .get_context(&uri)
        .await
        .map_err(|e| format!("context failed: {e}"))?;

    let mut track_ids = Vec::new();

    for page in &context.pages {
        for track in &page.tracks {
            let Some(uri) = track.uri.as_ref() else {
                continue;
            };
            let Some(id) = uri.strip_prefix("spotify:track:") else {
                continue;
            };

            track_ids.push(id.to_string());

            if track_ids.len() >= MAX_RESULTS {
                break;
            }
        }

        if !track_ids.is_empty() {
            break;
        }
    }

    Ok(track_ids)
}

fn spotify_track_base62(uri: &SpotifyUri) -> Option<String> {
    match uri {
        SpotifyUri::Track { id } => id.to_base62().ok(),
        _ => None,
    }
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
    let track_ids = match playlist_track_ids(session, playlist_arg).await {
        Ok(track_ids) => track_ids,
        Err(e) => return format!("ERR {e}\n"),
    };

    let track_uris = track_ids
        .iter()
        .map(|id| format!("spotify:track:{id}"))
        .collect::<Vec<_>>();

    enrich_track_uris(session, &track_uris).await
}

/// Fetch the same ordered track IDs exposed by `PLAYLIST`.
async fn playlist_track_ids(
    session: &Session,
    playlist_arg: &str,
) -> Result<Vec<String>, String> {
    if playlist_arg.is_empty() {
        return Err("empty playlist id".to_string());
    }

    const MAX_RESULTS: usize = 50;

    let uri_str = if playlist_arg.starts_with("spotify:") {
        playlist_arg.to_string()
    } else {
        format!("spotify:playlist:{playlist_arg}")
    };

    let uri = SpotifyUri::from_uri(&uri_str)
        .map_err(|e| format!("bad playlist uri '{uri_str}': {e}"))?;

    let playlist = Playlist::get(session, &uri)
        .await
        .map_err(|e| format!("playlist fetch failed: {e}"))?;

    let mut track_ids = Vec::new();

    for track in playlist.tracks() {
        if let SpotifyUri::Track { id } = track {
            if let Ok(base62) = id.to_base62() {
                if !base62.is_empty() {
                    track_ids.push(base62);
                }
            }
        }

        if track_ids.len() >= MAX_RESULTS {
            break;
        }
    }

    Ok(track_ids)
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
