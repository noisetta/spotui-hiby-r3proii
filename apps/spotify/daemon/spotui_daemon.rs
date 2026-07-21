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
//   LOAD_LIKED <request_id> <track_id>
//                            -> load within the Liked Songs queue
//   LOAD_PLAYLIST <request_id> <playlist_id> <track_id>
//                            -> load within a playlist queue
//   LOAD_SEARCH <request_id> <track_id>
//                            -> load within the latest search queue
//   PLAY                     -> resume
//   PAUSE                    -> pause
//   STOP                     -> stop
//   NEXT                     -> manually advance in the active queue
//   PREVIOUS                 -> manually go back in the active queue
//   STATUS                   -> report current playback state
//   QUEUE_STATUS             -> report the active queue position
//   QUEUE_PAGE <offset> <limit>
//                            -> page through the active play order
//   PLAYBACK_MODES           -> report shuffle and repeat modes
//   SET_SHUFFLE <ON|OFF>     -> persist and apply shuffle mode
//   SET_REPEAT <OFF|ALL|ONE> -> persist and apply repeat mode
//   NOW_PLAYING              -> report current track metadata
//   POSITION                 -> report playback position in milliseconds
//   SEEK <milliseconds>      -> seek within the loaded track
//   QUIT                     -> shut the daemon down
//
// Audio uses librespot's subprocess backend. Each playback sink owns an aplay
// process so loading, pause, and track replacement cannot reuse an ALSA stream
// that has already entered underrun recovery.

use std::{
    collections::HashMap,
    process::exit,
    sync::Arc,
    time::Duration,
};

use tokio::{
    io::{AsyncBufReadExt, AsyncWriteExt, BufReader},
    net::{TcpListener, UnixListener},
    sync::{Mutex, RwLock},
};

use librespot::{
    audio::AudioFetchParams,
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
const PLAYBACK_MODES_PATH: &str = "/usr/data/spotui_playback_modes";
const APLAY_COMMAND: &str =
    "aplay -D hw:0,0 -f S16_LE -r 44100 -c 2 -B 1000000 -F 125000";

struct NowPlaying {
    id: String,
    title: String,
    artist: String,
    duration_ms: u32,
}

enum QueueSource {
    Liked,
    Playlist(String),
    Search,
}

struct PlaybackQueue {
    source: Option<QueueSource>,
    track_ids: Vec<String>,
    current_index: Option<usize>,
    play_order: Vec<usize>,
    order_position: Option<usize>,
    request_id: u64,
}

impl Default for PlaybackQueue {
    fn default() -> Self {
        Self {
            source: None,
            track_ids: Vec::new(),
            current_index: None,
            play_order: Vec::new(),
            order_position: None,
            request_id: 0,
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum RepeatMode {
    Off,
    All,
    One,
}

impl RepeatMode {
    fn key(self) -> &'static str {
        match self {
            Self::Off => "off",
            Self::All => "all",
            Self::One => "one",
        }
    }
}

struct PlaybackModes {
    shuffle: bool,
    repeat: RepeatMode,
    shuffle_seed: u64,
}

fn load_playback_modes() -> PlaybackModes {
    let contents = std::fs::read_to_string(PLAYBACK_MODES_PATH)
        .unwrap_or_default();
    let shuffle = contents.lines().any(|line| line == "shuffle=on");
    let repeat = if contents.lines().any(|line| line == "repeat=all") {
        RepeatMode::All
    } else if contents.lines().any(|line| line == "repeat=one") {
        RepeatMode::One
    } else {
        RepeatMode::Off
    };

    PlaybackModes {
        shuffle,
        repeat,
        shuffle_seed: 0x5f37_59df_c2b1_4a6d,
    }
}

fn save_playback_modes(modes: &PlaybackModes) -> std::io::Result<()> {
    std::fs::write(
        PLAYBACK_MODES_PATH,
        format!(
            "shuffle={}\nrepeat={}\n",
            if modes.shuffle { "on" } else { "off" },
            modes.repeat.key()
        ),
    )
}

fn next_random(seed: &mut u64) -> u64 {
    *seed ^= *seed << 13;
    *seed ^= *seed >> 7;
    *seed ^= *seed << 17;
    *seed
}

fn make_play_order(
    len: usize,
    selected_index: usize,
    shuffle: bool,
    seed: &mut u64,
) -> Vec<usize> {
    if !shuffle {
        return (0..len).collect();
    }

    let mut order: Vec<usize> = (0..len)
        .filter(|index| *index != selected_index)
        .collect();

    for index in (1..order.len()).rev() {
        let swap_index = (next_random(seed) as usize) % (index + 1);
        order.swap(index, swap_index);
    }

    order.insert(0, selected_index);
    order
}

#[derive(Default)]
struct BrowseCache {
    liked_track_ids: Vec<String>,
    playlists: HashMap<String, Vec<String>>,
    search_track_ids: Vec<String>,
}

async fn register_load_request(
    latest_load_request: &RwLock<u64>,
    request_id: u64,
) -> bool {
    let mut latest = latest_load_request.write().await;

    if request_id < *latest {
        false
    } else {
        *latest = request_id;
        true
    }
}

async fn load_request_is_current(
    latest_load_request: &RwLock<u64>,
    request_id: u64,
) -> bool {
    *latest_load_request.read().await == request_id
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
    player_config.gapless = false;
    let audio_format = AudioFormat::default();

    // The device's WiFi stream can pause for longer than librespot's default
    // one-second startup read-ahead. Wait for two seconds of compressed audio
    // before decoding starts so the pipe does not starve at track startup.
    let mut audio_fetch_params = AudioFetchParams::default();
    audio_fetch_params.read_ahead_before_playback = Duration::from_secs(2);
    if AudioFetchParams::set(audio_fetch_params).is_err() {
        eprintln!("[spotui] audio fetch parameters were already initialized");
        exit(1);
    }

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
    // Start a fresh aplay process with each sink lifecycle. Keeping one aplay
    // process open across loading gaps leaves ALSA in an underrun/recovery
    // cycle that is audible at the beginning of the next track.
    let backend = audio_backend::find(Some("subprocess".to_string()))
        .expect("subprocess audio backend unavailable");
    let aplay_command = APLAY_COMMAND.to_string();

    // Software volume mixer (softvol). librespot attenuates the PCM by the
    // mixer's current volume before it reaches the subprocess/aplay. We keep
    // an Arc
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
        move || backend(Some(aplay_command.clone()), audio_format),
    );

    // Track actual player state and current metadata from librespot events.
    let playback_state = Arc::new(RwLock::new("STOPPED"));
    let now_playing = Arc::new(RwLock::new(None::<NowPlaying>));
    let playback_position = Arc::new(RwLock::new(None::<u32>));
    let playback_queue =
        Arc::new(RwLock::new(PlaybackQueue::default()));
    let playback_modes = Arc::new(RwLock::new(load_playback_modes()));
    let browse_cache =
        Arc::new(RwLock::new(BrowseCache::default()));
    let load_mutex = Arc::new(Mutex::new(()));
    let latest_load_request = Arc::new(RwLock::new(0u64));
    let event_player = player.clone();
    let event_state = playback_state.clone();
    let event_now_playing = now_playing.clone();
    let event_position = playback_position.clone();
    let event_playback_queue = playback_queue.clone();
    let event_playback_modes = playback_modes.clone();
    let mut player_events = player.get_player_event_channel();

    tokio::spawn(async move {
        let mut consecutive_unavailable: u8 = 0;

        while let Some(event) = player_events.recv().await {
            if let PlayerEvent::EndOfTrack { track_id, .. } = &event {
                let ended_id = spotify_track_base62(track_id);
                let mut next_track = None;
                let repeat_mode = event_playback_modes.read().await.repeat;

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
                            Some(QueueSource::Search) => {
                                "search".to_string()
                            }
                            None => "unknown".to_string(),
                        };

                        let current_matches = queue
                            .track_ids
                            .get(current_index)
                            .map(String::as_str)
                            == Some(ended_id);

                        if current_matches {
                            let next_order_position = if repeat_mode
                                == RepeatMode::One
                            {
                                queue.order_position
                            } else {
                                queue.order_position.and_then(|position| {
                                    let next = position + 1;
                                    if next < queue.play_order.len() {
                                        Some(next)
                                    } else if repeat_mode == RepeatMode::All
                                        && !queue.play_order.is_empty()
                                    {
                                        Some(0)
                                    } else {
                                        None
                                    }
                                })
                            };
                            let next_index = next_order_position.and_then(
                                |position| queue.play_order.get(position).copied(),
                            );
                            let next_id = next_index.and_then(|index| {
                                queue.track_ids.get(index).cloned()
                            });

                            if let (Some(next_id), Some(next_index), Some(next_order_position)) =
                                (next_id, next_index, next_order_position)
                            {
                                let queue_len = queue.track_ids.len();
                                queue.current_index = Some(next_index);
                                queue.order_position =
                                    Some(next_order_position);
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

            if matches!(&event, PlayerEvent::Playing { .. }) {
                if consecutive_unavailable > 0 {
                    eprintln!(
                        "[spotui] playback recovered; unavailable counter reset"
                    );
                }
                consecutive_unavailable = 0;
            }

            if matches!(&event, PlayerEvent::Unavailable { .. }) {
                consecutive_unavailable =
                    consecutive_unavailable.saturating_add(1);

                if consecutive_unavailable >= 2 {
                    *event_state.write().await = "ERROR";
                    eprintln!(
                        "[spotui] repeated unavailable tracks without successful playback; exiting for supervised recovery"
                    );
                    exit(75);
                }

                let retry_track = {
                    let queue = event_playback_queue.read().await;
                    queue.current_index.and_then(|index| {
                        queue.track_ids.get(index).cloned()
                    })
                };

                if let Some(retry_track) = retry_track {
                    match SpotifyId::from_base62(&retry_track) {
                        Ok(id) => {
                            *event_state.write().await = "LOADING";
                            eprintln!(
                                "[spotui] unavailable track 1/2; preserving queue and retrying {}",
                                retry_track
                            );
                            tokio::time::sleep(Duration::from_secs(2)).await;
                            event_player.load(
                                SpotifyUri::Track { id },
                                true,
                                0,
                            );
                            continue;
                        }
                        Err(error) => {
                            eprintln!(
                                "[spotui] unavailable retry id invalid '{}': {}",
                                retry_track,
                                error
                            );
                        }
                    }
                }

                *event_playback_queue.write().await = PlaybackQueue::default();
                event_player.stop();
                eprintln!(
                    "[spotui] unavailable track 1/2 without retry target; cleared queue"
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
    let unix_playback_modes = playback_modes.clone();
    let unix_browse_cache = browse_cache.clone();
    let unix_load_mutex = load_mutex.clone();
    let unix_latest_load_request = latest_load_request.clone();
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
                    let playback_modes = unix_playback_modes.clone();
                    let browse_cache = unix_browse_cache.clone();
                    let load_mutex = unix_load_mutex.clone();
                    let latest_load_request =
                        unix_latest_load_request.clone();
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
                            playback_modes,
                            browse_cache,
                            load_mutex,
                            latest_load_request,
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
    let tcp_playback_modes = playback_modes.clone();
    let tcp_browse_cache = browse_cache.clone();
    let tcp_load_mutex = load_mutex.clone();
    let tcp_latest_load_request = latest_load_request.clone();
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
                    let playback_modes = tcp_playback_modes.clone();
                    let browse_cache = tcp_browse_cache.clone();
                    let load_mutex = tcp_load_mutex.clone();
                    let latest_load_request =
                        tcp_latest_load_request.clone();
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
                            playback_modes,
                            browse_cache,
                            load_mutex,
                            latest_load_request,
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
    playback_modes: Arc<RwLock<PlaybackModes>>,
    browse_cache: Arc<RwLock<BrowseCache>>,
    load_mutex: Arc<Mutex<()>>,
    latest_load_request: Arc<RwLock<u64>>,
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
            "LOAD_LIKED" => {
                let mut args = arg.split_whitespace();
                let request_id = args.next().and_then(|value| {
                    value.parse::<u64>().ok()
                });
                let track_id = args.next().unwrap_or("");
                let has_extra_args = args.next().is_some();

                if request_id.is_none()
                    || track_id.is_empty()
                    || has_extra_args
                {
                    "ERR LOAD_LIKED needs request_id and track_id\n"
                        .to_string()
                } else {
                    let request_id = request_id.unwrap();
                    eprintln!(
                        "[spotui] queued liked load request {} ({})",
                        request_id,
                        track_id
                    );

                    if !register_load_request(
                        &latest_load_request,
                        request_id,
                    )
                    .await
                    {
                        eprintln!(
                            "[spotui] ignored stale liked load request {}",
                            request_id
                        );
                        format!("OK ignored stale request {request_id}\n")
                    } else {
                        let _load_guard = load_mutex.lock().await;

                        if !load_request_is_current(
                            &latest_load_request,
                            request_id,
                        )
                        .await
                        {
                            eprintln!(
                                "[spotui] skipped superseded liked load request {}",
                                request_id
                            );
                            format!("OK skipped superseded request {request_id}\n")
                        } else {
                            let loaded = liked_track_ids_for_load(
                                &session,
                                &browse_cache,
                                track_id,
                            )
                            .await;

                            if !load_request_is_current(
                                &latest_load_request,
                                request_id,
                            )
                            .await
                            {
                                eprintln!(
                                    "[spotui] skipped superseded liked load request {} after fetch",
                                    request_id
                                );
                                format!("OK skipped superseded request {request_id}\n")
                            } else {
                                match loaded {
                    Ok((track_ids, cache_hit)) => {
                        match track_ids.iter().position(|id| id == track_id) {
                            Some(index) => {
                                match SpotifyId::from_base62(track_id) {
                                    Ok(id) => {
                                        let queue_len =
                                            track_ids.len();
                                        let play_order = {
                                            let mut modes =
                                                playback_modes.write().await;
                                            let shuffle = modes.shuffle;
                                            make_play_order(
                                                queue_len,
                                                index,
                                                shuffle,
                                                &mut modes.shuffle_seed,
                                            )
                                        };
                                        let order_position = play_order
                                            .iter()
                                            .position(|queued| *queued == index)
                                            .unwrap_or(0);
                                        *playback_queue.write().await =
                                            PlaybackQueue {
                                                source: Some(
                                                    QueueSource::Liked,
                                                ),
                                                track_ids,
                                                current_index:
                                                    Some(index),
                                                play_order,
                                                order_position:
                                                    Some(order_position),
                                                request_id,
                                            };
                                        player.load(
                                            SpotifyUri::Track { id },
                                            true,
                                            0,
                                        );
                                        eprintln!(
                                            "[spotui] liked request {} loaded cache={} -> {}/{} ({})",
                                            request_id,
                                            if cache_hit {
                                                "hit"
                                            } else {
                                                "miss"
                                            },
                                            index + 1,
                                            queue_len,
                                            track_id
                                        );
                                        format!(
                                            "OK loading liked {request_id} {track_id}\n"
                                        )
                                    }
                                    Err(e) => format!(
                                        "ERR bad track id '{track_id}': {e}\n"
                                    ),
                                }
                            }
                            None => format!(
                                "ERR track '{track_id}' is not in Liked Songs\n"
                            ),
                        }
                    }
                    Err(e) => {
                        format!("ERR liked queue fetch failed: {e}\n")
                    }
                                }
                            }
                        }
                    }
                }
            }
            "LOAD_PLAYLIST" => {
                let mut args = arg.split_whitespace();
                let request_id = args.next().and_then(|value| {
                    value.parse::<u64>().ok()
                });
                let playlist_id = args.next().unwrap_or("");
                let track_id = args.next().unwrap_or("");
                let has_extra_args = args.next().is_some();

                if request_id.is_none()
                    || playlist_id.is_empty()
                    || track_id.is_empty()
                    || has_extra_args
                {
                    concat!(
                        "ERR LOAD_PLAYLIST needs request_id, ",
                        "playlist_id, and track_id\n"
                    )
                    .to_string()
                } else {
                    let request_id = request_id.unwrap();
                    eprintln!(
                        "[spotui] queued playlist load request {} ({})",
                        request_id,
                        track_id
                    );

                    if !register_load_request(
                        &latest_load_request,
                        request_id,
                    )
                    .await
                    {
                        eprintln!(
                            "[spotui] ignored stale playlist load request {}",
                            request_id
                        );
                        format!("OK ignored stale request {request_id}\n")
                    } else {
                    let _load_guard = load_mutex.lock().await;

                    if !load_request_is_current(
                        &latest_load_request,
                        request_id,
                    )
                    .await
                    {
                        eprintln!(
                            "[spotui] skipped superseded playlist load request {}",
                            request_id
                        );
                        format!("OK skipped superseded request {request_id}\n")
                    } else {

                    let loaded = playlist_track_ids_for_load(
                        &session,
                        &browse_cache,
                        playlist_id,
                        track_id,
                    )
                    .await;

                    if !load_request_is_current(
                        &latest_load_request,
                        request_id,
                    )
                    .await
                    {
                        eprintln!(
                            "[spotui] skipped superseded playlist load request {} after fetch",
                            request_id
                        );
                        format!("OK skipped superseded request {request_id}\n")
                    } else {
                    match loaded {
                        Ok((track_ids, cache_hit)) => {
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
                                            let play_order = {
                                                let mut modes = playback_modes
                                                    .write()
                                                    .await;
                                                let shuffle = modes.shuffle;
                                                make_play_order(
                                                    queue_len,
                                                    index,
                                                    shuffle,
                                                    &mut modes.shuffle_seed,
                                                )
                                            };
                                            let order_position = play_order
                                                .iter()
                                                .position(|queued| *queued == index)
                                                .unwrap_or(0);
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
                                                    play_order,
                                                    order_position:
                                                        Some(order_position),
                                                    request_id,
                                                };
                                            player.load(
                                                SpotifyUri::Track {
                                                    id,
                                                },
                                                true,
                                                0,
                                            );
                                            eprintln!(
                                                "[spotui] playlist {} request {} loaded cache={} -> {}/{} ({})",
                                                playlist_id,
                                                request_id,
                                                if cache_hit {
                                                    "hit"
                                                } else {
                                                    "miss"
                                                },
                                                index + 1,
                                                queue_len,
                                                track_id
                                            );
                                            format!(
                                                "OK loading playlist {} {} {}\n",
                                                request_id,
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
                    }
                }
            }
            "LOAD_SEARCH" => {
                let mut args = arg.split_whitespace();
                let request_id = args.next().and_then(|value| {
                    value.parse::<u64>().ok()
                });
                let track_id = args.next().unwrap_or("");
                let has_extra_args = args.next().is_some();

                if request_id.is_none()
                    || track_id.is_empty()
                    || has_extra_args
                {
                    "ERR LOAD_SEARCH needs request_id and track_id\n"
                        .to_string()
                } else {
                    let request_id = request_id.unwrap();
                    eprintln!(
                        "[spotui] queued search load request {} ({})",
                        request_id,
                        track_id
                    );

                    if !register_load_request(
                        &latest_load_request,
                        request_id,
                    )
                    .await
                    {
                        eprintln!(
                            "[spotui] ignored stale search load request {}",
                            request_id
                        );
                        format!("OK ignored stale request {request_id}\n")
                    } else {
                        let _load_guard = load_mutex.lock().await;

                        if !load_request_is_current(
                            &latest_load_request,
                            request_id,
                        )
                        .await
                        {
                            eprintln!(
                                "[spotui] skipped superseded search load request {}",
                                request_id
                            );
                            format!("OK skipped superseded request {request_id}\n")
                        } else {
                            let track_ids = browse_cache
                                .read()
                                .await
                                .search_track_ids
                                .clone();

                            match track_ids
                                .iter()
                                .position(|id| id == track_id)
                            {
                                Some(index) => {
                                    match SpotifyId::from_base62(track_id) {
                                        Ok(id) => {
                                            let queue_len = track_ids.len();
                                            let play_order = {
                                                let mut modes = playback_modes
                                                    .write()
                                                    .await;
                                                let shuffle = modes.shuffle;
                                                make_play_order(
                                                    queue_len,
                                                    index,
                                                    shuffle,
                                                    &mut modes.shuffle_seed,
                                                )
                                            };
                                            let order_position = play_order
                                                .iter()
                                                .position(|queued| *queued == index)
                                                .unwrap_or(0);
                                            *playback_queue.write().await =
                                                PlaybackQueue {
                                                    source: Some(
                                                        QueueSource::Search,
                                                    ),
                                                    track_ids,
                                                    current_index:
                                                        Some(index),
                                                    play_order,
                                                    order_position:
                                                        Some(order_position),
                                                    request_id,
                                                };
                                            player.load(
                                                SpotifyUri::Track { id },
                                                true,
                                                0,
                                            );
                                            eprintln!(
                                                "[spotui] search request {} loaded -> {}/{} ({})",
                                                request_id,
                                                index + 1,
                                                queue_len,
                                                track_id
                                            );
                                            format!(
                                                "OK loading search {request_id} {track_id}\n"
                                            )
                                        }
                                        Err(e) => format!(
                                            "ERR bad track id '{track_id}': {e}\n"
                                        ),
                                    }
                                }
                                None => format!(
                                    "ERR track '{track_id}' is not in current search results\n"
                                ),
                            }
                        }
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
            "NEXT" | "PREVIOUS" => {
                let forward = cmd == "NEXT";
                let repeat_mode = playback_modes.read().await.repeat;
                let mut queue = playback_queue.write().await;

                let target_position = queue.order_position.and_then(|position| {
                    if forward {
                        let next = position + 1;
                        if next < queue.play_order.len() {
                            Some(next)
                        } else if repeat_mode == RepeatMode::All
                            && !queue.play_order.is_empty()
                        {
                            Some(0)
                        } else {
                            None
                        }
                    } else if position > 0 {
                        Some(position - 1)
                    } else if repeat_mode == RepeatMode::All
                        && !queue.play_order.is_empty()
                    {
                        Some(queue.play_order.len() - 1)
                    } else {
                        None
                    }
                });

                match target_position.and_then(|position| {
                    queue.play_order.get(position).copied().map(|index| {
                        (position, index)
                    })
                }) {
                    Some((position, index)) => {
                        match queue.track_ids.get(index).cloned() {
                            Some(track_id) => {
                                match SpotifyId::from_base62(&track_id) {
                                    Ok(id) => {
                                        queue.current_index = Some(index);
                                        queue.order_position = Some(position);
                                        let queue_len = queue.track_ids.len();
                                        player.load(
                                            SpotifyUri::Track { id },
                                            true,
                                            0,
                                        );
                                        eprintln!(
                                            "[spotui] manual {} -> {}/{} ({})",
                                            if forward { "next" } else { "previous" },
                                            position + 1,
                                            queue_len,
                                            track_id
                                        );
                                        format!(
                                            "OK loading {} {} {}\n",
                                            if forward { "next" } else { "previous" },
                                            index,
                                            track_id
                                        )
                                    }
                                    Err(error) => format!(
                                        "ERR bad queued track id '{}': {}\n",
                                        track_id, error
                                    ),
                                }
                            }
                            None => "ERR queue index unavailable\n".to_string(),
                        }
                    }
                    None => "OK queue boundary\n".to_string(),
                }
            }
            "PLAYBACK_MODES" => {
                let modes = playback_modes.read().await;
                format!(
                    "MODES SHUFFLE {} REPEAT {}\n",
                    if modes.shuffle { "ON" } else { "OFF" },
                    modes.repeat.key().to_uppercase()
                )
            }
            "SET_SHUFFLE" => {
                let enabled = match arg.to_uppercase().as_str() {
                    "ON" => Some(true),
                    "OFF" => Some(false),
                    _ => None,
                };

                match enabled {
                    Some(enabled) => {
                        let mut modes = playback_modes.write().await;
                        modes.shuffle = enabled;

                        let mut queue = playback_queue.write().await;
                        if let Some(current_index) = queue.current_index {
                            queue.play_order = make_play_order(
                                queue.track_ids.len(),
                                current_index,
                                enabled,
                                &mut modes.shuffle_seed,
                            );
                            queue.order_position = queue
                                .play_order
                                .iter()
                                .position(|queued| *queued == current_index);
                        }

                        if let Err(error) = save_playback_modes(&modes) {
                            eprintln!(
                                "[spotui] failed to save playback modes: {}",
                                error
                            );
                        }
                        format!(
                            "OK shuffle {}\n",
                            if enabled { "on" } else { "off" }
                        )
                    }
                    None => "ERR SET_SHUFFLE needs ON or OFF\n".to_string(),
                }
            }
            "SET_REPEAT" => {
                let updated = match arg.to_uppercase().as_str() {
                    "OFF" => Some(RepeatMode::Off),
                    "ALL" => Some(RepeatMode::All),
                    "ONE" => Some(RepeatMode::One),
                    _ => None,
                };

                match updated {
                    Some(updated) => {
                        let mut modes = playback_modes.write().await;
                        modes.repeat = updated;
                        if let Err(error) = save_playback_modes(&modes) {
                            eprintln!(
                                "[spotui] failed to save playback modes: {}",
                                error
                            );
                        }
                        format!("OK repeat {}\n", updated.key())
                    }
                    None => {
                        "ERR SET_REPEAT needs OFF, ALL, or ONE\n".to_string()
                    }
                }
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
                        "QUEUE LIKED {} {} {} {}\n",
                        queue.request_id,
                        index,
                        queue.track_ids.len(),
                        track_id
                    ),
                    (
                        Some(QueueSource::Playlist(playlist_id)),
                        Some((index, track_id)),
                    ) => format!(
                        "QUEUE PLAYLIST {} {} {} {} {}\n",
                        queue.request_id,
                        playlist_id,
                        index,
                        queue.track_ids.len(),
                        track_id
                    ),
                    (
                        Some(QueueSource::Search),
                        Some((index, track_id)),
                    ) => format!(
                        "QUEUE SEARCH {} {} {} {}\n",
                        queue.request_id,
                        index,
                        queue.track_ids.len(),
                        track_id
                    ),
                    _ => "QUEUE NONE\n".to_string(),
                }
            }
            "QUEUE_PAGE" => {
                let mut args = arg.split_whitespace();
                let offset = args.next().and_then(|value| {
                    value.parse::<usize>().ok()
                });
                let limit = args.next().and_then(|value| {
                    value.parse::<usize>().ok()
                });
                let has_extra_args = args.next().is_some();

                if offset.is_none()
                    || limit.is_none()
                    || has_extra_args
                    || limit == Some(0)
                    || limit.unwrap_or(0) > 9
                {
                    "ERR QUEUE_PAGE needs offset and limit 1-9\n"
                        .to_string()
                } else {
                    let offset = offset.unwrap();
                    let limit = limit.unwrap();
                    let queue = playback_queue.read().await;

                    match (
                        queue.source.as_ref(),
                        queue.order_position,
                    ) {
                        (Some(source), Some(current_position))
                            if !queue.play_order.is_empty() =>
                        {
                            let total = queue.play_order.len();
                            let start = offset.min(total);
                            let end = (start + limit).min(total);
                            let source_label = match source {
                                QueueSource::Liked => "LIKED".to_string(),
                                QueueSource::Playlist(playlist_id) => {
                                    format!("PLAYLIST:{}", playlist_id)
                                }
                                QueueSource::Search => "SEARCH".to_string(),
                            };
                            let mut response = format!(
                                "QUEUE_PAGE {} {} {} {} {} {}\n",
                                source_label,
                                queue.request_id,
                                current_position,
                                total,
                                start,
                                end - start
                            );

                            for position in start..end {
                                if let Some(source_index) =
                                    queue.play_order.get(position).copied()
                                {
                                    if let Some(track_id) =
                                        queue.track_ids.get(source_index)
                                    {
                                        response.push_str(&format!(
                                            "ITEM {} {} {}\n",
                                            position,
                                            source_index,
                                            track_id
                                        ));
                                    }
                                }
                            }

                            response
                        }
                        _ => "QUEUE_PAGE NONE\n".to_string(),
                    }
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
            "SEARCH" => {
                search_tracks(&session, arg, &browse_cache).await
            }
            "LIKED" => {
                liked_tracks(&session, &browse_cache).await
            }
            "PLAYLIST" => {
                playlist_tracks(&session, arg, &browse_cache).await
            }
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
async fn search_tracks(
    session: &Session,
    query: &str,
    browse_cache: &RwLock<BrowseCache>,
) -> String {
    if query.is_empty() {
        return "ERR empty query\n".to_string();
    }

    // Build the search URI: whitespace -> '+'
    let q = query.split_whitespace().collect::<Vec<_>>().join("+");
    let uri = format!("spotify:search:{q}");

    let track_ids = match context_track_ids(session, &uri).await {
        Ok(track_ids) => track_ids,
        Err(e) => return format!("ERR {e}\n"),
    };

    browse_cache.write().await.search_track_ids =
        track_ids.clone();

    let track_uris = track_ids
        .iter()
        .map(|id| format!("spotify:track:{id}"))
        .collect::<Vec<_>>();

    enrich_track_uris(session, &track_uris).await
}

async fn context_track_ids(
    session: &Session,
    uri: &str,
) -> Result<Vec<String>, String> {
    const MAX_RESULTS: usize = 50;

    let context = session
        .spclient()
        .get_context(uri)
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

/// Fetch the current user's liked songs and cache their ordered IDs.
async fn liked_tracks(
    session: &Session,
    browse_cache: &RwLock<BrowseCache>,
) -> String {
    let track_ids = match liked_track_ids(session).await {
        Ok(track_ids) => track_ids,
        Err(e) => return format!("ERR liked fetch failed: {e}\n"),
    };

    browse_cache.write().await.liked_track_ids =
        track_ids.clone();

    let track_uris = track_ids
        .iter()
        .map(|id| format!("spotify:track:{id}"))
        .collect::<Vec<_>>();

    enrich_track_uris(session, &track_uris).await
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

/// Fetch a specific playlist's tracks. Accepts either a bare base62 id or a
/// full `spotify:playlist:<id>` URI.
async fn playlist_tracks(
    session: &Session,
    playlist_arg: &str,
    browse_cache: &RwLock<BrowseCache>,
) -> String {
    let track_ids = match playlist_track_ids(session, playlist_arg).await {
        Ok(track_ids) => track_ids,
        Err(e) => return format!("ERR {e}\n"),
    };

    browse_cache
        .write()
        .await
        .playlists
        .insert(
            playlist_cache_key(playlist_arg),
            track_ids.clone(),
        );

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

fn playlist_cache_key(playlist_arg: &str) -> String {
    playlist_arg
        .strip_prefix("spotify:playlist:")
        .unwrap_or(playlist_arg)
        .to_string()
}

async fn liked_track_ids_for_load(
    session: &Session,
    browse_cache: &RwLock<BrowseCache>,
    track_id: &str,
) -> Result<(Vec<String>, bool), String> {
    let cached = browse_cache.read().await.liked_track_ids.clone();

    if cached.iter().any(|id| id == track_id) {
        return Ok((cached, true));
    }

    let refreshed = liked_track_ids(session).await?;
    browse_cache.write().await.liked_track_ids =
        refreshed.clone();

    Ok((refreshed, false))
}

async fn playlist_track_ids_for_load(
    session: &Session,
    browse_cache: &RwLock<BrowseCache>,
    playlist_id: &str,
    track_id: &str,
) -> Result<(Vec<String>, bool), String> {
    let cache_key = playlist_cache_key(playlist_id);
    let cached = browse_cache
        .read()
        .await
        .playlists
        .get(&cache_key)
        .cloned()
        .unwrap_or_default();

    if cached.iter().any(|id| id == track_id) {
        return Ok((cached, true));
    }

    let refreshed =
        playlist_track_ids(session, playlist_id).await?;

    browse_cache
        .write()
        .await
        .playlists
        .insert(cache_key, refreshed.clone());

    Ok((refreshed, false))
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
