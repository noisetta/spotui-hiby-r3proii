// spotui-ui-poc: a throwaway proof-of-concept to test whether drawing a UI to
// the framebuffer + reading touch input + playing audio can coexist on the
// HiBy R3 Pro II's single weak core WITHOUT breaking audio.
//
// This is deliberately minimal and ugly. It proves feasibility, nothing more.
//
// What it does:
//   1. mmaps /dev/fb0 (480x720, RGB565)
//   2. draws a hardcoded list of items as text rows (embedded-graphics)
//   3. reads touch events from /dev/input/event1
//   4. highlights the tapped row
//   5. sends "LOAD <track_id>" to the running spotui daemon over TCP/unix,
//      so audio plays -- letting us watch for underruns while drawing.
//
// Run the spotui_daemon separately; it owns the per-playback aplay process.

use std::{
    fs::{File, OpenOptions},
    io::{Read, Write},
    os::unix::io::AsRawFd,
    os::unix::net::UnixStream,
};

use embedded_graphics::{
    mono_font::{ascii::FONT_10X20, ascii::FONT_9X15, ascii::FONT_9X15_BOLD, MonoTextStyle},
    pixelcolor::{Rgb565, WebColors},
    prelude::*,
    primitives::{Circle, Ellipse, PrimitiveStyle, Rectangle},
    text::{Baseline, Text},
};

// ---- Display constants (confirmed from the device) ----------------------
const FB_PATH: &str = "/dev/fb0";
const WIDTH: usize = 480;
const HEIGHT: usize = 720;
const BYTES_PER_PIXEL: usize = 2; // RGB565
const FB_FRAME_BYTES: usize = WIDTH * HEIGHT * BYTES_PER_PIXEL; // 691200

// ---- Input constants (decoded from event1 capture) ----------------------
const INPUT_PATH: &str = "/dev/input/event1";
// The physical power button is KEY_POWER on event0 ("md-gpio-keys").
const POWER_INPUT_PATH: &str = "/dev/input/event0";
const KEY_POWER: u16 = 116;
// Volume buttons live on event2 ("jz adc keyboard"). Decoded from capture:
// KEY_VOLUMEUP = 0x73, KEY_VOLUMEDOWN = 0x72, value 1 = press.
const VOL_INPUT_PATH: &str = "/dev/input/event2";
const KEY_VOLUMEUP: u16 = 0x73;
const KEY_VOLUMEDOWN: u16 = 0x72;
const EVENT_SIZE: usize = 16; // 32-bit input_event: 8 (time) + 2 + 2 + 4
const EV_KEY: u16 = 0x01;
const EV_ABS: u16 = 0x03;
const EV_SYN: u16 = 0x00;
const BTN_TOUCH: u16 = 0x014a;
const ABS_MT_POSITION_X: u16 = 0x35;
const ABS_MT_POSITION_Y: u16 = 0x36;
const SYN_REPORT: u16 = 0x00;
const VOLUME_POPUP_MS: u128 = 1800;
const AMBIENT_IDLE_MS: u128 = 2000;

// ---- Backlight ----------------------------------------------------------
const BL_BRIGHTNESS: &str = "/sys/class/backlight/backlight_pwm0/brightness";
const BL_POWER: &str = "/sys/class/backlight/backlight_pwm0/bl_power";

// ---- Panel power (framebuffer blank control) ----------------------------
const FB_BLANK_SYSFS: &str = "/sys/class/graphics/fb0/blank";
// FBIOBLANK ioctl request number (Linux framebuffer): 0x4611.
// FB_BLANK_UNBLANK = 0 (power panel on / sleep-out).
const FBIOBLANK: u64 = 0x4611;
// The fb is double-buffered (480x1440 virtual = two 480x720 frames). The
// visible frame is selected by the pan y-offset. We draw to frame 0 and pan
// there via the sysfs `pan` control so our frame is the one shown.
const FB_PAN_SYSFS: &str = "/sys/class/graphics/fb0/pan";

// ---- Daemon control socket ----------------------------------------------
const DAEMON_SOCK: &str = "/tmp/spotui.sock";

// ---- Output jack detection ----------------------------------------------
// Two physical outputs, selected via the 'Output Port Switch' ALSA control:
//   3.5mm single-ended -> port 2   (headset switch)
//   4.4mm balanced     -> port 3   (balance switch)
const SW_HEADSET: &str = "/sys/class/switch/headset/state";
const SW_BALANCE: &str = "/sys/class/switch/balance/state";
const PORT_35MM: u8 = 2;
const PORT_44MM: u8 = 3;

// ---- UI layout ----------------------------------------------------------
const ROW_HEIGHT: i32 = 60;

/// A simple framebuffer wrapper that implements embedded-graphics DrawTarget.
/// Holds an in-memory RGB565 buffer; flush() writes it to /dev/fb0.
struct Framebuffer {
    file: File,
    buf: Vec<u8>, // WIDTH*HEIGHT*2 bytes, RGB565 little-endian
}

impl Framebuffer {
    fn open() -> std::io::Result<Self> {
        let file = OpenOptions::new().read(true).write(true).open(FB_PATH)?;
        Ok(Self {
            file,
            buf: vec![0u8; FB_FRAME_BYTES],
        })
    }

    fn flush(&mut self) -> std::io::Result<()> {
        use std::io::Seek;

        self.file.seek(std::io::SeekFrom::Start(0))?;
        self.file.write_all(&self.buf)?;

        // Pan the display to frame 0 (the buffer we just wrote), in case the
        // previous owner (hiby_player) left the view panned to frame 1.
        write_sysfs(FB_PAN_SYSFS, "0,0");

        Ok(())
    }

    /// Cheap keep-alive: re-flush the existing buffer to the panel without
    /// re-rendering anything. Used to keep the panel lit between actual UI
    /// changes, at much lower CPU cost than a full redraw.
    fn keepalive(&mut self) {
        self.flush().ok();
    }

    /// DIAGNOSTIC: fill a solid color directly into the framebuffer device at a
    /// given frame (0 or 1), then pan to that frame. This avoids the normal in-memory buffer path.
    /// This isolates "do our writes reach the panel" from text-rendering logic.
    fn diag_fill_frame(&mut self, frame: usize, color: Rgb565) -> std::io::Result<()> {
        use std::io::Seek;
        let raw: u16 = color.into_storage();
        let lo = (raw & 0xff) as u8;
        let hi = (raw >> 8) as u8;
        // Build one full 480x720 frame of this color.
        let mut frame_buf = vec![0u8; FB_FRAME_BYTES];
        for px in frame_buf.chunks_exact_mut(2) {
            px[0] = lo;
            px[1] = hi;
        }
        // Write it at the chosen frame's byte offset.
        let offset = (frame * FB_FRAME_BYTES) as u64;
        self.file.seek(std::io::SeekFrom::Start(offset))?;
        self.file.write_all(&frame_buf)?;
        Ok(())
    }

    #[inline]
    fn set_pixel(&mut self, x: i32, y: i32, color: Rgb565) {
        if x < 0 || y < 0 || x >= WIDTH as i32 || y >= HEIGHT as i32 {
            return;
        }
        let idx = (y as usize * WIDTH + x as usize) * BYTES_PER_PIXEL;
        // RGB565 little-endian. into_storage() yields the u16 raw value.
        let raw: u16 = color.into_storage();
        self.buf[idx] = (raw & 0xff) as u8;
        self.buf[idx + 1] = (raw >> 8) as u8;
    }
}

impl Dimensions for Framebuffer {
    fn bounding_box(&self) -> Rectangle {
        Rectangle::new(Point::zero(), Size::new(WIDTH as u32, HEIGHT as u32))
    }
}

impl DrawTarget for Framebuffer {
    type Color = Rgb565;
    type Error = core::convert::Infallible;

    fn draw_iter<I>(&mut self, pixels: I) -> Result<(), Self::Error>
    where
        I: IntoIterator<Item = Pixel<Self::Color>>,
    {
        for Pixel(coord, color) in pixels.into_iter() {
            self.set_pixel(coord.x, coord.y, color);
        }
        Ok(())
    }
}

/// Write a value to a sysfs file (backlight control).
fn write_sysfs(path: &str, val: &str) {
    if let Ok(mut f) = OpenOptions::new().write(true).open(path) {
        let _ = f.write_all(val.as_bytes());
    }
}

fn backlight_on() {
    write_sysfs(BL_POWER, "0");
    write_sysfs(BL_BRIGHTNESS, "101");
}

/// Wake the display panel (sleep-out). Tries the sysfs blank control AND the
/// FBIOBLANK ioctl on the framebuffer fd, since panel drivers vary in which
/// they honour. This is what hiby_player normally does to power the panel.
fn panel_wake(fb_fd: std::os::unix::io::RawFd) {
    // sysfs: write 0 (FB_BLANK_UNBLANK) to unblank.
    write_sysfs(FB_BLANK_SYSFS, "0");
    // ioctl: FBIOBLANK with FB_BLANK_UNBLANK (0). The request parameter type
    // (libc::Ioctl) differs by platform (u64 on x86-64, i32 on mips), so cast
    // via that alias to stay portable across native and target builds.
    unsafe {
        libc::ioctl(fb_fd, FBIOBLANK as libc::Ioctl, 0 as libc::c_int);
    }
}

/// Read a /sys/class/switch state file; returns true if it reads "1".
fn switch_active(path: &str) -> bool {
    std::fs::read_to_string(path)
        .map(|s| s.trim() == "1")
        .unwrap_or(false)
}

/// Set the ALSA 'Output Port Switch' (numid=9) via amixer subprocess.
fn set_output_port(port: u8) {
    let _ = std::process::Command::new("amixer")
        .args(["-c", "0", "cset", "numid=9", &port.to_string()])
        .output();
}

/// Detect which jack is plugged and select the matching output port.
/// Balanced (4.4mm) takes priority if both somehow read active. Returns the
/// port chosen, or None if neither jack is detected.
fn auto_select_output() -> Option<u8> {
    if switch_active(SW_BALANCE) {
        set_output_port(PORT_44MM);
        Some(PORT_44MM)
    } else if switch_active(SW_HEADSET) {
        set_output_port(PORT_35MM);
        Some(PORT_35MM)
    } else {
        None
    }
}

/// Send a command and return the daemon reply.
fn daemon_request(cmd: &str) -> Option<String> {
    let mut s = UnixStream::connect(DAEMON_SOCK).ok()?;
    s.write_all(cmd.as_bytes()).ok()?;
    s.write_all(&[10]).ok()?;

    let mut buf = [0u8; 1024];
    let read = s.read(&mut buf).ok()?;

    if read == 0 {
        return None;
    }

    Some(
        String::from_utf8_lossy(&buf[..read])
            .trim()
            .to_string(),
    )
}

/// Send a command without waiting for the daemon reply.
///
/// Playback controls use one connection per command. Dropping the stream after
/// the complete newline-terminated command lets the UI redraw immediately
/// while the daemon continues processing the request.
fn daemon_send(cmd: &str) {
    let Ok(mut stream) = UnixStream::connect(DAEMON_SOCK) else {
        return;
    };

    if stream.write_all(cmd.as_bytes()).is_err() {
        return;
    }

    let _ = stream.write_all(b"\n");
}

/// Extract a percentage from replies such as `OK vol 85`.
fn parse_volume_reply(reply: &str) -> Option<u8> {
    let value = reply.strip_prefix("OK vol ")?;
    let percent = value.trim().parse::<u8>().ok()?;

    if percent <= 100 {
        Some(percent)
    } else {
        None
    }
}

/// Current playback state reported by the daemon.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum PlaybackState {
    Unknown,
    Stopped,
    Loading,
    Playing,
    Paused,
    Error,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum StartupStage {
    Wifi,
    Spotify,
    Library,
}

impl StartupStage {
    fn label(self) -> &'static str {
        match self {
            Self::Wifi => "Starting Wi-Fi",
            Self::Spotify => "Connecting to Spotify",
            Self::Library => "Loading Liked Songs",
        }
    }

    fn detail(self) -> &'static str {
        match self {
            Self::Wifi => "Preparing the network connection",
            Self::Spotify => "Signing in and starting playback",
            Self::Library => "Fetching your saved tracks",
        }
    }

    fn progress_width(self) -> u32 {
        match self {
            Self::Wifi => 120,
            Self::Spotify => 240,
            Self::Library => 360,
        }
    }

    fn retry_label(self) -> &'static str {
        match self {
            Self::Wifi => "Retry Wi-Fi",
            Self::Spotify => "Retry Spotify",
            Self::Library => "Retry Library",
        }
    }
}

fn wifi_has_default_route() -> bool {
    std::fs::read_to_string("/proc/net/route")
        .ok()
        .map(|routes| {
            routes.lines().skip(1).any(|line| {
                let mut fields = line.split_whitespace();
                fields.next() == Some("wlan0")
                    && fields.next() == Some("00000000")
            })
        })
        .unwrap_or(false)
}

fn terminate_process_with_arg(argument: &str) -> bool {
    let Ok(entries) = std::fs::read_dir("/proc") else {
        return false;
    };

    for entry in entries.flatten() {
        let Some(pid) = entry
            .file_name()
            .to_str()
            .and_then(|name| name.parse::<i32>().ok())
        else {
            continue;
        };
        let cmdline_path = entry.path().join("cmdline");
        let Ok(cmdline) = std::fs::read(cmdline_path) else {
            continue;
        };

        if cmdline
            .split(|byte| *byte == 0)
            .any(|field| field == argument.as_bytes())
        {
            return unsafe { libc::kill(pid, libc::SIGTERM) == 0 };
        }
    }

    false
}

fn request_startup_recovery(stage: StartupStage) {
    match stage {
        StartupStage::Wifi => {
            let _ = std::process::Command::new("killall")
                .arg("udhcpc")
                .status();
            let _ = std::process::Command::new("wpa_cli")
                .args(["-i", "wlan0", "reassociate"])
                .status();
            let _ = std::process::Command::new("udhcpc")
                .args([
                    "-i",
                    "wlan0",
                    "-b",
                    "-n",
                    "-t",
                    "5",
                    "-T",
                    "2",
                    "-x",
                    "hostname:HiBy-R3PROII",
                ])
                .stdout(std::process::Stdio::null())
                .stderr(std::process::Stdio::null())
                .spawn();
        }
        StartupStage::Spotify => {
            terminate_process_with_arg("/usr/data/spotui_daemon");
        }
        StartupStage::Library => {}
    }
}

/// Check whether a named process is present without spawning `ps`. Diagnostics
/// refreshes are intentionally read-only and avoid adding work to playback.
fn process_running(name: &str) -> bool {
    let Ok(entries) = std::fs::read_dir("/proc") else {
        return false;
    };

    entries.flatten().any(|entry| {
        entry
            .file_name()
            .to_string_lossy()
            .chars()
            .all(|character| character.is_ascii_digit())
            && std::fs::read_to_string(entry.path().join("comm"))
                .map(|comm| comm.trim() == name)
                .unwrap_or(false)
    })
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum RepeatMode {
    Off,
    All,
    One,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct PlaybackModes {
    shuffle: bool,
    repeat: RepeatMode,
}

impl Default for PlaybackModes {
    fn default() -> Self {
        Self {
            shuffle: false,
            repeat: RepeatMode::Off,
        }
    }
}

fn daemon_playback_modes() -> Option<PlaybackModes> {
    let reply = daemon_request("PLAYBACK_MODES")?;
    let fields: Vec<&str> = reply.split_whitespace().collect();

    if fields.len() != 5
        || fields[0] != "MODES"
        || fields[1] != "SHUFFLE"
        || fields[3] != "REPEAT"
    {
        return None;
    }

    let shuffle = match fields[2] {
        "ON" => true,
        "OFF" => false,
        _ => return None,
    };
    let repeat = match fields[4] {
        "OFF" => RepeatMode::Off,
        "ALL" => RepeatMode::All,
        "ONE" => RepeatMode::One,
        _ => return None,
    };

    Some(PlaybackModes { shuffle, repeat })
}

impl PlaybackState {
    fn from_status_reply(reply: &str) -> Option<Self> {
        match reply {
            "STATUS STOPPED" => Some(Self::Stopped),
            "STATUS LOADING" => Some(Self::Loading),
            "STATUS PLAYING" => Some(Self::Playing),
            "STATUS PAUSED" => Some(Self::Paused),
            "STATUS ERROR" => Some(Self::Error),
            _ => None,
        }
    }

    fn is_paused(self) -> bool {
        self == Self::Paused
    }
}

/// Query the playback state reported by the daemon. Connection failures
/// return None so startup polling remains quiet until the daemon is available.
fn daemon_playback_state() -> Option<PlaybackState> {
    let mut s = UnixStream::connect(DAEMON_SOCK).ok()?;
    s.write_all(b"STATUS\n").ok()?;

    let mut buf = [0u8; 128];
    let n = s.read(&mut buf).ok()?;
    let reply = String::from_utf8_lossy(&buf[..n]);

    PlaybackState::from_status_reply(reply.trim())
}

#[derive(Clone, Debug, PartialEq, Eq)]
enum QueueSource {
    Liked,
    Playlist(String),
    Search,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct QueueStatus {
    source: QueueSource,
    request_id: u64,
    index: usize,
    length: usize,
    track_id: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct QueuePageItem {
    position: usize,
    source_index: usize,
    track_id: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct QueuePage {
    source: QueueSource,
    current_position: usize,
    total: usize,
    start: usize,
    items: Vec<QueuePageItem>,
}

struct PendingQueueSelection {
    source: QueueSource,
    track_id: String,
    request_id: u64,
    started: std::time::Instant,
}

struct PendingLoadCommand {
    request_id: u64,
    command: String,
    started: std::time::Instant,
}

/// Query the active playback queue position.
///
/// The outer Option indicates whether a valid daemon response was received.
/// The inner Option is None when playback is not using a queue.
fn daemon_queue_status() -> Option<Option<QueueStatus>> {
    let reply = daemon_request("QUEUE_STATUS")?;

    if reply == "QUEUE NONE" {
        return Some(None);
    }

    let (request_id, source, rest) =
        if let Some(rest) = reply.strip_prefix("QUEUE LIKED ") {
            let mut parts = rest.splitn(2, ' ');
            let request_id = parts.next()?.parse::<u64>().ok()?;
            let remaining = parts.next()?;
            (request_id, QueueSource::Liked, remaining)
        } else if let Some(rest) =
            reply.strip_prefix("QUEUE PLAYLIST ")
        {
            let mut parts = rest.splitn(3, ' ');
            let request_id = parts.next()?.parse::<u64>().ok()?;
            let playlist_id = parts.next()?.trim().to_string();
            let remaining = parts.next()?;

            if playlist_id.is_empty() {
                return None;
            }

            (
                request_id,
                QueueSource::Playlist(playlist_id),
                remaining,
            )
        } else if let Some(rest) =
            reply.strip_prefix("QUEUE SEARCH ")
        {
            let mut parts = rest.splitn(2, ' ');
            let request_id = parts.next()?.parse::<u64>().ok()?;
            let remaining = parts.next()?;
            (request_id, QueueSource::Search, remaining)
        } else {
            return None;
        };

    let mut parts = rest.splitn(3, ' ');
    let index = parts.next()?.parse::<usize>().ok()?;
    let length = parts.next()?.parse::<usize>().ok()?;
    let track_id = parts.next()?.trim().to_string();

    if track_id.is_empty() || index >= length {
        return None;
    }

    Some(Some(QueueStatus {
        source,
        request_id,
        index,
        length,
        track_id,
    }))
}

fn daemon_queue_page(offset: usize) -> Option<Option<QueuePage>> {
    let reply = daemon_request(&format!(
        "QUEUE_PAGE {} {}",
        offset, VISIBLE_ROWS
    ))?;

    if reply == "QUEUE_PAGE NONE" {
        return Some(None);
    }

    let mut lines = reply.lines();
    let header = lines.next()?;
    let mut fields = header.split_whitespace();

    if fields.next() != Some("QUEUE_PAGE") {
        return None;
    }

    let source_field = fields.next()?;
    let source = if source_field == "LIKED" {
        QueueSource::Liked
    } else if source_field == "SEARCH" {
        QueueSource::Search
    } else if let Some(playlist_id) =
        source_field.strip_prefix("PLAYLIST:")
    {
        if playlist_id.is_empty() {
            return None;
        }
        QueueSource::Playlist(playlist_id.to_string())
    } else {
        return None;
    };

    let _request_id = fields.next()?.parse::<u64>().ok()?;
    let current_position = fields.next()?.parse::<usize>().ok()?;
    let total = fields.next()?.parse::<usize>().ok()?;
    let start = fields.next()?.parse::<usize>().ok()?;
    let count = fields.next()?.parse::<usize>().ok()?;

    if fields.next().is_some()
        || current_position >= total
        || start > total
        || count > VISIBLE_ROWS
    {
        return None;
    }

    let mut items = Vec::with_capacity(count);
    for line in lines {
        let mut item_fields = line.split_whitespace();
        if item_fields.next() != Some("ITEM") {
            return None;
        }
        let position = item_fields.next()?.parse::<usize>().ok()?;
        let source_index = item_fields.next()?.parse::<usize>().ok()?;
        let track_id = item_fields.next()?.to_string();

        if item_fields.next().is_some()
            || track_id.is_empty()
            || position >= total
        {
            return None;
        }

        items.push(QueuePageItem {
            position,
            source_index,
            track_id,
        });
    }

    if items.len() != count {
        return None;
    }

    Some(Some(QueuePage {
        source,
        current_position,
        total,
        start,
        items,
    }))
}

/// Metadata for the track currently loaded by the daemon.
#[derive(Clone, Debug, PartialEq, Eq)]
struct NowPlaying {
    id: String,
    title: String,
    artist: String,
    duration_ms: u32,
}

/// Query the current track reported by the daemon.
///
/// The outer Option indicates whether a valid daemon response was received.
/// The inner Option is None when no track is loaded.
fn daemon_now_playing() -> Option<Option<NowPlaying>> {
    let mut s = UnixStream::connect(DAEMON_SOCK).ok()?;
    s.write_all(b"NOW_PLAYING\n").ok()?;

    let mut buf = [0u8; 1024];
    let n = s.read(&mut buf).ok()?;
    let reply = String::from_utf8_lossy(&buf[..n]);
    let reply = reply.trim();

    if reply == "NOW_PLAYING NONE" {
        return Some(None);
    }

    let rest = reply.strip_prefix("NOW_PLAYING ")?;
    let mut parts = rest.splitn(4, "\t");

    let id = parts.next()?.trim().to_string();
    let title = parts.next()?.trim().to_string();
    let artist = parts.next()?.trim().to_string();
    let duration_ms = parts.next()?.trim().parse().ok()?;

    if id.is_empty() {
        return None;
    }

    Some(Some(NowPlaying {
        id,
        title,
        artist,
        duration_ms,
    }))
}

/// Query the playback position reported by the daemon.
///
/// The outer Option indicates whether a valid daemon response was received.
/// The inner Option is None when no track is loaded.
fn daemon_playback_position() -> Option<Option<u32>> {
    let mut s = UnixStream::connect(DAEMON_SOCK).ok()?;
    s.write_all(b"POSITION\n").ok()?;

    let mut buf = [0u8; 128];
    let n = s.read(&mut buf).ok()?;
    let reply = String::from_utf8_lossy(&buf[..n]);
    let reply = reply.trim();

    if reply == "POSITION NONE" {
        return Some(None);
    }

    let position_ms = reply
        .strip_prefix("POSITION ")?
        .trim()
        .parse()
        .ok()?;

    Some(Some(position_ms))
}

/// A track fetched from the daemon browse commands.
#[derive(Clone)]
struct TrackItem {
    id: String,
    name: String,
    artist: String,
}

impl TrackItem {
    /// Display label for the list row.
    fn label(&self) -> String {
        if self.artist.is_empty() {
            self.name.clone()
        } else {
            format!("{} - {}", self.name, self.artist)
        }
    }
}

/// A playlist fetched from the daemon PLAYLISTS command.
#[derive(Clone)]
struct PlaylistItem {
    id: String,
    name: String,
    owner: String,
}

impl PlaylistItem {
    /// Display label for a playlist row.
    fn label(&self) -> String {
        if self.owner.is_empty() {
            self.name.clone()
        } else {
            format!("{} - {}", self.name, self.owner)
        }
    }
}

fn truncate_label(s: &str, max_chars: usize) -> String {
    let count = s.chars().count();
    if count <= max_chars {
        return s.to_string();
    }
    if max_chars <= 3 {
        return "...".chars().take(max_chars).collect();
    }
    let mut out: String = s.chars().take(max_chars - 3).collect();
    out.push_str("...");
    out
}

fn format_playback_time(milliseconds: u32) -> String {
    let total_seconds = milliseconds / 1000;
    let minutes = total_seconds / 60;
    let seconds = total_seconds % 60;
    format!("{minutes}:{seconds:02}")
}

const BACKLIGHT_BRIGHTNESS: &str = "/sys/class/backlight/backlight_pwm0/brightness";
const BATTERY_CAPACITY: &str = "/sys/class/power_supply/battery/capacity";
const STORAGE_PATH: &[u8] = b"/usr/data\0";
const BRIGHTNESS_STATE_FILE: &str = "/usr/data/spotui_brightness";
const THEME_STATE_FILE: &str = "/usr/data/spotui_theme";
const SCREEN_SLEEP_STATE_FILE: &str = "/usr/data/spotui_screen_sleep";
const SEARCH_HISTORY_STATE_FILE: &str = "/usr/data/spotui_search_history";
const SEARCH_HISTORY_LIMIT: usize = 8;
const BRIGHTNESS_LEVELS: [u32; 5] = [100, 80, 60, 40, 25];
const BRIGHTNESS_LABELS: [&str; 5] = ["100%", "80%", "60%", "40%", "25%"];
const SCREEN_SLEEP_TIMEOUTS: [Option<u128>; 5] = [
    Some(30_000),
    Some(60_000),
    Some(120_000),
    Some(300_000),
    None,
];


fn read_battery_percent() -> Option<u8> {
    std::fs::read_to_string(BATTERY_CAPACITY)
        .ok()
        .and_then(|s| s.trim().parse::<u8>().ok())
        .filter(|value| *value <= 100)
}

/// Return available space on /usr/data in whole mebibytes.
fn read_storage_free_mb() -> Option<u64> {
    let mut stats =
        std::mem::MaybeUninit::<libc::statvfs>::uninit();

    let result = unsafe {
        libc::statvfs(
            STORAGE_PATH.as_ptr() as *const libc::c_char,
            stats.as_mut_ptr(),
        )
    };

    if result != 0 {
        return None;
    }

    let stats = unsafe { stats.assume_init() };
    let block_size = if stats.f_frsize > 0 {
        stats.f_frsize as u64
    } else {
        stats.f_bsize as u64
    };

    Some(
        (stats.f_bavail as u64)
            .saturating_mul(block_size)
            / (1024 * 1024),
    )
}

/// Return available system memory in whole mebibytes.
///
/// Prefer MemAvailable when provided by the kernel. Older kernels fall back
/// to MemFree plus reclaimable buffers and cache.
fn read_memory_available_mb() -> Option<u64> {
    let meminfo = std::fs::read_to_string("/proc/meminfo").ok()?;

    let mut mem_available_kb: Option<u64> = None;
    let mut mem_free_kb: u64 = 0;
    let mut buffers_kb: u64 = 0;
    let mut cached_kb: u64 = 0;

    for line in meminfo.lines() {
        let mut fields = line.split_whitespace();

        let key = match fields.next() {
            Some(value) => value,
            None => continue,
        };

        let value_kb = match fields
            .next()
            .and_then(|value| value.parse::<u64>().ok())
        {
            Some(value) => value,
            None => continue,
        };

        match key {
            "MemAvailable:" => mem_available_kb = Some(value_kb),
            "MemFree:" => mem_free_kb = value_kb,
            "Buffers:" => buffers_kb = value_kb,
            "Cached:" => cached_kb = value_kb,
            _ => {}
        }
    }

    let available_kb = mem_available_kb.unwrap_or_else(|| {
        mem_free_kb
            .saturating_add(buffers_kb)
            .saturating_add(cached_kb)
    });

    if available_kb == 0 {
        None
    } else {
        Some(available_kb / 1024)
    }
}

fn load_brightness_idx() -> usize {
    match std::fs::read_to_string(BRIGHTNESS_STATE_FILE) {
        Ok(s) => match s.trim().parse::<usize>() {
            Ok(idx) if idx < BRIGHTNESS_LEVELS.len() => idx,
            _ => 0,
        },
        Err(_) => 0,
    }
}

fn save_brightness_idx(idx: usize) {
    if let Err(e) = std::fs::write(BRIGHTNESS_STATE_FILE, idx.to_string()) {
        eprintln!("[poc] brightness state save failed: {}", e);
    }
}

fn load_screen_sleep_idx() -> usize {
    match std::fs::read_to_string(SCREEN_SLEEP_STATE_FILE) {
        Ok(value) => match value.trim().parse::<usize>() {
            Ok(index) if index < SCREEN_SLEEP_TIMEOUTS.len() => index,
            _ => 1,
        },
        Err(_) => 1,
    }
}

fn save_screen_sleep_idx(index: usize) {
    if let Err(error) =
        std::fs::write(SCREEN_SLEEP_STATE_FILE, index.to_string())
    {
        eprintln!("[poc] screen sleep state save failed: {}", error);
    }
}

fn load_search_history() -> Vec<String> {
    std::fs::read_to_string(SEARCH_HISTORY_STATE_FILE)
        .map(|contents| {
            contents
                .lines()
                .map(str::trim)
                .filter(|query| !query.is_empty())
                .take(SEARCH_HISTORY_LIMIT)
                .map(str::to_string)
                .collect()
        })
        .unwrap_or_default()
}

fn save_search_history(history: &[String]) {
    let contents = history
        .iter()
        .take(SEARCH_HISTORY_LIMIT)
        .map(String::as_str)
        .collect::<Vec<_>>()
        .join("\n");

    if let Err(error) = std::fs::write(SEARCH_HISTORY_STATE_FILE, contents) {
        eprintln!("[poc] search history save failed: {}", error);
    }
}

fn remember_search(history: &mut Vec<String>, query: &str) {
    let query = query.trim();
    if query.is_empty() {
        return;
    }

    history.retain(|saved| !saved.eq_ignore_ascii_case(query));
    history.insert(0, query.to_string());
    history.truncate(SEARCH_HISTORY_LIMIT);
    save_search_history(history);
}

fn load_theme() -> Theme {
    match std::fs::read_to_string(THEME_STATE_FILE) {
        Ok(value) => match Theme::from_key(value.trim()) {
            Some(theme) => theme,
            None => {
                eprintln!(
                    "[poc] invalid saved theme {:?}; using El Kay Kay",
                    value.trim()
                );
                Theme::ElKayKay
            }
        },
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Theme::ElKayKay,
        Err(e) => {
            eprintln!(
                "[poc] theme state load failed: {}; using El Kay Kay",
                e
            );
            Theme::ElKayKay
        }
    }
}

fn save_theme(theme: Theme) {
    if let Err(e) = std::fs::write(THEME_STATE_FILE, theme.key()) {
        eprintln!("[poc] theme state save failed: {}", e);
    }
}

fn apply_brightness(idx: usize) {
    let level = BRIGHTNESS_LEVELS[idx];
    if let Err(e) = std::fs::write(BACKLIGHT_BRIGHTNESS, level.to_string()) {
        eprintln!("[poc] brightness write failed: {}", e);
    } else {
        eprintln!("[poc] brightness -> {}", BRIGHTNESS_LABELS[idx]);
    }
}

/// Send a browse command and read the full multi-line response until "END".
/// Parses `RESULT <id>\t<name>\t<artist>` lines into TrackItems.
fn daemon_query(cmd: &str) -> Vec<TrackItem> {
    let mut items = Vec::new();
    let mut s = match UnixStream::connect(DAEMON_SOCK) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("[poc] daemon_query connect failed: {e}");
            return items;
        }
    };
    if s.write_all(cmd.as_bytes()).is_err() || s.write_all(b"\n").is_err() {
        return items;
    }

    // Read the whole response into a buffer (browse responses are bounded:
    // up to ~50 tracks). Stop once a complete "END" line has arrived.
    let mut buf = Vec::new();
    let mut chunk = [0u8; 2048];
    loop {
        match s.read(&mut chunk) {
            Ok(0) => break,
            Ok(n) => {
                buf.extend_from_slice(&chunk[..n]);
                let text = String::from_utf8_lossy(&buf);
                if text.lines().any(|l| l.starts_with("END")) {
                    break;
                }
            }
            Err(_) => break,
        }
    }

    let text = String::from_utf8_lossy(&buf);
    for line in text.lines() {
        if let Some(rest) = line.strip_prefix("RESULT ") {
            let mut parts = rest.splitn(3, '\t');
            let id = parts.next().unwrap_or("").trim().to_string();
            let name = parts.next().unwrap_or("").trim().to_string();
            let artist = parts.next().unwrap_or("").trim().to_string();
            if !id.is_empty() {
                items.push(TrackItem { id, name, artist });
            }
        }
    }
    items
}

/// Send a playlist-listing command and parse its multiline response.
///
/// Expected lines:
/// `PLAYLIST <id>\t<name>\t<owner>`
/// followed by `END <count>`.
fn daemon_playlist_query(cmd: &str) -> Vec<PlaylistItem> {
    let mut playlists = Vec::new();

    let mut socket = match UnixStream::connect(DAEMON_SOCK) {
        Ok(socket) => socket,
        Err(e) => {
            eprintln!(
                "[poc] daemon_playlist_query connect failed: {e}"
            );
            return playlists;
        }
    };

    if socket.write_all(cmd.as_bytes()).is_err()
        || socket.write_all(b"\n").is_err()
    {
        return playlists;
    }

    // Profile playlist responses are bounded to at most 100 entries.
    let mut buffer = Vec::new();
    let mut chunk = [0u8; 2048];

    loop {
        match socket.read(&mut chunk) {
            Ok(0) => break,
            Ok(read) => {
                buffer.extend_from_slice(&chunk[..read]);

                let response = String::from_utf8_lossy(&buffer);
                if response
                    .lines()
                    .any(|line| line.starts_with("END "))
                {
                    break;
                }
            }
            Err(_) => break,
        }
    }

    let response = String::from_utf8_lossy(&buffer);

    for line in response.lines() {
        if let Some(rest) = line.strip_prefix("PLAYLIST ") {
            let mut parts = rest.splitn(3, '\t');

            let id =
                parts.next().unwrap_or("").trim().to_string();
            let name =
                parts.next().unwrap_or("").trim().to_string();
            let owner =
                parts.next().unwrap_or("").trim().to_string();

            if !id.is_empty() {
                playlists.push(PlaylistItem {
                    id,
                    name,
                    owner,
                });
            }
        }
    }

    playlists
}

/// Main content displayed above the now-playing strip.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum AppView {
    Library,
    Playlists,
    PlaylistTracks,
    SearchInput,
    SearchResults,
    SearchHistory,
    Menu,
    Sound,
    UpNext,
    NowPlaying,
    Appearance,
    Special,
    Diagnostics,
    Settings,
}

/// Stable identity for every built-in SpotUI colour theme.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Theme {
    ElKayKay,
    Tidepool,
    CitrusGrove,
    MonochromeStatic,
    PaperLantern,
    DurandalTerminal,
    ArcadeBloom,
    DesertBloom,
    AlNoor,
    NightMarket,
}

impl Theme {
    fn palette(self) -> Palette {
        match self {
            Theme::ElKayKay => Palette::el_kay_kay(),
            Theme::Tidepool => Palette::tidepool(),
            Theme::CitrusGrove => Palette::citrus_grove(),
            Theme::MonochromeStatic => Palette::monochrome_static(),
            Theme::PaperLantern => Palette::paper_lantern(),
            Theme::DurandalTerminal => Palette::durandal_terminal(),
            Theme::ArcadeBloom => Palette::arcade_bloom(),
            Theme::DesertBloom => Palette::desert_bloom(),
            Theme::AlNoor => Palette::al_noor(),
            Theme::NightMarket => Palette::night_market(),
        }
    }

    /// Stable identifier stored in /usr/data/spotui_theme.
    fn key(self) -> &'static str {
        match self {
            Theme::ElKayKay => "el-kay-kay",
            Theme::Tidepool => "tidepool",
            Theme::CitrusGrove => "citrus-grove",
            Theme::MonochromeStatic => "monochrome-static",
            Theme::PaperLantern => "paper-lantern",
            Theme::DurandalTerminal => "durandal-terminal",
            Theme::ArcadeBloom => "arcade-bloom",
            Theme::DesertBloom => "desert-bloom",
            Theme::AlNoor => "al-noor",
            Theme::NightMarket => "night-market",
        }
    }

    fn from_key(key: &str) -> Option<Self> {
        match key {
            "el-kay-kay" | "forest" => Some(Theme::ElKayKay),
            "tidepool" | "ocean" => Some(Theme::Tidepool),
            "citrus-grove" | "violet" => Some(Theme::CitrusGrove),
            "monochrome-static" | "bauhaus" | "amber" => {
                Some(Theme::MonochromeStatic)
            }
            "paper-lantern" | "monochrome" => {
                Some(Theme::PaperLantern)
            }
            "durandal-terminal" => Some(Theme::DurandalTerminal),
            "arcade-bloom" | "synthwave" => Some(Theme::ArcadeBloom),
            "desert-bloom" | "sunset" => Some(Theme::DesertBloom),
            "al-noor" | "ice" => Some(Theme::AlNoor),
            "night-market" | "crimson" => Some(Theme::NightMarket),
            _ => None,
        }
    }
}

/// Top-level menu tiles.
const MENU_LABELS: [&str; 6] = [
    "Search",
    "Sound",
    "Library",
    "Appearance",
    "Device",
    "Diagnostics",
];

const SOUND_LABELS: [&str; 6] = [
    "Shuffle",
    "Repeat Off",
    "Repeat All",
    "Repeat One",
    "Up Next",
    "Back",
];

const SEARCH_KEY_ROWS: [&str; 3] = [
    "QWERTYUIOP",
    "ASDFGHJKL",
    "ZXCVBNM",
];

/// Appearance submenu tiles.
const APPEARANCE_LABELS: [&str; 6] = [
    "El Kay Kay",
    "Tidepool",
    "Citrus Grove",
    "Monochrome",
    "Paper Lantern",
    "Page 2",
];

/// Special theme submenu tiles.
const SPECIAL_LABELS: [&str; 6] = [
    "Durandal",
    "Arcade Bloom",
    "Desert Bloom",
    "Al Noor",
    "Night Market",
    "Back",
];

/// Diagnostics submenu tiles.
const DIAGNOSTICS_LABELS: [&str; 6] = [
    "Wi-Fi",
    "Spotify",
    "Audio",
    "Output",
    "Queue",
    "Refresh Status",
];

const SETTINGS_LABELS: [&str; 6] = [
    "Sleep: 30 sec",
    "Sleep: 60 sec",
    "Sleep: 2 min",
    "Sleep: 5 min",
    "Sleep: Never",
    "Back",
];

/// Colours used by the main SpotUI interface.
///
/// Keeping these values together allows complete theme presets to be added
/// without scattering theme-specific checks throughout the drawing code.
#[derive(Clone, Copy)]
struct Palette {
    background: Rgb565,
    header: Rgb565,
    header_text: Rgb565,
    text: Rgb565,
    selected_row: Rgb565,
    selected_text: Rgb565,
    now_playing: Rgb565,
    progress_track: Rgb565,
    progress_fill: Rgb565,
    separator: Rgb565,
    toolbar: Rgb565,
    border: Rgb565,
}

impl Palette {
    /// Navy, taupe, dark-grey, purple, and pink palette.
    fn el_kay_kay() -> Self {
        Self {
            background: Rgb565::new(1, 4, 10),
            header: Rgb565::new(20, 36, 17),
            header_text: Rgb565::new(1, 4, 10),
            text: Rgb565::new(30, 54, 27),
            selected_row: Rgb565::new(18, 8, 16),
            selected_text: Rgb565::new(31, 50, 27),
            now_playing: Rgb565::new(7, 7, 12),
            progress_track: Rgb565::new(10, 17, 12),
            progress_fill: Rgb565::new(31, 30, 23),
            separator: Rgb565::new(13, 7, 12),
            toolbar: Rgb565::new(18, 18, 17),
            border: Rgb565::new(25, 18, 18),
        }
    }

    /// Petrol blue, seafoam, coral, sand, and deep-ink palette.
    fn tidepool() -> Self {
        Self {
            background: Rgb565::new(1, 8, 11),
            header: Rgb565::new(30, 30, 18),
            header_text: Rgb565::new(1, 8, 11),
            text: Rgb565::new(29, 55, 24),
            selected_row: Rgb565::new(2, 35, 26),
            selected_text: Rgb565::new(31, 60, 26),
            now_playing: Rgb565::new(1, 16, 19),
            progress_track: Rgb565::new(3, 24, 22),
            progress_fill: Rgb565::new(12, 55, 24),
            separator: Rgb565::new(18, 16, 13),
            toolbar: Rgb565::new(2, 25, 24),
            border: Rgb565::new(24, 26, 16),
        }
    }

    /// Olive, chartreuse, tangerine, cream, and espresso palette.
    fn citrus_grove() -> Self {
        Self {
            background: Rgb565::new(4, 4, 1),
            header: Rgb565::new(31, 30, 2),
            header_text: Rgb565::new(4, 4, 1),
            text: Rgb565::new(31, 58, 23),
            selected_row: Rgb565::new(15, 28, 2),
            selected_text: Rgb565::new(31, 61, 22),
            now_playing: Rgb565::new(3, 12, 2),
            progress_track: Rgb565::new(10, 20, 2),
            progress_fill: Rgb565::new(22, 60, 3),
            separator: Rgb565::new(12, 13, 1),
            toolbar: Rgb565::new(10, 22, 2),
            border: Rgb565::new(25, 25, 2),
        }
    }

    /// High-contrast monochrome palette with an animated snow texture.
    fn monochrome_static() -> Self {
        Self {
            background: Rgb565::new(2, 4, 2),
            header: Rgb565::new(26, 52, 26),
            header_text: Rgb565::BLACK,
            text: Rgb565::new(29, 58, 29),
            selected_row: Rgb565::new(18, 36, 18),
            selected_text: Rgb565::WHITE,
            now_playing: Rgb565::new(5, 10, 5),
            progress_track: Rgb565::new(11, 22, 11),
            progress_fill: Rgb565::WHITE,
            separator: Rgb565::new(9, 18, 9),
            toolbar: Rgb565::new(13, 26, 13),
            border: Rgb565::new(22, 44, 22),
        }
    }

    /// Indigo, persimmon, jade, parchment, and charcoal palette.
    fn paper_lantern() -> Self {
        Self {
            background: Rgb565::new(3, 4, 11),
            header: Rgb565::new(29, 54, 23),
            header_text: Rgb565::new(3, 4, 11),
            text: Rgb565::new(30, 57, 25),
            selected_row: Rgb565::new(4, 31, 17),
            selected_text: Rgb565::new(30, 57, 25),
            now_playing: Rgb565::new(5, 8, 10),
            progress_track: Rgb565::new(12, 17, 13),
            progress_fill: Rgb565::new(31, 24, 6),
            separator: Rgb565::new(10, 8, 12),
            toolbar: Rgb565::new(3, 20, 14),
            border: Rgb565::new(25, 25, 9),
        }
    }

    /// Terminal and industrial palette inspired by the classic Marathon era.
    fn durandal_terminal() -> Self {
        Self {
            background: Rgb565::new(1, 3, 5),
            header: Rgb565::new(13, 20, 8),
            header_text: Rgb565::new(8, 63, 8),
            text: Rgb565::new(28, 58, 27),
            selected_row: Rgb565::new(12, 7, 17),
            selected_text: Rgb565::new(8, 63, 8),
            now_playing: Rgb565::new(3, 7, 10),
            progress_track: Rgb565::CSS_DARK_GRAY,
            progress_fill: Rgb565::new(8, 63, 8),
            separator: Rgb565::new(7, 13, 5),
            toolbar: Rgb565::new(22, 5, 4),
            border: Rgb565::new(14, 28, 14),
        }
    }

    /// Cobalt, acid-green, hot-coral, lavender, and near-black palette.
    fn arcade_bloom() -> Self {
        Self {
            background: Rgb565::new(2, 1, 6),
            header: Rgb565::new(3, 13, 31),
            header_text: Rgb565::new(28, 63, 5),
            text: Rgb565::new(25, 48, 31),
            selected_row: Rgb565::new(20, 60, 3),
            selected_text: Rgb565::new(2, 1, 6),
            now_playing: Rgb565::new(8, 3, 13),
            progress_track: Rgb565::new(12, 10, 18),
            progress_fill: Rgb565::new(31, 18, 12),
            separator: Rgb565::new(13, 4, 20),
            toolbar: Rgb565::new(4, 10, 24),
            border: Rgb565::new(20, 60, 3),
        }
    }

    /// Terracotta, turquoise, saffron, plum, and bone palette.
    fn desert_bloom() -> Self {
        Self {
            background: Rgb565::new(8, 3, 9),
            header: Rgb565::new(31, 44, 5),
            header_text: Rgb565::new(8, 3, 9),
            text: Rgb565::new(31, 57, 24),
            selected_row: Rgb565::new(2, 40, 25),
            selected_text: Rgb565::new(31, 60, 26),
            now_playing: Rgb565::new(14, 7, 7),
            progress_track: Rgb565::new(19, 13, 9),
            progress_fill: Rgb565::new(31, 44, 5),
            separator: Rgb565::new(18, 8, 8),
            toolbar: Rgb565::new(22, 12, 8),
            border: Rgb565::new(2, 40, 25),
        }
    }

    /// Pastel cherry, yellow, lilac, and mint palette.
    fn al_noor() -> Self {
        Self {
            background: Rgb565::new(5, 2, 8),
            header: Rgb565::new(25, 45, 27),
            header_text: Rgb565::new(5, 2, 8),
            text: Rgb565::new(21, 58, 25),
            selected_row: Rgb565::new(26, 12, 11),
            selected_text: Rgb565::new(31, 57, 12),
            now_playing: Rgb565::new(9, 4, 12),
            progress_track: Rgb565::CSS_DARK_GRAY,
            progress_fill: Rgb565::new(31, 54, 10),
            separator: Rgb565::new(14, 15, 20),
            toolbar: Rgb565::new(18, 6, 10),
            border: Rgb565::new(19, 40, 24),
        }
    }

    /// Lacquer-red, jade, gold, midnight-blue, and charcoal palette.
    fn night_market() -> Self {
        Self {
            background: Rgb565::new(1, 3, 8),
            header: Rgb565::new(26, 5, 4),
            header_text: Rgb565::new(31, 48, 5),
            text: Rgb565::new(30, 56, 23),
            selected_row: Rgb565::new(3, 31, 16),
            selected_text: Rgb565::new(31, 58, 24),
            now_playing: Rgb565::new(4, 7, 8),
            progress_track: Rgb565::new(11, 13, 10),
            progress_fill: Rgb565::new(31, 48, 5),
            separator: Rgb565::new(14, 5, 6),
            toolbar: Rgb565::new(18, 4, 5),
            border: Rgb565::new(24, 35, 5),
        }
    }
}

/// Number of track rows visible above the now-playing strip.
/// Nine 60-pixel rows leave 60 pixels for current-track metadata.
const VISIBLE_ROWS: usize = 9;

fn draw_monochrome_static(
    fb: &mut Framebuffer,
    mut seed: u32,
) {
    for _ in 0..180 {
        seed ^= seed << 13;
        seed ^= seed >> 17;
        seed ^= seed << 5;
        let x = (seed % WIDTH as u32) as i32;

        seed ^= seed << 13;
        seed ^= seed >> 17;
        seed ^= seed << 5;
        let y = 40 + (seed % 540) as i32;
        let level = 4 + ((seed >> 16) % 15) as u8;
        let color = Rgb565::new(level, level * 2, level);

        Pixel(Point::new(x, y), color).draw(fb).ok();
    }
}

fn draw_al_noor_sky(
    fb: &mut Framebuffer,
    frame: u32,
    palette: &Palette,
) {
    for (base_x, base_y, speed) in [
        (28u32, 92u32, 1u32),
        (250, 238, 2),
        (118, 410, 3),
    ] {
        let x = ((base_x + frame * speed) % 440) as i32;
        let drift = ((frame + base_x) % 24) as i32;
        let y = base_y as i32 + drift - 12;

        Circle::new(Point::new(x, y), 22)
            .into_styled(PrimitiveStyle::with_fill(
                palette.progress_fill,
            ))
            .draw(fb)
            .ok();

        Circle::new(Point::new(x + 7, y - 3), 22)
            .into_styled(PrimitiveStyle::with_fill(
                palette.background,
            ))
            .draw(fb)
            .ok();
    }

    for index in 0..28u32 {
        let x = ((index * 83 + 19 + frame * (index % 3 + 1))
            % WIDTH as u32) as i32;
        let y = 45
            + ((index * 47 + frame * (index % 2 + 1)) % 525)
                as i32;
        let color = if (index + frame) % 4 == 0 {
            palette.progress_fill
        } else {
            palette.text
        };

        Pixel(Point::new(x, y), color).draw(fb).ok();
    }
}

fn draw_el_kay_kay_motes(
    fb: &mut Framebuffer,
    frame: u32,
    palette: &Palette,
) {
    for index in 0..14u32 {
        let x = ((index * 71 + frame * (index % 4 + 1))
            % WIDTH as u32) as i32;
        let y = 48
            + ((index * 43 + frame * (index % 3 + 1)) % 520)
                as i32;
        let diameter = 1 + (index % 3);
        let color = if (index + frame) % 4 == 0 {
            palette.header
        } else {
            palette.progress_fill
        };

        Circle::new(Point::new(x, y), diameter)
            .into_styled(PrimitiveStyle::with_fill(color))
            .draw(fb)
            .ok();
    }
}

fn draw_el_kay_kay_puppy(fb: &mut Framebuffer, frame: u32) {
    let x = 389;
    let y = 478 + (frame % 2) as i32;
    let white = Rgb565::WHITE;
    let black = Rgb565::BLACK;
    let pink = Rgb565::new(31, 30, 23);

    // Symmetrical floppy ears frame a large, front-facing white head.
    let ear_drop = (frame % 3) as i32;
    Circle::new(Point::new(x - 13, y + 9 + ear_drop), 34)
        .into_styled(PrimitiveStyle::with_fill(black))
        .draw(fb)
        .ok();
    Circle::new(Point::new(x + 60, y + 9 + ear_drop), 34)
        .into_styled(PrimitiveStyle::with_fill(black))
        .draw(fb)
        .ok();
    Ellipse::new(Point::new(x + 3, y + 4), Size::new(76, 72))
        .into_styled(PrimitiveStyle::with_fill(white))
        .draw(fb)
        .ok();

    // Soft black crown patches make the white puppy markings unmistakable.
    Circle::new(Point::new(x + 5, y - 2), 29)
        .into_styled(PrimitiveStyle::with_fill(black))
        .draw(fb)
        .ok();
    Circle::new(Point::new(x + 48, y - 2), 29)
        .into_styled(PrimitiveStyle::with_fill(black))
        .draw(fb)
        .ok();

    // Tall oval anime eyes with large highlights soften the expression.
    for eye_x in [x + 15, x + 49] {
        Ellipse::new(Point::new(eye_x, y + 24), Size::new(21, 29))
            .into_styled(PrimitiveStyle::with_fill(black))
            .draw(fb)
            .ok();
        Ellipse::new(
            Point::new(eye_x + 5, y + 28),
            Size::new(8, 11),
        )
            .into_styled(PrimitiveStyle::with_fill(white))
            .draw(fb)
            .ok();
        Circle::new(Point::new(eye_x + 4, y + 43), 4)
            .into_styled(PrimitiveStyle::with_fill(white))
            .draw(fb)
            .ok();
    }

    // Rounded muzzle, triangular-looking nose, small smile, and pink cheeks.
    Circle::new(Point::new(x + 27, y + 45), 28)
        .into_styled(PrimitiveStyle::with_fill(white))
        .draw(fb)
        .ok();
    Ellipse::new(Point::new(x + 36, y + 49), Size::new(10, 8))
        .into_styled(PrimitiveStyle::with_fill(black))
        .draw(fb)
        .ok();
    Rectangle::new(Point::new(x + 40, y + 56), Size::new(2, 6))
        .into_styled(PrimitiveStyle::with_fill(black))
        .draw(fb)
        .ok();
    Circle::new(Point::new(x + 31, y + 57), 11)
        .into_styled(PrimitiveStyle::with_stroke(black, 2))
        .draw(fb)
        .ok();
    Circle::new(Point::new(x + 41, y + 57), 11)
        .into_styled(PrimitiveStyle::with_stroke(black, 2))
        .draw(fb)
        .ok();
    Circle::new(Point::new(x + 8, y + 51), 8)
        .into_styled(PrimitiveStyle::with_fill(pink))
        .draw(fb)
        .ok();
    Circle::new(Point::new(x + 66, y + 51), 8)
        .into_styled(PrimitiveStyle::with_fill(pink))
        .draw(fb)
        .ok();
}

fn draw_tidepool_bubbles(
    fb: &mut Framebuffer,
    frame: u32,
    palette: &Palette,
) {
    for index in 0..10u32 {
        let x = ((index * 89 + frame * (index % 3 + 1))
            % WIDTH as u32) as i32;
        let y = 570
            - ((index * 67 + frame * (index % 4 + 2)) % 520)
                as i32;
        let diameter = 3 + (index % 4) * 2;

        Circle::new(Point::new(x, y), diameter)
            .into_styled(PrimitiveStyle::with_stroke(
                if index % 3 == 0 {
                    palette.progress_fill
                } else {
                    palette.border
                },
                1,
            ))
            .draw(fb)
            .ok();
    }
}

fn draw_citrus_grove_leaves(
    fb: &mut Framebuffer,
    frame: u32,
    palette: &Palette,
) {
    for index in 0..12u32 {
        let x = ((index * 97 + frame * (index % 5 + 1))
            % WIDTH as u32) as i32;
        let y = 44
            + ((index * 53 + frame * (index % 3 + 1)) % 530)
                as i32;
        let size = if index % 4 == 0 { 7 } else { 5 };

        Rectangle::new(Point::new(x, y), Size::new(size + 2, size))
            .into_styled(PrimitiveStyle::with_fill(
                if index % 3 == 0 {
                    palette.header
                } else {
                    palette.progress_fill
                },
            ))
            .draw(fb)
            .ok();
    }
}

fn draw_paper_lanterns(
    fb: &mut Framebuffer,
    frame: u32,
    palette: &Palette,
) {
    for index in 0..6u32 {
        let x = ((index * 113 + frame * (index % 2 + 1))
            % WIDTH as u32) as i32;
        let y = 550
            - ((index * 91 + frame * (index % 3 + 1)) % 500)
                as i32;

        Rectangle::new(Point::new(x, y), Size::new(9, 12))
            .into_styled(PrimitiveStyle::with_fill(
                if index % 2 == 0 {
                    palette.progress_fill
                } else {
                    palette.header
                },
            ))
            .draw(fb)
            .ok();
        Rectangle::new(Point::new(x + 2, y + 12), Size::new(5, 2))
            .into_styled(PrimitiveStyle::with_fill(palette.border))
            .draw(fb)
            .ok();
    }
}

fn draw_arcade_sparks(
    fb: &mut Framebuffer,
    frame: u32,
    palette: &Palette,
) {
    for index in 0..14u32 {
        let x = ((index * 61 + frame * (index % 4 + 2))
            % WIDTH as u32) as i32;
        let y = 45
            + ((index * 79 + frame * (index % 3 + 1)) % 525)
                as i32;
        let color = match index % 3 {
            0 => palette.progress_fill,
            1 => palette.border,
            _ => palette.header,
        };

        Rectangle::new(Point::new(x, y), Size::new(2, 2))
            .into_styled(PrimitiveStyle::with_fill(color))
            .draw(fb)
            .ok();
    }
}

fn draw_desert_dust(
    fb: &mut Framebuffer,
    frame: u32,
    palette: &Palette,
) {
    for index in 0..11u32 {
        let x = ((index * 101 + frame * (index % 5 + 2))
            % WIDTH as u32) as i32;
        let y = 70 + ((index * 47) % 480) as i32;
        let width = 2 + index % 4;

        Rectangle::new(Point::new(x, y), Size::new(width, 1))
            .into_styled(PrimitiveStyle::with_fill(
                if index % 4 == 0 {
                    palette.border
                } else {
                    palette.progress_fill
                },
            ))
            .draw(fb)
            .ok();
    }
}

fn draw_night_market_lights(
    fb: &mut Framebuffer,
    frame: u32,
    palette: &Palette,
) {
    for index in 0..7u32 {
        let x = ((index * 109 + frame * (index % 3 + 1))
            % WIDTH as u32) as i32;
        let y = 58
            + ((index * 73 + frame * (index % 2 + 1)) % 500)
                as i32;
        let lit = (index + frame) % 4 != 0;

        Rectangle::new(Point::new(x, y), Size::new(4, 6))
            .into_styled(PrimitiveStyle::with_fill(if lit {
                palette.progress_fill
            } else {
                palette.header
            }))
            .draw(fb)
            .ok();
        Pixel(Point::new(x + 1, y + 7), palette.border)
            .draw(fb)
            .ok();
    }
}

/// Draw a temporary volume indicator over the now-playing strip.
fn draw_volume_popup(
    fb: &mut Framebuffer,
    volume_percent: u8,
    palette: &Palette,
) {
    const POPUP_WIDTH: u32 = 240;
    const POPUP_HEIGHT: u32 = 44;

    let popup_x =
        (WIDTH as i32 - POPUP_WIDTH as i32) / 2;
    let popup_y = HEIGHT as i32 - 136;

    Rectangle::new(
        Point::new(popup_x - 2, popup_y - 2),
        Size::new(POPUP_WIDTH + 4, POPUP_HEIGHT + 4),
    )
    .into_styled(PrimitiveStyle::with_fill(palette.border))
    .draw(fb)
    .ok();

    Rectangle::new(
        Point::new(popup_x, popup_y),
        Size::new(POPUP_WIDTH, POPUP_HEIGHT),
    )
    .into_styled(PrimitiveStyle::with_fill(palette.toolbar))
    .draw(fb)
    .ok();

    let label = format!("Volume {}%", volume_percent);
    let label_width = label.chars().count() as i32 * 9;
    let label_x = (WIDTH as i32 - label_width) / 2;
    let popup_style =
        MonoTextStyle::new(&FONT_9X15_BOLD, palette.text);

    Text::with_baseline(
        &label,
        Point::new(label_x, popup_y + 14),
        popup_style,
        Baseline::Top,
    )
    .draw(fb)
    .ok();

    fb.flush().ok();
}

/// Redraw only the 60-pixel current-track strip.
fn draw_now_playing_strip(
    fb: &mut Framebuffer,
    now_playing: Option<&NowPlaying>,
    playback_position: Option<u32>,
    playback_state: PlaybackState,
    palette: &Palette,
) {
    let text_style =
        MonoTextStyle::new(&FONT_9X15, palette.text);

    // Dedicated current-track strip between the list and toolbar.
    let now_playing_y = HEIGHT as i32 - 140;
    Rectangle::new(
        Point::new(0, now_playing_y),
        Size::new(WIDTH as u32, 60),
    )
    .into_styled(PrimitiveStyle::with_fill(palette.now_playing))
    .draw(fb)
    .ok();

    match now_playing {
        Some(item) => {
            let duration_ms = item.duration_ms;
            let position_ms = playback_position.unwrap_or(0).min(duration_ms);
            let time_label = format!(
                "{} / {}",
                format_playback_time(position_ms),
                format_playback_time(duration_ms)
            );
            let time_width = time_label.chars().count() as i32 * 9;

            let artist_available_width =
                (WIDTH as i32 - 20 - time_width - 12).max(0);
            let artist_max_chars = (artist_available_width / 9) as usize;

            let now_title = truncate_label(&item.title, 43);
            let artist_source = if item.artist.is_empty() {
                "Unknown artist"
            } else {
                item.artist.as_str()
            };
            let now_artist =
                truncate_label(artist_source, artist_max_chars);

            let now_title_style =
                MonoTextStyle::new(&FONT_9X15_BOLD, palette.text);

            Text::with_baseline(
                "<|",
                Point::new(8, now_playing_y + 3),
                now_title_style,
                Baseline::Top,
            )
            .draw(fb)
            .ok();

            Text::with_baseline(
                "|>",
                Point::new(452, now_playing_y + 3),
                now_title_style,
                Baseline::Top,
            )
            .draw(fb)
            .ok();

            Text::with_baseline(
                &now_title,
                Point::new(38, now_playing_y + 3),
                now_title_style,
                Baseline::Top,
            )
            .draw(fb)
            .ok();

            Text::with_baseline(
                &now_artist,
                Point::new(10, now_playing_y + 21),
                text_style,
                Baseline::Top,
            )
            .draw(fb)
            .ok();

            Text::with_baseline(
                &time_label,
                Point::new(
                    WIDTH as i32 - 10 - time_width,
                    now_playing_y + 21,
                ),
                text_style,
                Baseline::Top,
            )
            .draw(fb)
            .ok();

            let progress_x = 10;
            let progress_y = now_playing_y + 53;
            let progress_width = WIDTH as u32 - 20;

            Rectangle::new(
                Point::new(progress_x, progress_y),
                Size::new(progress_width, 4),
            )
            .into_styled(PrimitiveStyle::with_fill(
                palette.progress_track,
            ))
            .draw(fb)
            .ok();

            if duration_ms > 0 {
                let filled_width = (
                    position_ms as u64 * progress_width as u64
                        / duration_ms as u64
                ) as u32;

                if filled_width > 0 {
                    Rectangle::new(
                        Point::new(progress_x, progress_y),
                        Size::new(filled_width, 4),
                    )
                    .into_styled(PrimitiveStyle::with_fill(
                        palette.progress_fill,
                    ))
                    .draw(fb)
                    .ok();
                }
            }
        }
        None => {
            let empty_label = match playback_state {
                PlaybackState::Unknown => "Reconnecting...",
                PlaybackState::Error => "Playback error",
                _ => "Nothing playing",
            };
            Text::with_baseline(
                empty_label,
                Point::new(10, now_playing_y + 22),
                text_style,
                Baseline::Top,
            )
            .draw(fb)
            .ok();
        }
    }
}

fn draw_now_playing_view(
    fb: &mut Framebuffer,
    now_playing: Option<&NowPlaying>,
    playback_position: Option<u32>,
    playback_state: PlaybackState,
    palette: &Palette,
) {
    Rectangle::new(Point::new(0, 40), Size::new(WIDTH as u32, 620))
        .into_styled(PrimitiveStyle::with_fill(palette.background))
        .draw(fb)
        .ok();

    let title_style = MonoTextStyle::new(&FONT_10X20, palette.text);
    let text_style = MonoTextStyle::new(&FONT_9X15, palette.text);
    let button_style = MonoTextStyle::new(&FONT_9X15_BOLD, palette.text);

    match now_playing {
        Some(item) => {
            let title = truncate_label(&item.title, 42);
            let artist = truncate_label(
                if item.artist.is_empty() {
                    "Unknown artist"
                } else {
                    item.artist.as_str()
                },
                48,
            );
            let title_x = (WIDTH as i32 - title.chars().count() as i32 * 10) / 2;
            let artist_x = (WIDTH as i32 - artist.chars().count() as i32 * 9) / 2;

            Text::with_baseline(
                &title,
                Point::new(title_x.max(8), 145),
                title_style,
                Baseline::Top,
            )
            .draw(fb)
            .ok();
            Text::with_baseline(
                &artist,
                Point::new(artist_x.max(8), 190),
                text_style,
                Baseline::Top,
            )
            .draw(fb)
            .ok();

            let duration_ms = item.duration_ms;
            let position_ms = playback_position.unwrap_or(0).min(duration_ms);
            let elapsed = format_playback_time(position_ms);
            let remaining = format!(
                "-{}",
                format_playback_time(duration_ms.saturating_sub(position_ms))
            );

            Text::with_baseline(
                &elapsed,
                Point::new(20, 322),
                text_style,
                Baseline::Top,
            )
            .draw(fb)
            .ok();
            Text::with_baseline(
                &remaining,
                Point::new(460 - remaining.chars().count() as i32 * 9, 322),
                text_style,
                Baseline::Top,
            )
            .draw(fb)
            .ok();

            Rectangle::new(Point::new(20, 300), Size::new(440, 10))
                .into_styled(PrimitiveStyle::with_fill(palette.progress_track))
                .draw(fb)
                .ok();
            if duration_ms > 0 {
                let filled =
                    (position_ms as u64 * 440 / duration_ms as u64) as u32;
                if filled > 0 {
                    Rectangle::new(Point::new(20, 300), Size::new(filled, 10))
                        .into_styled(PrimitiveStyle::with_fill(palette.progress_fill))
                        .draw(fb)
                        .ok();
                }
            }
        }
        None => {
            let label = match playback_state {
                PlaybackState::Unknown => "Reconnecting...",
                PlaybackState::Error => "Playback error",
                _ => "Nothing playing",
            };
            let label_x = (WIDTH as i32 - label.chars().count() as i32 * 10) / 2;
            Text::with_baseline(
                label,
                Point::new(label_x, 190),
                title_style,
                Baseline::Top,
            )
            .draw(fb)
            .ok();
        }
    }

    for x in [0, 160, 320] {
        Rectangle::new(Point::new(x, 390), Size::new(160, 100))
            .into_styled(PrimitiveStyle::with_fill(palette.now_playing))
            .draw(fb)
            .ok();
    }
    for x in [160, 320] {
        Rectangle::new(Point::new(x, 390), Size::new(1, 100))
            .into_styled(PrimitiveStyle::with_fill(palette.border))
            .draw(fb)
            .ok();
    }

    let middle_label = if playback_state.is_paused() { "Play" } else { "Pause" };
    for (label, center_x) in [("Previous", 80), (middle_label, 240), ("Next", 400)] {
        Text::with_baseline(
            label,
            Point::new(center_x - label.chars().count() as i32 * 9 / 2, 432),
            button_style,
            Baseline::Top,
        )
        .draw(fb)
        .ok();
    }

    Rectangle::new(Point::new(40, 520), Size::new(400, 90))
        .into_styled(PrimitiveStyle::with_fill(palette.now_playing))
        .draw(fb)
        .ok();
    let queue_label = "View Up Next";
    Text::with_baseline(
        queue_label,
        Point::new((WIDTH as i32 - queue_label.len() as i32 * 9) / 2, 558),
        button_style,
        Baseline::Top,
    )
    .draw(fb)
    .ok();
}

/// Draw the track list with scrolling. `scroll` is the index of the first
/// visible item; `selected` highlights one row (absolute index).
fn draw_list(
    fb: &mut Framebuffer,
    items: &[TrackItem],
    playlists: &[PlaylistItem],
    playlist_tracks: &[TrackItem],
    search_results: &[TrackItem],
    recent_searches: &[String],
    search_query: &str,
    search_in_progress: bool,
    up_next_page: Option<&QueuePage>,
    scroll: usize,
    playlist_scroll: usize,
    playlist_track_scroll: usize,
    search_scroll: usize,
    selected: Option<usize>,
    playlist_selected: Option<usize>,
    playlist_track_selected: Option<usize>,
    search_selected: Option<usize>,
    title: &str,
    battery_percent: Option<u8>,
    _storage_free_mb: Option<u64>,
    _memory_available_mb: Option<u64>,
    brightness_idx: usize,
    screen_sleep_idx: usize,
    playback_state: PlaybackState,
    startup_stage: Option<StartupStage>,
    startup_elapsed_secs: u64,
    startup_retry_active: bool,
    playback_modes: PlaybackModes,
    queue_status: Option<&QueueStatus>,
    diagnostics_refreshed: bool,
    now_playing: Option<&NowPlaying>,
    playback_position: Option<u32>,
    palette: &Palette,
    active_theme: Theme,
    static_seed: u32,
    animation_frame: u32,
    app_view: AppView,
    exit_armed: bool,
) {
    // Clear to dark blue.
    Rectangle::new(Point::zero(), Size::new(WIDTH as u32, HEIGHT as u32))
        .into_styled(PrimitiveStyle::with_fill(palette.background))
        .draw(fb)
        .ok();

    if active_theme == Theme::ElKayKay {
        draw_el_kay_kay_motes(fb, animation_frame, palette);
        draw_el_kay_kay_puppy(fb, animation_frame);
    } else if active_theme == Theme::Tidepool {
        draw_tidepool_bubbles(fb, animation_frame, palette);
    } else if active_theme == Theme::CitrusGrove {
        draw_citrus_grove_leaves(fb, animation_frame, palette);
    } else if active_theme == Theme::MonochromeStatic {
        draw_monochrome_static(fb, static_seed);
    } else if active_theme == Theme::PaperLantern {
        draw_paper_lanterns(fb, animation_frame, palette);
    } else if active_theme == Theme::ArcadeBloom {
        draw_arcade_sparks(fb, animation_frame, palette);
    } else if active_theme == Theme::DesertBloom {
        draw_desert_dust(fb, animation_frame, palette);
    } else if active_theme == Theme::AlNoor {
        draw_al_noor_sky(fb, animation_frame, palette);
    } else if active_theme == Theme::NightMarket {
        draw_night_market_lights(fb, animation_frame, palette);
    }

    // Green header bar with a title.
    Rectangle::new(Point::zero(), Size::new(WIDTH as u32, 40))
        .into_styled(PrimitiveStyle::with_fill(palette.header))
        .draw(fb)
        .ok();
    let header_style = MonoTextStyle::new(&FONT_9X15_BOLD, palette.header_text);
    let header_title = if startup_stage.is_some() {
        "SpotUI"
    } else {
        match app_view {
        AppView::Library => title,
        AppView::Playlists => "Playlists",
        AppView::PlaylistTracks => "Playlist",
        AppView::SearchInput => "Search",
        AppView::SearchResults => "Search Results",
        AppView::SearchHistory => "Recent Searches",
        AppView::Menu => "More",
        AppView::Sound => "Sound",
        AppView::UpNext => "Up Next",
        AppView::NowPlaying => "Now Playing",
        AppView::Appearance => "Appearance",
        AppView::Special => "Appearance 2",
        AppView::Diagnostics => "Diagnostics",
        AppView::Settings => "Settings",
        }
    };

    Text::with_baseline(
        header_title,
        Point::new(6, 12),
        header_style,
        Baseline::Top,
    )
    .draw(fb)
    .ok();

    let playback_status = if startup_stage.is_some() {
        "Starting"
    } else {
        match playback_state {
        PlaybackState::Unknown => "Connecting",
        PlaybackState::Stopped => "Stopped",
        PlaybackState::Loading => "Loading",
        PlaybackState::Playing => "Playing",
        PlaybackState::Paused => "Paused",
        PlaybackState::Error => "Error",
        }
    };
    let playback_status_x =
        (WIDTH as i32 - playback_status.chars().count() as i32 * 9) / 2;
    Text::with_baseline(
        playback_status,
        Point::new(playback_status_x, 12),
        header_style,
        Baseline::Top,
    )
    .draw(fb)
    .ok();

    let battery_label = match battery_percent {
        Some(value) => format!("{}%", value),
        None => "--%".to_string(),
    };
    let battery_x = WIDTH as i32 - 8 - battery_label.chars().count() as i32 * 9;
    Text::with_baseline(
        &battery_label,
        Point::new(battery_x, 12),
        header_style,
        Baseline::Top,
    )
    .draw(fb)
    .ok();

    if let Some(stage) = startup_stage {
        Rectangle::new(
            Point::new(0, 40),
            Size::new(WIDTH as u32, (HEIGHT - 40) as u32),
        )
        .into_styled(PrimitiveStyle::with_fill(palette.background))
        .draw(fb)
        .ok();

        let logo = "SpotUI";
        let logo_x = (WIDTH as i32 - logo.len() as i32 * 9) / 2;
        Text::with_baseline(
            logo,
            Point::new(logo_x, 190),
            MonoTextStyle::new(&FONT_9X15_BOLD, palette.text),
            Baseline::Top,
        )
        .draw(fb)
        .ok();

        let stage_label = stage.label();
        let stage_x =
            (WIDTH as i32 - stage_label.chars().count() as i32 * 9) / 2;
        Text::with_baseline(
            stage_label,
            Point::new(stage_x, 275),
            MonoTextStyle::new(&FONT_9X15_BOLD, palette.text),
            Baseline::Top,
        )
        .draw(fb)
        .ok();

        let detail = stage.detail();
        let detail_x =
            (WIDTH as i32 - detail.chars().count() as i32 * 9) / 2;
        Text::with_baseline(
            detail,
            Point::new(detail_x, 310),
            MonoTextStyle::new(&FONT_9X15, palette.text),
            Baseline::Top,
        )
        .draw(fb)
        .ok();

        Rectangle::new(Point::new(60, 365), Size::new(360, 8))
            .into_styled(PrimitiveStyle::with_fill(
                palette.progress_track,
            ))
            .draw(fb)
            .ok();
        Rectangle::new(
            Point::new(60, 365),
            Size::new(stage.progress_width(), 8),
        )
        .into_styled(PrimitiveStyle::with_fill(palette.progress_fill))
        .draw(fb)
        .ok();

        let elapsed_label = format!("Waiting {}s", startup_elapsed_secs);
        let elapsed_x =
            (WIDTH as i32 - elapsed_label.chars().count() as i32 * 9) / 2;
        Text::with_baseline(
            &elapsed_label,
            Point::new(elapsed_x, 405),
            MonoTextStyle::new(&FONT_9X15, palette.text),
            Baseline::Top,
        )
        .draw(fb)
        .ok();

        let retry_fill = if startup_retry_active {
            palette.header
        } else {
            palette.selected_row
        };
        let retry_text = if startup_retry_active {
            palette.header_text
        } else {
            palette.selected_text
        };
        Rectangle::new(Point::new(110, 455), Size::new(260, 60))
            .into_styled(PrimitiveStyle::with_fill(retry_fill))
            .draw(fb)
            .ok();
        let retry_label = if startup_retry_active {
            "Retry requested"
        } else {
            stage.retry_label()
        };
        let retry_x =
            (WIDTH as i32 - retry_label.chars().count() as i32 * 9) / 2;
        Text::with_baseline(
            retry_label,
            Point::new(retry_x, 477),
            MonoTextStyle::new(&FONT_9X15_BOLD, retry_text),
            Baseline::Top,
        )
        .draw(fb)
        .ok();

        fb.flush().ok();
        return;
    }

    let text_style = MonoTextStyle::new(&FONT_9X15, palette.text);
    let sel_style =
        MonoTextStyle::new(&FONT_9X15_BOLD, palette.selected_text);

    let (list_scroll, list_length, list_end) = match app_view {
        AppView::Library => {
            let end = (scroll + VISIBLE_ROWS).min(items.len());
            (scroll, items.len(), end)
        }
        AppView::Playlists => {
            let end =
                (playlist_scroll + VISIBLE_ROWS).min(playlists.len());
            (playlist_scroll, playlists.len(), end)
        }
        AppView::PlaylistTracks => {
            let end = (playlist_track_scroll + VISIBLE_ROWS)
                .min(playlist_tracks.len());
            (
                playlist_track_scroll,
                playlist_tracks.len(),
                end,
            )
        }
        AppView::SearchResults => {
            let end = (search_scroll + VISIBLE_ROWS)
                .min(search_results.len());
            (search_scroll, search_results.len(), end)
        }
        AppView::SearchHistory => {
            (0, recent_searches.len(), recent_searches.len())
        }
        AppView::UpNext => match up_next_page {
            Some(page) => (
                page.start,
                page.total,
                page.start + page.items.len(),
            ),
            None => (0, 0, 0),
        },
        _ => (0, 0, 0),
    };

    match app_view {
        AppView::Library => {
            for (row, index) in (scroll..list_end).enumerate() {
                let item = &items[index];
                let mut label = item.label();
                let y = 40 + row as i32 * ROW_HEIGHT;
                let is_selected = selected == Some(index);

                label = truncate_label(
                    &label,
                    if is_selected { 50 } else { 52 },
                );

                if is_selected {
                    Rectangle::new(
                        Point::new(0, y),
                        Size::new(
                            WIDTH as u32,
                            ROW_HEIGHT as u32,
                        ),
                    )
                    .into_styled(PrimitiveStyle::with_fill(
                        palette.selected_row,
                    ))
                    .draw(fb)
                    .ok();

                    let selected_label = format!("> {}", label);
                    Text::with_baseline(
                        &selected_label,
                        Point::new(10, y + 22),
                        sel_style,
                        Baseline::Top,
                    )
                    .draw(fb)
                    .ok();
                } else {
                    Text::with_baseline(
                        &label,
                        Point::new(10, y + 22),
                        text_style,
                        Baseline::Top,
                    )
                    .draw(fb)
                    .ok();
                }

                Rectangle::new(
                    Point::new(0, y + ROW_HEIGHT - 1),
                    Size::new(WIDTH as u32, 1),
                )
                .into_styled(PrimitiveStyle::with_fill(
                    palette.border,
                ))
                .draw(fb)
                .ok();
            }
        }
        AppView::Playlists => {
            for (row, index) in
                (playlist_scroll..list_end).enumerate()
            {
                let playlist = &playlists[index];
                let mut label = playlist.label();
                let y = 40 + row as i32 * ROW_HEIGHT;
                let is_selected =
                    playlist_selected == Some(index);

                label = truncate_label(
                    &label,
                    if is_selected { 50 } else { 52 },
                );

                if is_selected {
                    Rectangle::new(
                        Point::new(0, y),
                        Size::new(
                            WIDTH as u32,
                            ROW_HEIGHT as u32,
                        ),
                    )
                    .into_styled(PrimitiveStyle::with_fill(
                        palette.selected_row,
                    ))
                    .draw(fb)
                    .ok();

                    let selected_label = format!("> {}", label);
                    Text::with_baseline(
                        &selected_label,
                        Point::new(10, y + 22),
                        sel_style,
                        Baseline::Top,
                    )
                    .draw(fb)
                    .ok();
                } else {
                    Text::with_baseline(
                        &label,
                        Point::new(10, y + 22),
                        text_style,
                        Baseline::Top,
                    )
                    .draw(fb)
                    .ok();
                }

                Rectangle::new(
                    Point::new(0, y + ROW_HEIGHT - 1),
                    Size::new(WIDTH as u32, 1),
                )
                .into_styled(PrimitiveStyle::with_fill(
                    palette.border,
                ))
                .draw(fb)
                .ok();
            }
        }
        AppView::PlaylistTracks => {
            for (row, index) in
                (playlist_track_scroll..list_end).enumerate()
            {
                let item = &playlist_tracks[index];
                let mut label = item.label();
                let y = 40 + row as i32 * ROW_HEIGHT;
                let is_selected =
                    playlist_track_selected == Some(index);

                label = truncate_label(
                    &label,
                    if is_selected { 50 } else { 52 },
                );

                if is_selected {
                    Rectangle::new(
                        Point::new(0, y),
                        Size::new(
                            WIDTH as u32,
                            ROW_HEIGHT as u32,
                        ),
                    )
                    .into_styled(PrimitiveStyle::with_fill(
                        palette.selected_row,
                    ))
                    .draw(fb)
                    .ok();

                    let selected_label = format!("> {}", label);
                    Text::with_baseline(
                        &selected_label,
                        Point::new(10, y + 22),
                        sel_style,
                        Baseline::Top,
                    )
                    .draw(fb)
                    .ok();
                } else {
                    Text::with_baseline(
                        &label,
                        Point::new(10, y + 22),
                        text_style,
                        Baseline::Top,
                    )
                    .draw(fb)
                    .ok();
                }

                Rectangle::new(
                    Point::new(0, y + ROW_HEIGHT - 1),
                    Size::new(WIDTH as u32, 1),
                )
                .into_styled(PrimitiveStyle::with_fill(
                    palette.border,
                ))
                .draw(fb)
                .ok();
            }
        }
        AppView::SearchResults => {
            for (row, index) in
                (search_scroll..list_end).enumerate()
            {
                let item = &search_results[index];
                let mut label = item.label();
                let y = 40 + row as i32 * ROW_HEIGHT;
                let is_selected = search_selected == Some(index);

                label = truncate_label(
                    &label,
                    if is_selected { 50 } else { 52 },
                );

                if is_selected {
                    Rectangle::new(
                        Point::new(0, y),
                        Size::new(WIDTH as u32, ROW_HEIGHT as u32),
                    )
                    .into_styled(PrimitiveStyle::with_fill(
                        palette.selected_row,
                    ))
                    .draw(fb)
                    .ok();

                    Text::with_baseline(
                        &format!("> {}", label),
                        Point::new(10, y + 22),
                        sel_style,
                        Baseline::Top,
                    )
                    .draw(fb)
                    .ok();
                } else {
                    Text::with_baseline(
                        &label,
                        Point::new(10, y + 22),
                        text_style,
                        Baseline::Top,
                    )
                    .draw(fb)
                    .ok();
                }

                Rectangle::new(
                    Point::new(0, y + ROW_HEIGHT - 1),
                    Size::new(WIDTH as u32, 1),
                )
                .into_styled(PrimitiveStyle::with_fill(
                    palette.border,
                ))
                .draw(fb)
                .ok();
            }
        }
        AppView::SearchHistory => {
            if recent_searches.is_empty() {
                Text::with_baseline(
                    "No recent searches",
                    Point::new(10, 62),
                    text_style,
                    Baseline::Top,
                )
                .draw(fb)
                .ok();
            } else {
                for (row, query) in recent_searches.iter().enumerate() {
                    let y = 40 + row as i32 * ROW_HEIGHT;
                    let label = truncate_label(query, 50);
                    Text::with_baseline(
                        &label,
                        Point::new(10, y + 22),
                        text_style,
                        Baseline::Top,
                    )
                    .draw(fb)
                    .ok();
                    Rectangle::new(
                        Point::new(0, y + ROW_HEIGHT - 1),
                        Size::new(WIDTH as u32, 1),
                    )
                    .into_styled(PrimitiveStyle::with_fill(palette.border))
                    .draw(fb)
                    .ok();
                }

                let clear_y = 40 + recent_searches.len() as i32 * ROW_HEIGHT;
                Text::with_baseline(
                    "Clear History",
                    Point::new(10, clear_y + 22),
                    sel_style,
                    Baseline::Top,
                )
                .draw(fb)
                .ok();
            }
        }
        AppView::UpNext => {
            if let Some(page) = up_next_page {
                for (row, queued) in page.items.iter().enumerate() {
                    let source_items = match &page.source {
                        QueueSource::Liked => items,
                        QueueSource::Playlist(_) => playlist_tracks,
                        QueueSource::Search => search_results,
                    };
                    let track = source_items
                        .get(queued.source_index)
                        .filter(|item| item.id == queued.track_id)
                        .or_else(|| {
                            source_items.iter().find(|item| {
                                item.id == queued.track_id
                            })
                        });
                    let track_label = track
                        .map(TrackItem::label)
                        .unwrap_or_else(|| queued.track_id.clone());
                    let label = truncate_label(
                        &format!(
                            "{}. {}",
                            queued.position + 1,
                            track_label
                        ),
                        48,
                    );
                    let y = 40 + row as i32 * ROW_HEIGHT;
                    let is_current =
                        queued.position == page.current_position;

                    if is_current {
                        Rectangle::new(
                            Point::new(0, y),
                            Size::new(WIDTH as u32, ROW_HEIGHT as u32),
                        )
                        .into_styled(PrimitiveStyle::with_fill(
                            palette.selected_row,
                        ))
                        .draw(fb)
                        .ok();

                        Text::with_baseline(
                            &format!("> {}", label),
                            Point::new(10, y + 22),
                            sel_style,
                            Baseline::Top,
                        )
                        .draw(fb)
                        .ok();
                    } else {
                        Text::with_baseline(
                            &label,
                            Point::new(10, y + 22),
                            text_style,
                            Baseline::Top,
                        )
                        .draw(fb)
                        .ok();
                    }

                    Rectangle::new(
                        Point::new(0, y + ROW_HEIGHT - 1),
                        Size::new(WIDTH as u32, 1),
                    )
                    .into_styled(PrimitiveStyle::with_fill(palette.border))
                    .draw(fb)
                    .ok();
                }
            } else {
                Text::with_baseline(
                    "No active queue",
                    Point::new(10, 62),
                    text_style,
                    Baseline::Top,
                )
                .draw(fb)
                .ok();
            }
        }
        AppView::NowPlaying => {
            draw_now_playing_view(
                fb,
                now_playing,
                playback_position,
                playback_state,
                palette,
            );
        }
        AppView::SearchInput => {
            Rectangle::new(
                Point::new(0, 40),
                Size::new(WIDTH as u32, 540),
            )
            .into_styled(PrimitiveStyle::with_fill(palette.background))
            .draw(fb)
            .ok();

            let shown_query = if search_in_progress {
                "Searching...".to_string()
            } else if search_query.is_empty() {
                "Tap letters to search".to_string()
            } else {
                truncate_label(search_query, 48)
            };

            Text::with_baseline(
                &shown_query,
                Point::new(12, 62),
                MonoTextStyle::new(&FONT_9X15_BOLD, palette.text),
                Baseline::Top,
            )
            .draw(fb)
            .ok();

            for (row_index, row_keys) in
                SEARCH_KEY_ROWS.iter().enumerate()
            {
                let key_count = row_keys.chars().count() as i32;
                let key_width = 48;
                let row_width = key_count * key_width;
                let start_x = (WIDTH as i32 - row_width) / 2;
                let key_y = 105 + row_index as i32 * 105;

                for (column, key) in row_keys.chars().enumerate() {
                    let key_x = start_x + column as i32 * key_width;
                    Rectangle::new(
                        Point::new(key_x + 2, key_y),
                        Size::new((key_width - 4) as u32, 92),
                    )
                    .into_styled(PrimitiveStyle::with_fill(
                        palette.now_playing,
                    ))
                    .draw(fb)
                    .ok();

                    Text::with_baseline(
                        &key.to_string(),
                        Point::new(key_x + 20, key_y + 36),
                        MonoTextStyle::new(
                            &FONT_9X15_BOLD,
                            palette.text,
                        ),
                        Baseline::Top,
                    )
                    .draw(fb)
                    .ok();
                }
            }

            for (x, width, label) in [
                (
                    0,
                    120,
                    if search_query.is_empty() && !recent_searches.is_empty() {
                        "Recent"
                    } else {
                        "Clear"
                    },
                ),
                (120, 120, "Space"),
                (240, 120, "Delete"),
                (
                    360,
                    120,
                    if search_in_progress {
                        "Wait"
                    } else {
                        "Go"
                    },
                ),
            ] {
                Rectangle::new(
                    Point::new(x, 420),
                    Size::new(width, 120),
                )
                .into_styled(PrimitiveStyle::with_fill(
                    palette.now_playing,
                ))
                .draw(fb)
                .ok();

                Text::with_baseline(
                    label,
                    Point::new(x + 12, 468),
                    MonoTextStyle::new(&FONT_9X15_BOLD, palette.text),
                    Baseline::Top,
                )
                .draw(fb)
                .ok();
            }
        }
        _ => {}
    }

    // Header scroll indicators.
    if matches!(
        app_view,
        AppView::Library
            | AppView::Playlists
            | AppView::PlaylistTracks
            | AppView::SearchResults
            | AppView::UpNext
    ) && list_scroll > 0
    {
        Text::with_baseline(
            "^",
            Point::new(WIDTH as i32 - 112, 12),
            header_style,
            Baseline::Top,
        )
        .draw(fb)
        .ok();
    }

    if matches!(
        app_view,
        AppView::Library
            | AppView::Playlists
            | AppView::PlaylistTracks
            | AppView::SearchResults
            | AppView::UpNext
    ) && list_end < list_length
    {
        Text::with_baseline(
            "v",
            Point::new(WIDTH as i32 - 88, 12),
            header_style,
            Baseline::Top,
        )
        .draw(fb)
        .ok();
    }

    // Menu screens replace the visible library area while preserving
    // the header, now-playing strip, and toolbar.
    let visible_menu_labels = match app_view {
        AppView::Library
        | AppView::Playlists
        | AppView::PlaylistTracks
        | AppView::SearchInput
        | AppView::SearchResults
        | AppView::SearchHistory
        | AppView::UpNext
        | AppView::NowPlaying => None,
        AppView::Menu => Some(&MENU_LABELS),
        AppView::Sound => Some(&SOUND_LABELS),
        AppView::Appearance => Some(&APPEARANCE_LABELS),
        AppView::Special => Some(&SPECIAL_LABELS),
        AppView::Diagnostics => Some(&DIAGNOSTICS_LABELS),
        AppView::Settings => Some(&SETTINGS_LABELS),
    };

    if let Some(menu_labels) = visible_menu_labels {
        Rectangle::new(
            Point::new(0, 40),
            Size::new(WIDTH as u32, 540),
        )
        .into_styled(PrimitiveStyle::with_fill(palette.background))
        .draw(fb)
        .ok();

        let menu_style =
            MonoTextStyle::new(&FONT_9X15_BOLD, palette.text);

        for (index, &label) in menu_labels.iter().enumerate() {
            let tile_x = (index % 2) as i32 * 240;
            let tile_y = 40 + (index / 2) as i32 * 180;

            Rectangle::new(
                Point::new(tile_x, tile_y),
                Size::new(240, 180),
            )
            .into_styled(PrimitiveStyle::with_fill(
                palette.now_playing,
            ))
            .draw(fb)
            .ok();

            Rectangle::new(
                Point::new(tile_x + 239, tile_y),
                Size::new(1, 180),
            )
            .into_styled(PrimitiveStyle::with_fill(palette.border))
            .draw(fb)
            .ok();

            Rectangle::new(
                Point::new(tile_x, tile_y + 179),
                Size::new(240, 1),
            )
            .into_styled(PrimitiveStyle::with_fill(palette.border))
            .draw(fb)
            .ok();

            let is_active_theme = match app_view {
                AppView::Appearance => match index {
                    0 => active_theme == Theme::ElKayKay,
                    1 => active_theme == Theme::Tidepool,
                    2 => active_theme == Theme::CitrusGrove,
                    3 => active_theme == Theme::MonochromeStatic,
                    4 => active_theme == Theme::PaperLantern,
                    _ => false,
                },
                AppView::Special => match index {
                    0 => active_theme == Theme::DurandalTerminal,
                    1 => active_theme == Theme::ArcadeBloom,
                    2 => active_theme == Theme::DesertBloom,
                    3 => active_theme == Theme::AlNoor,
                    4 => active_theme == Theme::NightMarket,
                    _ => false,
                },
                AppView::Sound => match index {
                    0 => playback_modes.shuffle,
                    1 => playback_modes.repeat == RepeatMode::Off,
                    2 => playback_modes.repeat == RepeatMode::All,
                    3 => playback_modes.repeat == RepeatMode::One,
                    _ => false,
                },
                AppView::Settings => index == screen_sleep_idx,
                AppView::Library
                | AppView::Playlists
                | AppView::PlaylistTracks
                | AppView::SearchInput
                | AppView::SearchResults
                | AppView::SearchHistory
                | AppView::UpNext
                | AppView::NowPlaying
                | AppView::Menu
                | AppView::Diagnostics => false,
            };

            let display_label =
                if app_view == AppView::Sound && index == 0 {
                    format!(
                        "Shuffle: {}",
                        if playback_modes.shuffle { "On" } else { "Off" }
                    )
                } else if app_view == AppView::Diagnostics {
                    match index {
                        0 => format!(
                            "Wi-Fi: {}",
                            if wifi_has_default_route() {
                                "Connected"
                            } else {
                                "Offline"
                            }
                        ),
                        1 => {
                            let daemon_status = match playback_state {
                                PlaybackState::Unknown => "Reconnecting",
                                PlaybackState::Stopped => "Stopped",
                                PlaybackState::Loading => "Loading",
                                PlaybackState::Playing => "Playing",
                                PlaybackState::Paused => "Paused",
                                PlaybackState::Error => "Error",
                            };

                            format!("Spotify: {}", daemon_status)
                        }
                        2 => {
                            let audio_status = match playback_state {
                                PlaybackState::Unknown
                                | PlaybackState::Error => "Offline",
                                PlaybackState::Playing
                                    if !process_running("aplay") =>
                                {
                                    "Starting"
                                }
                                _ => "Ready",
                            };
                            format!("Audio: {}", audio_status)
                        }
                        3 => {
                            let output = if switch_active(SW_BALANCE) {
                                "4.4 mm"
                            } else if switch_active(SW_HEADSET) {
                                "3.5 mm"
                            } else {
                                "No jack"
                            };
                            format!("Output: {}", output)
                        }
                        4 => {
                            let source = match queue_status
                                .map(|queue| &queue.source)
                            {
                                Some(QueueSource::Liked) => "Liked Songs",
                                Some(QueueSource::Playlist(_)) => "Playlist",
                                Some(QueueSource::Search) => "Search",
                                None => "None",
                            };
                            format!("Queue: {}", source)
                        }
                        5 if diagnostics_refreshed => {
                            "Status Refreshed".to_string()
                        }
                        _ => label.to_string(),
                    }
                } else if is_active_theme {
                    format!("> {}", label)
                } else {
                    label.to_string()
                };
            let label_width =
                display_label.chars().count() as i32 * 9;
            let label_x = tile_x + (240 - label_width) / 2;

            Text::with_baseline(
                &display_label,
                Point::new(label_x, tile_y + 82),
                menu_style,
                Baseline::Top,
            )
            .draw(fb)
            .ok();
        }
    }

    if app_view != AppView::NowPlaying {
        draw_now_playing_strip(
            fb,
            now_playing,
            playback_position,
            playback_state,
            palette,
        );

        // Non-interactive separator immediately above the toolbar.
        let down_strip_y = HEIGHT as i32 - 80;
        Rectangle::new(
            Point::new(0, down_strip_y),
            Size::new(WIDTH as u32, 20),
        )
        .into_styled(PrimitiveStyle::with_fill(palette.separator))
        .draw(fb)
        .ok();
    }

    // Fixed four-button toolbar.
    let toolbar_y = HEIGHT as i32 - 60;
    Rectangle::new(
        Point::new(0, toolbar_y),
        Size::new(WIDTH as u32, 60),
    )
    .into_styled(PrimitiveStyle::with_fill(palette.toolbar))
    .draw(fb)
    .ok();

    for x in [120, 240, 360] {
        Rectangle::new(Point::new(x, toolbar_y), Size::new(1, 60))
            .into_styled(PrimitiveStyle::with_fill(palette.border))
            .draw(fb)
            .ok();
    }

    let exit_label = if exit_armed { "Confirm" } else { "Exit" };
    let brightness_label =
        format!("Bright {}", BRIGHTNESS_LABELS[brightness_idx]);
    let playback_label =
        if playback_state.is_paused() { "Resume" } else { "Pause" };
    let menu_label = match app_view {
        AppView::Library => "More",
        AppView::Playlists
        | AppView::PlaylistTracks
        | AppView::SearchInput
        | AppView::SearchResults
        | AppView::SearchHistory
        | AppView::UpNext
        | AppView::NowPlaying => "Back",
        AppView::Menu
        | AppView::Sound
        | AppView::Appearance
        | AppView::Special
        | AppView::Diagnostics
        | AppView::Settings => "Back",
    };
    let button_style =
        MonoTextStyle::new(&FONT_9X15_BOLD, palette.text);

    Text::with_baseline(
        exit_label,
        Point::new(40, toolbar_y + 22),
        button_style,
        Baseline::Top,
    )
    .draw(fb)
    .ok();

    Text::with_baseline(
        &brightness_label,
        Point::new(128, toolbar_y + 22),
        button_style,
        Baseline::Top,
    )
    .draw(fb)
    .ok();

    Text::with_baseline(
        playback_label,
        Point::new(270, toolbar_y + 22),
        button_style,
        Baseline::Top,
    )
    .draw(fb)
    .ok();

    let menu_label_width = menu_label.chars().count() as i32 * 9;
    let menu_label_x = 360 + (120 - menu_label_width) / 2;

    Text::with_baseline(
        menu_label,
        Point::new(menu_label_x, toolbar_y + 22),
        button_style,
        Baseline::Top,
    )
    .draw(fb)
    .ok();

    fb.flush().ok();
}

/// DIAGNOSTIC: step through every combination of (which frame we write) and
/// (which pan offset we set), each with a distinct solid color, pausing so the
/// operator can see which combination actually lights the panel.
///
/// The fb is 480x1440 = frame0 (rows 0..719) + frame1 (rows 720..1439).
/// Pan offset selects which 720-row window is scanned to the panel.
///
/// Watch the screen and note (from the stderr log) which step shows color.
fn run_diagnostics(fb: &mut Framebuffer) {
    use std::{thread::sleep, time::Duration};

    let pause = Duration::from_secs(4);

    // Distinct colors so each step is unmistakable.
    let red = Rgb565::new(31, 0, 0);
    let green = Rgb565::new(0, 63, 0);
    let blue = Rgb565::new(0, 0, 31);
    let white = Rgb565::new(31, 63, 31);

    eprintln!("[diag] === starting; watch the screen, each step is 4s ===");

    // Step 1: write RED to frame 0, pan to 0,0 (frame 0 should show).
    eprintln!("[diag] step1: RED -> frame0, pan 0,0  (expect RED if frame0+pan0 visible)");
    fb.diag_fill_frame(0, red).ok();
    write_sysfs(FB_PAN_SYSFS, "0,0");
    sleep(pause);

    // Step 2: write GREEN to frame 1, pan to 0,720 (frame 1 should show).
    eprintln!("[diag] step2: GREEN -> frame1, pan 0,720  (expect GREEN if frame1+pan720 visible)");
    fb.diag_fill_frame(1, green).ok();
    write_sysfs(FB_PAN_SYSFS, "0,720");
    sleep(pause);

    // Step 3: write BLUE to frame 0, pan to 0,0 again.
    eprintln!("[diag] step3: BLUE -> frame0, pan 0,0");
    fb.diag_fill_frame(0, blue).ok();
    write_sysfs(FB_PAN_SYSFS, "0,0");
    sleep(pause);

    // Step 4: write WHITE to BOTH frames, pan 0,0 (rules out pan entirely).
    eprintln!("[diag] step4: WHITE -> both frames, pan 0,0  (expect WHITE regardless)");
    fb.diag_fill_frame(0, white).ok();
    fb.diag_fill_frame(1, white).ok();
    write_sysfs(FB_PAN_SYSFS, "0,0");
    sleep(pause);

    // Step 5: WHITE both frames, pan 0,720.
    eprintln!("[diag] step5: WHITE both frames, pan 0,720");
    write_sysfs(FB_PAN_SYSFS, "0,720");
    sleep(pause);

    // Step 6: report what pan reads back as, and try the FBIOPAN ioctl path too.
    eprintln!("[diag] step6: done. Note which steps showed color.");
    eprintln!("[diag] If NO step showed color: writes/panel not reaching display.");
    eprintln!("[diag] If only some: that frame+pan combo is the visible one.");
}


/// Parse a little-endian u16 / i32 from a byte slice.
fn le_u16(b: &[u8]) -> u16 {
    u16::from_le_bytes([b[0], b[1]])
}
fn le_i32(b: &[u8]) -> i32 {
    i32::from_le_bytes([b[0], b[1], b[2], b[3]])
}

fn return_to_hiby() {
    eprintln!("[poc] exit requested; running return_to_hiby.sh");
    let _ = std::process::Command::new("sh")
        .arg("/usr/data/return_to_hiby.sh")
        .spawn();
}

fn main() {
    eprintln!("[poc] starting");

    backlight_on();

    let mut fb = match Framebuffer::open() {
        Ok(f) => f,
        Err(e) => {
            eprintln!("[poc] cannot open {FB_PATH}: {e}");
            return;
        }
    };

    // Wake the panel itself (not just the backlight). Needed when hiby_player
    // is dead and nothing else is holding the panel awake.
    panel_wake(fb.file.as_raw_fd());
    eprintln!("[poc] panel wake requested");

    // DIAGNOSTIC MODE: run with argument "diag" to systematically test which
    // (frame, pan-offset) combination actually appears on the panel. This
    // isolates the double-buffer/pan display bug. Watch the screen during each
    // step and note which one shows the color.
    let args: Vec<String> = std::env::args().collect();
    if args.iter().any(|a| a == "diag") {
        run_diagnostics(&mut fb);
        return;
    }

    // FILL MODE: fill the in-memory buffer with a solid color and use the
    // NORMAL flush() path (same as the real UI). Cycles red/green/blue/white,
    // 5s each, so we can see if the real draw path displays anything at all.
    // This separates "flush path broken" from "text rendering broken".
    if args.iter().any(|a| a == "fill") {
        use std::{thread::sleep, time::Duration};
        let colors = [
            ("RED", Rgb565::new(31, 0, 0)),
            ("GREEN", Rgb565::new(0, 63, 0)),
            ("BLUE", Rgb565::new(0, 0, 31)),
            ("WHITE", Rgb565::new(31, 63, 31)),
        ];
        for (name, color) in colors.iter() {
            eprintln!("[fill] filling {name} via normal flush() path");
            // Fill the in-memory buffer exactly as draw operations would.
            Rectangle::new(Point::zero(), Size::new(WIDTH as u32, HEIGHT as u32))
                .into_styled(PrimitiveStyle::with_fill(*color))
                .draw(&mut fb)
                .ok();
            fb.flush().ok();
            sleep(Duration::from_secs(5));
        }
        eprintln!("[fill] done");
        return;
    }

    // Try to fetch liked songs from the daemon. At cold boot the daemon may
    // not be listening yet (WiFi/daemon still starting), so we DON'T block here
    // -- we show a "connecting" placeholder and retry in the main loop. This is
    // what lets the UI come up immediately after hiby_player is killed, keeping
    // the panel lit (its continuous refresh prevents the ~20s blank) while the
    // daemon comes up in the background.
    eprintln!("[poc] fetching liked songs...");
    let mut items: Vec<TrackItem> = daemon_query("LIKED");
    let mut tracks_loaded = !items.is_empty();
    let mut startup_stage = if tracks_loaded {
        None
    } else if wifi_has_default_route() {
        Some(StartupStage::Spotify)
    } else {
        Some(StartupStage::Wifi)
    };
    if tracks_loaded {
        eprintln!("[poc] loaded {} liked tracks", items.len());
    } else {
        eprintln!("[poc] daemon not ready yet; will retry");
        items.push(TrackItem {
            id: String::new(),
            name: "Connecting...".to_string(),
            artist: String::new(),
        });
    }

    let mut playlists: Vec<PlaylistItem> = Vec::new();
    let mut playlist_selected: Option<usize> = None;
    let mut playlist_scroll: usize = 0;
    let mut playlist_tracks: Vec<TrackItem> = Vec::new();
    let mut playlist_track_selected: Option<usize> = None;
    let mut playlist_track_scroll: usize = 0;
    let mut search_query = String::new();
    let mut search_results: Vec<TrackItem> = Vec::new();
    let mut recent_searches = load_search_history();
    let mut search_selected: Option<usize> = None;
    let mut search_scroll: usize = 0;
    let mut search_in_progress = false;
    let mut search_receiver:
        Option<std::sync::mpsc::Receiver<Vec<TrackItem>>> = None;
    let mut up_next_page: Option<QueuePage> = None;
    let mut up_next_offset: usize = 0;

    let mut brightness_idx: usize = load_brightness_idx();
    let mut screen_sleep_idx: usize = load_screen_sleep_idx();
    let mut selected: Option<usize> = None;
    let mut pending_queue_selection:
        Option<PendingQueueSelection> = None;
    let mut pending_load_command: Option<PendingLoadCommand> = None;
    let mut next_load_request_id = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|elapsed| elapsed.as_nanos() as u64)
        .unwrap_or(1);
    let mut scroll: usize = 0;
    let mut playback_state = PlaybackState::Unknown;
    let mut playback_modes =
        daemon_playback_modes().unwrap_or_default();
    let mut active_queue_status =
        daemon_queue_status().unwrap_or(None);
    let mut diagnostics_refreshed_at:
        Option<std::time::Instant> = None;
    let mut now_playing: Option<NowPlaying> = None;
    let mut playback_position: Option<u32> = None;
    let mut theme = load_theme();
    let mut palette = theme.palette();
    let mut static_seed: u32 = 0x5a17_c9e3;
    let mut animation_frame: u32 = 0;
    eprintln!("[poc] startup theme -> {}", theme.key());
    let mut app_view = AppView::Library;
    let mut now_playing_return_view = AppView::Library;
    let mut up_next_return_view = AppView::Sound;
    let mut exit_armed = false;
    let title = "Liked Songs";
    let mut battery_percent = read_battery_percent();
    let mut storage_free_mb = read_storage_free_mb();
    let mut memory_available_mb = read_memory_available_mb();
    draw_list(
        &mut fb,
        &items,
        &playlists,
        &playlist_tracks,
        &search_results,
        &recent_searches,
        &search_query,
        search_in_progress,
        up_next_page.as_ref(),
        scroll,
        playlist_scroll,
        playlist_track_scroll,
        search_scroll,
        selected,
        playlist_selected,
        playlist_track_selected,
        search_selected,
        title,
        battery_percent,
        storage_free_mb,
        memory_available_mb,
        brightness_idx,
        screen_sleep_idx,
        playback_state,
        startup_stage,
        0,
        false,
        playback_modes,
        active_queue_status.as_ref(),
        false,
        now_playing.as_ref(),
        playback_position,
        &palette,
        theme,
        static_seed,
        animation_frame,
        app_view,
        exit_armed,
    );
    eprintln!("[poc] drew initial list");

    // Open touch input (non-blocking, so we can redraw on a heartbeat even
    // when there's no touch -- the panel needs continuous re-flushing to stay
    // lit).
    let input = match OpenOptions::new().read(true).open(INPUT_PATH) {
        Ok(f) => f,
        Err(e) => {
            eprintln!("[poc] cannot open {INPUT_PATH}: {e}");
            return;
        }
    };
    let input_fd = input.as_raw_fd();
    // Set non-blocking.
    unsafe {
        let flags = libc::fcntl(input_fd, libc::F_GETFL, 0);
        libc::fcntl(input_fd, libc::F_SETFL, flags | libc::O_NONBLOCK);
    }
    let mut input = input;

    // Open volume buttons (event2), also non-blocking. Optional -- if it fails
    // we just carry on without hardware volume.
    let mut vol_input = match OpenOptions::new().read(true).open(VOL_INPUT_PATH) {
        Ok(f) => {
            let fd = f.as_raw_fd();
            unsafe {
                let flags = libc::fcntl(fd, libc::F_GETFL, 0);
                libc::fcntl(fd, libc::F_SETFL, flags | libc::O_NONBLOCK);
            }
            Some(f)
        }
        Err(e) => {
            eprintln!("[poc] cannot open {VOL_INPUT_PATH} (no hw volume): {e}");
            None
        }
    };

    // Open the physical power button (event0), also non-blocking. SpotUI only
    // uses it to wake an already sleeping screen; awake presses are ignored.
    let mut power_input =
        match OpenOptions::new().read(true).open(POWER_INPUT_PATH) {
            Ok(f) => {
                let fd = f.as_raw_fd();
                unsafe {
                    let flags = libc::fcntl(fd, libc::F_GETFL, 0);
                    libc::fcntl(fd, libc::F_SETFL, flags | libc::O_NONBLOCK);
                }
                Some(f)
            }
            Err(e) => {
                eprintln!(
                    "[poc] cannot open {POWER_INPUT_PATH} (no power wake): {e}"
                );
                None
            }
        };

    let mut ev = [0u8; EVENT_SIZE];
    let mut cur_x: i32 = 0;
    let mut cur_y: i32 = 0;
    let mut touch_down = false;

    // Keep-alive interval (ms) is configurable via the 2nd arg, so we can sweep
    // it to find the slowest refresh that keeps the panel lit while leaving CPU
    // for audio. Default 200ms (5Hz). Usage: spotui-ui-poc [keepalive_ms]
    let keepalive_ms: u64 = args
        .iter()
        .skip(1)
        .find_map(|a| a.parse::<u64>().ok())
        .unwrap_or(200);
    eprintln!("[poc] keepalive interval = {keepalive_ms}ms");

    eprintln!("[poc] entering input loop; tap the screen");

    // Row hit-testing must match the draw layout: rows start at y=40.
    const LIST_TOP: i32 = 40;

    let mut dirty = false; // does the buffer content need re-rendering?
    let mut last_flush = std::time::Instant::now();

    // Auto-select the output port once at startup based on which jack is
    // plugged, then re-check periodically so plug/unplug during use is handled.
    let initial_port = auto_select_output();
    match initial_port {
        Some(p) => eprintln!("[poc] output port set to {p} (jack detected)"),
        None => eprintln!("[poc] no jack detected at startup"),
    }
    let mut last_jack_check = std::time::Instant::now();
    let mut last_port: Option<u8> = initial_port;
    let mut last_liked_retry = std::time::Instant::now();
    let mut startup_stage_started_at = std::time::Instant::now();
    let mut startup_retry_at: Option<std::time::Instant> = None;
    let mut last_status_check = std::time::Instant::now();
    let mut consecutive_status_failures: u8 = 0;
    let mut last_battery_check = std::time::Instant::now();
    let mut last_storage_check = std::time::Instant::now();
    let mut last_memory_check = std::time::Instant::now();
    let mut last_static_refresh = std::time::Instant::now();
    let mut last_user_input = std::time::Instant::now();
    let mut screen_asleep = false;
    let startup_time = std::time::Instant::now();
    let mut startup_brightness_applied = false;
    let mut volume_popup:
        Option<(u8, std::time::Instant)> = None;
    let mut now_playing_dirty = false;

    loop {
        // Drain any available input events (non-blocking).
        loop {
            match input.read_exact(&mut ev) {
                Ok(()) => {
                    let etype = le_u16(&ev[8..10]);
                    let code = le_u16(&ev[10..12]);
                    let value = le_i32(&ev[12..16]);

                    match etype {
                        EV_ABS => {
                            if code == ABS_MT_POSITION_X {
                                cur_x = value;
                            } else if code == ABS_MT_POSITION_Y {
                                cur_y = value;
                            }
                        }
                        EV_KEY => {
                            if code == BTN_TOUCH {
                                touch_down = value == 1;
                            }
                        }
                        EV_SYN => {
                            if code == SYN_REPORT && touch_down {
                                if screen_asleep {
                                    screen_asleep = false;
                                    last_user_input =
                                        std::time::Instant::now();
                                    panel_wake(fb.file.as_raw_fd());
                                    apply_brightness(brightness_idx);
                                    fb.flush().ok();
                                    last_flush =
                                        std::time::Instant::now();
                                    touch_down = false;
                                    eprintln!(
                                        "[poc] screen woke from touch"
                                    );
                                    continue;
                                }

                                last_user_input = std::time::Instant::now();
                                // Header = page up; strip above toolbar = page down.
                                // The bottom 60px are four equal-width controls.
                                const NOW_PLAYING_TOP: i32 = 580;
                                const SEEK_HIT_TOP: i32 = 620;
                                const PROGRESS_LEFT: i32 = 10;
                                const PROGRESS_RIGHT: i32 = 470;
                                const DOWN_STRIP_TOP: i32 = 640;
                                const TOOLBAR_TOP: i32 = 660;
                                const BUTTON_WIDTH: i32 = 120;

                                if let Some(stage) = startup_stage {
                                    if cur_y >= 435 && cur_y < 535 {
                                        request_startup_recovery(stage);
                                        if stage == StartupStage::Library {
                                            last_liked_retry =
                                                std::time::Instant::now()
                                                    .checked_sub(
                                                        std::time::Duration::from_secs(3),
                                                    )
                                                    .unwrap_or_else(
                                                        std::time::Instant::now,
                                                    );
                                        }
                                        startup_stage_started_at =
                                            std::time::Instant::now();
                                        startup_retry_at =
                                            Some(std::time::Instant::now());
                                        dirty = true;
                                        touch_down = false;
                                        eprintln!(
                                            "[poc] startup retry -> {:?}",
                                            stage
                                        );
                                        continue;
                                    }
                                }

                                if cur_y < LIST_TOP {
                                    exit_armed = false;

                                    let page =
                                        VISIBLE_ROWS.saturating_sub(1).max(1);

                                    match app_view {
                                        AppView::Library => {
                                            let max_scroll = items
                                                .len()
                                                .saturating_sub(VISIBLE_ROWS);

                                            if cur_x < WIDTH as i32 - 120 {
                                                scroll =
                                                    scroll.saturating_sub(page);
                                            } else {
                                                scroll = (scroll + page)
                                                    .min(max_scroll);
                                            }

                                            dirty = true;
                                            eprintln!(
                                                "[poc] liked scroll -> {scroll}"
                                            );
                                        }
                                        AppView::Playlists => {
                                            let max_scroll = playlists
                                                .len()
                                                .saturating_sub(VISIBLE_ROWS);

                                            if cur_x < WIDTH as i32 - 120 {
                                                playlist_scroll =
                                                    playlist_scroll
                                                        .saturating_sub(page);
                                            } else {
                                                playlist_scroll =
                                                    (playlist_scroll + page)
                                                        .min(max_scroll);
                                            }

                                            dirty = true;
                                            eprintln!(
                                                "[poc] playlist scroll -> {}",
                                                playlist_scroll
                                            );
                                        }
                                        AppView::PlaylistTracks => {
                                            let max_scroll = playlist_tracks
                                                .len()
                                                .saturating_sub(VISIBLE_ROWS);

                                            if cur_x < WIDTH as i32 - 120 {
                                                playlist_track_scroll =
                                                    playlist_track_scroll
                                                        .saturating_sub(page);
                                            } else {
                                                playlist_track_scroll =
                                                    (playlist_track_scroll
                                                        + page)
                                                        .min(max_scroll);
                                            }

                                            dirty = true;
                                            eprintln!(
                                                "[poc] playlist track scroll -> {}",
                                                playlist_track_scroll
                                            );
                                        }
                                        AppView::SearchResults => {
                                            let max_scroll = search_results
                                                .len()
                                                .saturating_sub(VISIBLE_ROWS);

                                            if cur_x < WIDTH as i32 - 120 {
                                                search_scroll =
                                                    search_scroll
                                                        .saturating_sub(page);
                                            } else {
                                                search_scroll =
                                                    (search_scroll + page)
                                                        .min(max_scroll);
                                            }

                                            dirty = true;
                                            eprintln!(
                                                "[poc] search scroll -> {}",
                                                search_scroll
                                            );
                                        }
                                        AppView::UpNext => {
                                            let total = up_next_page
                                                .as_ref()
                                                .map(|page| page.total)
                                                .unwrap_or(0);
                                            let max_offset = total
                                                .saturating_sub(1)
                                                / VISIBLE_ROWS
                                                * VISIBLE_ROWS;

                                            if cur_x < WIDTH as i32 - 120 {
                                                up_next_offset = up_next_offset
                                                    .saturating_sub(VISIBLE_ROWS);
                                            } else {
                                                up_next_offset =
                                                    (up_next_offset + VISIBLE_ROWS)
                                                        .min(max_offset);
                                            }

                                            if let Some(updated) =
                                                daemon_queue_page(up_next_offset)
                                            {
                                                up_next_page = updated;
                                                dirty = true;
                                            }
                                            eprintln!(
                                                "[poc] up next offset -> {}",
                                                up_next_offset
                                            );
                                        }
                                        _ => {}
                                    }
                                } else if cur_y >= TOOLBAR_TOP {
                                    let safe_x = cur_x.max(0).min(WIDTH as i32 - 1);
                                    let button = safe_x / BUTTON_WIDTH;

                                    match button {
                                        0 => {
                                            if exit_armed {
                                                eprintln!("[poc] toolbar exit confirmed");
                                                return_to_hiby();
                                                return;
                                            } else {
                                                exit_armed = true;
                                                dirty = true;
                                                eprintln!("[poc] toolbar exit armed");
                                            }
                                        }
                                        1 => {
                                            exit_armed = false;
                                            brightness_idx =
                                                (brightness_idx + 1) % BRIGHTNESS_LEVELS.len();
                                            apply_brightness(brightness_idx);
                                            save_brightness_idx(brightness_idx);
                                            dirty = true;
                                            eprintln!(
                                                "[poc] toolbar brightness -> {}",
                                                BRIGHTNESS_LABELS[brightness_idx]
                                            );
                                        }
                                        2 => {
                                            exit_armed = false;
                                            if playback_state.is_paused() {
                                                daemon_send("PLAY");
                                                eprintln!("[poc] toolbar resume");
                                            } else {
                                                daemon_send("PAUSE");
                                                eprintln!("[poc] toolbar pause");
                                            }
                                            dirty = true;
                                        }
                                        3 => {
                                            exit_armed = false;
                                            if app_view == AppView::SearchInput
                                                && search_in_progress
                                            {
                                                search_in_progress = false;
                                                search_receiver = None;
                                                eprintln!(
                                                    "[poc] search cancelled"
                                                );
                                            }
                                            app_view = match app_view {
                                                AppView::Library => AppView::Menu,
                                                AppView::Playlists => AppView::Menu,
                                                AppView::PlaylistTracks => {
                                                    AppView::Playlists
                                                }
                                                AppView::SearchInput => {
                                                    AppView::Menu
                                                }
                                                AppView::SearchResults => {
                                                    AppView::SearchInput
                                                }
                                                AppView::SearchHistory => {
                                                    AppView::SearchInput
                                                }
                                                AppView::Menu => AppView::Library,
                                                AppView::Sound => AppView::Menu,
                                                AppView::UpNext => up_next_return_view,
                                                AppView::NowPlaying => {
                                                    now_playing_return_view
                                                }
                                                AppView::Appearance => AppView::Menu,
                                                AppView::Special => AppView::Appearance,
                                                AppView::Diagnostics => AppView::Menu,
                                                AppView::Settings => AppView::Menu,
                                            };
                                            dirty = true;
                                            eprintln!(
                                                "[poc] app view -> {:?}",
                                                app_view
                                            );
                                        }
                                        _ => {}
                                    }
                                } else if app_view == AppView::NowPlaying {
                                    exit_armed = false;

                                    if cur_y >= 275
                                        && cur_y < 350
                                        && cur_x >= 20
                                        && cur_x <= 460
                                    {
                                        if let Some(item) = now_playing.as_ref() {
                                            if item.duration_ms > 0 {
                                                let target_ms = (
                                                    (cur_x - 20) as u64
                                                        * item.duration_ms as u64
                                                        / 440
                                                ) as u32;
                                                daemon_send(&format!(
                                                    "SEEK {}",
                                                    target_ms
                                                ));
                                                playback_position = Some(target_ms);
                                                dirty = true;
                                                eprintln!(
                                                    "[poc] now-playing view seek -> {} ms",
                                                    target_ms
                                                );
                                            }
                                        }
                                    } else if cur_y >= 390 && cur_y < 490 {
                                        if cur_x < 160 || cur_x >= 320 {
                                            let command = if cur_x < 160 {
                                                "PREVIOUS"
                                            } else {
                                                "NEXT"
                                            };
                                            let loaded = daemon_request(command)
                                                .map(|reply| {
                                                    reply.starts_with("OK loading ")
                                                })
                                                .unwrap_or(false);
                                            if loaded {
                                                playback_state = PlaybackState::Loading;
                                                playback_position = Some(0);
                                                dirty = true;
                                                eprintln!(
                                                    "[poc] now-playing view {} requested",
                                                    command.to_lowercase()
                                                );
                                            }
                                        } else {
                                            if playback_state.is_paused() {
                                                daemon_send("PLAY");
                                                eprintln!("[poc] now-playing view resume");
                                            } else {
                                                daemon_send("PAUSE");
                                                eprintln!("[poc] now-playing view pause");
                                            }
                                            dirty = true;
                                        }
                                    } else if cur_y >= 520 && cur_y < 640 {
                                        let first_page = daemon_queue_page(0).flatten();
                                        up_next_offset = first_page
                                            .as_ref()
                                            .map(|page| {
                                                page.current_position
                                                    / VISIBLE_ROWS
                                                    * VISIBLE_ROWS
                                            })
                                            .unwrap_or(0);
                                        up_next_page = if up_next_offset > 0 {
                                            daemon_queue_page(up_next_offset).flatten()
                                        } else {
                                            first_page
                                        };
                                        up_next_return_view = AppView::NowPlaying;
                                        app_view = AppView::UpNext;
                                        dirty = true;
                                        eprintln!("[poc] app view -> UpNext");
                                    }
                                } else if cur_y >= DOWN_STRIP_TOP {
                                    // Separator area intentionally does nothing.
                                    exit_armed = false;
                                } else if cur_y >= NOW_PLAYING_TOP {
                                    exit_armed = false;

                                    if cur_y < SEEK_HIT_TOP
                                        && now_playing.is_some()
                                        && (cur_x < 80 || cur_x >= 400)
                                    {
                                        let command =
                                            if cur_x < 80 { "PREVIOUS" } else { "NEXT" };
                                        let loaded = daemon_request(command)
                                            .map(|reply| {
                                                reply.starts_with("OK loading ")
                                            })
                                            .unwrap_or(false);

                                        if loaded {
                                            playback_state =
                                                PlaybackState::Loading;
                                            playback_position = Some(0);
                                            dirty = true;
                                            eprintln!(
                                                "[poc] now-playing {} requested",
                                                command.to_lowercase()
                                            );
                                        } else {
                                            eprintln!(
                                                "[poc] now-playing {} reached queue boundary",
                                                command.to_lowercase()
                                            );
                                        }
                                    } else if cur_y >= SEEK_HIT_TOP
                                        && cur_x >= PROGRESS_LEFT
                                        && cur_x <= PROGRESS_RIGHT
                                    {
                                        if let Some(item) = now_playing.as_ref() {
                                            if item.duration_ms > 0 {
                                                let progress_width =
                                                    (PROGRESS_RIGHT
                                                        - PROGRESS_LEFT)
                                                        as u64;
                                                let progress_offset =
                                                    (cur_x - PROGRESS_LEFT)
                                                        as u64;
                                                let target_ms = (
                                                    progress_offset
                                                        * item.duration_ms
                                                            as u64
                                                        / progress_width
                                                ) as u32;

                                                daemon_send(&format!(
                                                    "SEEK {}",
                                                    target_ms
                                                ));

                                                // Update immediately for visual
                                                // feedback; daemon polling will
                                                // soon replace this with the
                                                // actual position.
                                                playback_position =
                                                    Some(target_ms);
                                                now_playing_dirty = true;

                                                eprintln!(
                                                    "[poc] progress seek -> {} ms",
                                                    target_ms
                                                );
                                            }
                                        }
                                    } else if cur_y < SEEK_HIT_TOP
                                        && cur_x >= 80
                                        && cur_x < 400
                                        && now_playing.is_some()
                                    {
                                        now_playing_return_view = app_view;
                                        app_view = AppView::NowPlaying;
                                        dirty = true;
                                        eprintln!("[poc] app view -> NowPlaying");
                                    }
                                } else if app_view == AppView::SearchInput {
                                    exit_armed = false;

                                    if search_in_progress {
                                        // Drain every touch report queued while
                                        // the request is running. None may be
                                        // reinterpreted as a result-row tap.
                                        touch_down = false;
                                        continue;
                                    }

                                    if cur_y >= 105 && cur_y < 407 {
                                        let row_index =
                                            ((cur_y - 105) / 105) as usize;

                                        if let Some(row_keys) =
                                            SEARCH_KEY_ROWS.get(row_index)
                                        {
                                            let key_count =
                                                row_keys.chars().count() as i32;
                                            let key_width = 48;
                                            let row_width =
                                                key_count * key_width;
                                            let start_x =
                                                (WIDTH as i32 - row_width) / 2;
                                            let relative_x = cur_x - start_x;

                                            if relative_x >= 0 {
                                                let column =
                                                    (relative_x / key_width)
                                                        as usize;

                                                if let Some(key) =
                                                    row_keys.chars().nth(column)
                                                {
                                                    if search_query
                                                        .chars()
                                                        .count()
                                                        < 48
                                                    {
                                                        search_query.push(
                                                            key.to_ascii_lowercase(),
                                                        );
                                                        dirty = true;
                                                    }
                                                }
                                            }
                                        }
                                    } else if cur_y >= 420
                                        && cur_y < 540
                                    {
                                        let safe_x = cur_x
                                            .max(0)
                                            .min(WIDTH as i32 - 1);

                                        match safe_x / 120 {
                                            0 => {
                                                if search_query.is_empty()
                                                    && !recent_searches.is_empty()
                                                {
                                                    app_view =
                                                        AppView::SearchHistory;
                                                    eprintln!(
                                                        "[poc] app view -> SearchHistory"
                                                    );
                                                } else {
                                                    search_query.clear();
                                                }
                                                dirty = true;
                                            }
                                            1 => {
                                                if !search_query.is_empty()
                                                    && !search_query
                                                        .ends_with(' ')
                                                    && search_query
                                                        .chars()
                                                        .count()
                                                        < 48
                                                {
                                                    search_query.push(' ');
                                                    dirty = true;
                                                }
                                            }
                                            2 => {
                                                search_query.pop();
                                                dirty = true;
                                            }
                                            3 => {
                                                let query =
                                                    search_query.trim();

                                                if !query.is_empty() {
                                                    let query =
                                                        query.to_string();
                                                    eprintln!(
                                                        "[poc] searching -> {}",
                                                        query
                                                    );
                                                    let (sender, receiver) =
                                                        std::sync::mpsc::channel();
                                                    std::thread::spawn(move || {
                                                        let fetched = daemon_query(
                                                            &format!(
                                                                "SEARCH {}",
                                                                query
                                                            ),
                                                        );
                                                        let _ = sender.send(fetched);
                                                    });
                                                    search_receiver =
                                                        Some(receiver);
                                                    search_in_progress = true;
                                                    dirty = true;
                                                }
                                            }
                                            _ => {}
                                        }
                                    }
                                } else if app_view == AppView::SearchHistory {
                                    exit_armed = false;
                                    let row = ((cur_y - LIST_TOP) / ROW_HEIGHT)
                                        .max(0) as usize;

                                    if let Some(query) =
                                        recent_searches.get(row).cloned()
                                    {
                                        search_query = query.clone();
                                        eprintln!(
                                            "[poc] recent search -> {}",
                                            query
                                        );
                                        let (sender, receiver) =
                                            std::sync::mpsc::channel();
                                        std::thread::spawn(move || {
                                            let fetched = daemon_query(
                                                &format!("SEARCH {}", query),
                                            );
                                            let _ = sender.send(fetched);
                                        });
                                        search_receiver = Some(receiver);
                                        search_in_progress = true;
                                        app_view = AppView::SearchInput;
                                        dirty = true;
                                    } else if row == recent_searches.len()
                                        && !recent_searches.is_empty()
                                    {
                                        recent_searches.clear();
                                        save_search_history(&recent_searches);
                                        app_view = AppView::SearchInput;
                                        dirty = true;
                                        eprintln!("[poc] search history cleared");
                                    }
                                } else if matches!(
                                    app_view,
                                    AppView::Menu
                                        | AppView::Sound
                                        | AppView::Appearance
                                        | AppView::Special
                                        | AppView::Diagnostics
                                        | AppView::Settings
                                ) {
                                    exit_armed = false;

                                    let safe_x =
                                        cur_x.max(0).min(WIDTH as i32 - 1);
                                    let column = (safe_x / 240) as usize;
                                    let row =
                                        ((cur_y - LIST_TOP) / 180) as usize;
                                    let menu_index = row * 2 + column;

                                    let menu_labels = match app_view {
                                        AppView::Menu => &MENU_LABELS,
                                        AppView::Sound => &SOUND_LABELS,
                                        AppView::Appearance => &APPEARANCE_LABELS,
                                        AppView::Special => &SPECIAL_LABELS,
                                        AppView::Diagnostics => &DIAGNOSTICS_LABELS,
                                        AppView::Settings => &SETTINGS_LABELS,
                                        AppView::Library
                                        | AppView::Playlists
                                        | AppView::PlaylistTracks
                                        | AppView::SearchInput
                                        | AppView::SearchResults
                                        | AppView::SearchHistory
                                        | AppView::UpNext
                                        | AppView::NowPlaying => &MENU_LABELS,
                                        };

                                    if let Some(label) =
                                        menu_labels.get(menu_index)
                                    {
                                        match app_view {
                                            AppView::Menu => {
                                                match menu_index {
                                                    0 => {
                                                        app_view =
                                                            AppView::SearchInput;
                                                        dirty = true;
                                                        eprintln!(
                                                            "[poc] app view -> {:?}",
                                                            app_view
                                                        );
                                                    }
                                                    2 => {
                                                        let fetched =
                                                            daemon_playlist_query(
                                                                "PLAYLISTS",
                                                            );

                                                        playlists =
                                                            if fetched.is_empty() {
                                                                vec![PlaylistItem {
                                                                    id: String::new(),
                                                                    name: "No playlists found"
                                                                        .to_string(),
                                                                    owner: String::new(),
                                                                }]
                                                            } else {
                                                                fetched
                                                            };

                                                        playlist_scroll = 0;
                                                        playlist_selected = None;
                                                        app_view =
                                                            AppView::Playlists;
                                                        dirty = true;

                                                        eprintln!(
                                                            "[poc] loaded {} playlists",
                                                            playlists.len()
                                                        );
                                                    }
                                                    1 => {
                                                        if let Some(updated) =
                                                            daemon_playback_modes()
                                                        {
                                                            playback_modes = updated;
                                                        }
                                                        app_view = AppView::Sound;
                                                        dirty = true;
                                                        eprintln!(
                                                            "[poc] app view -> {:?}",
                                                            app_view
                                                        );
                                                    }
                                                    3 => {
                                                        app_view =
                                                            AppView::Appearance;
                                                        dirty = true;
                                                        eprintln!(
                                                            "[poc] app view -> {:?}",
                                                            app_view
                                                        );
                                                    }
                                                    5 => {
                                                        active_queue_status =
                                                            daemon_queue_status()
                                                                .unwrap_or(None);
                                                        app_view =
                                                            AppView::Diagnostics;
                                                        dirty = true;
                                                        eprintln!(
                                                            "[poc] app view -> {:?}",
                                                            app_view
                                                        );
                                                    }
                                                    4 => {
                                                        app_view =
                                                            AppView::Settings;
                                                        dirty = true;
                                                        eprintln!(
                                                            "[poc] app view -> {:?}",
                                                            app_view
                                                        );
                                                    }
                                                    _ => {
                                                        eprintln!(
                                                            "[poc] menu placeholder -> {}",
                                                            label
                                                        );
                                                    }
                                                }
                                            }
                                            AppView::Appearance => {
                                                if menu_index == 5 {
                                                    app_view = AppView::Special;
                                                    dirty = true;
                                                    eprintln!(
                                                        "[poc] app view -> {:?}",
                                                        app_view
                                                    );
                                                } else {
                                                    let updated_theme =
                                                        match menu_index {
                                                            0 => Some(
                                                                Theme::ElKayKay,
                                                            ),
                                                            1 => Some(
                                                                Theme::Tidepool,
                                                            ),
                                                            2 => Some(
                                                                Theme::CitrusGrove,
                                                            ),
                                                            3 => Some(
                                                                Theme::MonochromeStatic,
                                                            ),
                                                            4 => Some(
                                                                Theme::PaperLantern,
                                                            ),
                                                            _ => None,
                                                        };

                                                    if let Some(updated) =
                                                        updated_theme
                                                    {
                                                        theme = updated;
                                                        palette =
                                                            theme.palette();
                                                        save_theme(theme);
                                                        dirty = true;
                                                        eprintln!(
                                                            "[poc] theme -> {}",
                                                            label
                                                        );
                                                    }
                                                }
                                            }
                                            AppView::Sound => {
                                                let command = match menu_index {
                                                    0 => Some(format!(
                                                        "SET_SHUFFLE {}",
                                                        if playback_modes.shuffle {
                                                            "OFF"
                                                        } else {
                                                            "ON"
                                                        }
                                                    )),
                                                    1 => Some(
                                                        "SET_REPEAT OFF".to_string(),
                                                    ),
                                                    2 => Some(
                                                        "SET_REPEAT ALL".to_string(),
                                                    ),
                                                    3 => Some(
                                                        "SET_REPEAT ONE".to_string(),
                                                    ),
                                                    _ => None,
                                                };

                                                if menu_index == 5 {
                                                    app_view = AppView::Menu;
                                                    dirty = true;
                                                } else if menu_index == 4 {
                                                    up_next_return_view = AppView::Sound;
                                                    let first_page =
                                                        daemon_queue_page(0)
                                                            .flatten();
                                                    up_next_offset = first_page
                                                        .as_ref()
                                                        .map(|page| {
                                                            page.current_position
                                                                / VISIBLE_ROWS
                                                                * VISIBLE_ROWS
                                                        })
                                                        .unwrap_or(0);
                                                    up_next_page =
                                                        if up_next_offset > 0 {
                                                            daemon_queue_page(
                                                                up_next_offset,
                                                            )
                                                            .flatten()
                                                        } else {
                                                            first_page
                                                        };
                                                    app_view = AppView::UpNext;
                                                    dirty = true;
                                                    eprintln!(
                                                        "[poc] app view -> {:?}",
                                                        app_view
                                                    );
                                                } else if let Some(command) = command {
                                                    if daemon_request(&command)
                                                        .map(|reply| {
                                                            reply.starts_with("OK ")
                                                        })
                                                        .unwrap_or(false)
                                                    {
                                                        if let Some(updated) =
                                                            daemon_playback_modes()
                                                        {
                                                            playback_modes = updated;
                                                            dirty = true;
                                                            eprintln!(
                                                                "[poc] playback modes -> shuffle={} repeat={:?}",
                                                                playback_modes.shuffle,
                                                                playback_modes.repeat
                                                            );
                                                        }
                                                    }
                                                }
                                            }
                                            AppView::Special => {
                                                if menu_index == 5 {
                                                    app_view =
                                                        AppView::Appearance;
                                                    dirty = true;
                                                    eprintln!(
                                                        "[poc] app view -> {:?}",
                                                        app_view
                                                    );
                                                } else {
                                                    let updated_theme =
                                                        match menu_index {
                                                            0 => Some((
                                                                Theme::DurandalTerminal,
                                                                "Durandal Terminal",
                                                            )),
                                                            1 => Some((
                                                                Theme::ArcadeBloom,
                                                                "Arcade Bloom",
                                                            )),
                                                            2 => Some((
                                                                Theme::DesertBloom,
                                                                "Desert Bloom",
                                                            )),
                                                            3 => Some((
                                                                Theme::AlNoor,
                                                                "Al Noor",
                                                            )),
                                                            4 => Some((
                                                                Theme::NightMarket,
                                                                "Night Market",
                                                            )),
                                                            _ => None,
                                                        };

                                                    if let Some((
                                                        updated,
                                                        theme_name,
                                                    )) = updated_theme
                                                    {
                                                        theme = updated;
                                                        palette =
                                                            theme.palette();
                                                        save_theme(theme);
                                                        dirty = true;
                                                        eprintln!(
                                                            "[poc] theme -> {}",
                                                            theme_name
                                                        );
                                                    }
                                                }
                                            }
                                            AppView::Diagnostics => {
                                                if menu_index == 5 {
                                                    if let Some(updated_state) =
                                                        daemon_playback_state()
                                                    {
                                                        playback_state =
                                                            updated_state;
                                                    }
                                                    active_queue_status =
                                                        daemon_queue_status()
                                                            .unwrap_or(None);
                                                    diagnostics_refreshed_at =
                                                        Some(std::time::Instant::now());
                                                    dirty = true;
                                                    eprintln!(
                                                        "[poc] diagnostics refreshed"
                                                    );
                                                } else {
                                                    eprintln!(
                                                        "[poc] diagnostics status -> {}",
                                                        label
                                                    );
                                                }
                                            }
                                            AppView::Settings => {
                                                if menu_index == 5 {
                                                    app_view = AppView::Menu;
                                                } else if menu_index
                                                    < SCREEN_SLEEP_TIMEOUTS.len()
                                                {
                                                    screen_sleep_idx = menu_index;
                                                    save_screen_sleep_idx(
                                                        screen_sleep_idx,
                                                    );
                                                    last_user_input =
                                                        std::time::Instant::now();
                                                    eprintln!(
                                                        "[poc] screen sleep setting -> {}",
                                                        SETTINGS_LABELS[
                                                            screen_sleep_idx
                                                        ]
                                                    );
                                                }
                                                dirty = true;
                                            }
                                            AppView::Library
                                            | AppView::Playlists
                                            | AppView::PlaylistTracks
                                            | AppView::SearchInput
                                            | AppView::SearchResults
                                            | AppView::SearchHistory
                                            | AppView::UpNext
                                            | AppView::NowPlaying => {}
                                        }
                                    }
                                } else if app_view
                                    == AppView::SearchResults
                                {
                                    let rel = cur_y - LIST_TOP;
                                    let visible_row =
                                        (rel / ROW_HEIGHT) as usize;
                                    let index =
                                        search_scroll + visible_row;

                                    if index < search_results.len() {
                                        search_selected = Some(index);
                                        exit_armed = false;

                                        let item_id =
                                            search_results[index].id.clone();
                                        let item_name =
                                            search_results[index].name.clone();

                                        eprintln!(
                                            "[poc] tapped search result {}: {} ({})",
                                            index,
                                            item_name,
                                            item_id
                                        );

                                        if !item_id.is_empty() {
                                            let request_id =
                                                next_load_request_id;
                                            next_load_request_id =
                                                next_load_request_id
                                                    .wrapping_add(1);
                                            selected = None;
                                            playlist_track_selected = None;
                                            pending_queue_selection = Some(
                                                PendingQueueSelection {
                                                    source:
                                                        QueueSource::Search,
                                                    track_id:
                                                        item_id.clone(),
                                                    request_id,
                                                    started:
                                                        std::time::Instant::now(),
                                                },
                                            );
                                            pending_load_command = Some(
                                                PendingLoadCommand {
                                                    request_id,
                                                    command: format!(
                                                        "LOAD_SEARCH {} {}",
                                                        request_id,
                                                        item_id
                                                    ),
                                                    started:
                                                        std::time::Instant::now(),
                                                },
                                            );
                                            eprintln!(
                                                "[poc] queued search load request {}",
                                                request_id
                                            );
                                        }

                                        dirty = true;
                                    }
                                } else if app_view
                                    == AppView::Playlists
                                {
                                    let rel = cur_y - LIST_TOP;
                                    let visible_row =
                                        (rel / ROW_HEIGHT) as usize;
                                    let index =
                                        playlist_scroll + visible_row;

                                    if index < playlists.len() {
                                        playlist_selected = Some(index);
                                        exit_armed = false;

                                        let playlist_id =
                                            playlists[index].id.clone();
                                        let playlist_name =
                                            playlists[index].name.clone();

                                        eprintln!(
                                            "[poc] selected playlist {} ({})",
                                            playlist_name,
                                            playlist_id
                                        );

                                        if !playlist_id.is_empty() {
                                            let fetched = daemon_query(
                                                &format!(
                                                    "PLAYLIST {}",
                                                    playlist_id
                                                ),
                                            );
                                            let track_count = fetched.len();

                                            playlist_tracks =
                                                if fetched.is_empty() {
                                                    vec![TrackItem {
                                                        id: String::new(),
                                                        name:
                                                            "No tracks found"
                                                                .to_string(),
                                                        artist: String::new(),
                                                    }]
                                                } else {
                                                    fetched
                                                };

                                            playlist_track_scroll = 0;
                                            playlist_track_selected = None;
                                            app_view =
                                                AppView::PlaylistTracks;

                                            eprintln!(
                                                "[poc] loaded {} tracks from playlist {}",
                                                track_count,
                                                playlist_name
                                            );
                                        }

                                        dirty = true;
                                    }
                                } else if app_view
                                    == AppView::PlaylistTracks
                                {
                                    let rel = cur_y - LIST_TOP;
                                    let visible_row =
                                        (rel / ROW_HEIGHT) as usize;
                                    let index =
                                        playlist_track_scroll + visible_row;

                                    if index < playlist_tracks.len() {
                                        playlist_track_selected = Some(index);
                                        exit_armed = false;

                                        let item_id =
                                            playlist_tracks[index].id.clone();
                                        let item_name =
                                            playlist_tracks[index].name.clone();

                                        eprintln!(
                                            "[poc] tapped playlist track {}: {} ({})",
                                            index,
                                            item_name,
                                            item_id
                                        );

                                        if !item_id.is_empty() {
                                            let playlist_id =
                                                playlist_selected
                                                    .and_then(|selected_index| {
                                                        playlists.get(
                                                            selected_index,
                                                        )
                                                    })
                                                    .map(|playlist| {
                                                        playlist.id.clone()
                                                    });

                                            if let Some(playlist_id) =
                                                playlist_id
                                            {
                                                let request_id =
                                                    next_load_request_id;
                                                next_load_request_id =
                                                    next_load_request_id
                                                        .wrapping_add(1);
                                                selected = None;
                                                pending_queue_selection = Some(
                                                    PendingQueueSelection {
                                                        source:
                                                            QueueSource::Playlist(
                                                                playlist_id
                                                                    .clone(),
                                                            ),
                                                        track_id:
                                                            item_id.clone(),
                                                        request_id,
                                                        started:
                                                            std::time::Instant::now(),
                                                    },
                                                );
                                                pending_load_command = Some(
                                                    PendingLoadCommand {
                                                        request_id,
                                                        command: format!(
                                                    "LOAD_PLAYLIST {} {} {}",
                                                    request_id,
                                                    playlist_id,
                                                    item_id
                                                        ),
                                                        started:
                                                            std::time::Instant::now(),
                                                    },
                                                );
                                                eprintln!(
                                                    "[poc] queued playlist load request {}",
                                                    request_id
                                                );
                                            }
                                        }

                                        dirty = true;
                                    }
                                } else {
                                    let rel = cur_y - LIST_TOP;
                                    let visible_row = (rel / ROW_HEIGHT) as usize;
                                    let idx = scroll + visible_row;

                                    if idx < items.len() {
                                        selected = Some(idx);
                                        exit_armed = false;

                                        let item_id = items[idx].id.clone();
                                        let item_name = items[idx].name.clone();

                                        eprintln!(
                                            "[poc] tapped idx {idx}: {} ({})",
                                            item_name, item_id
                                        );

                                        if !item_id.is_empty() {
                                            let request_id =
                                                next_load_request_id;
                                            next_load_request_id =
                                                next_load_request_id
                                                    .wrapping_add(1);
                                            playlist_track_selected = None;
                                            pending_queue_selection = Some(
                                                PendingQueueSelection {
                                                    source:
                                                        QueueSource::Liked,
                                                    track_id:
                                                        item_id.clone(),
                                                    request_id,
                                                    started:
                                                        std::time::Instant::now(),
                                                },
                                            );
                                            pending_load_command = Some(
                                                PendingLoadCommand {
                                                    request_id,
                                                    command: format!(
                                                        "LOAD_LIKED {} {}",
                                                        request_id,
                                                        item_id
                                                    ),
                                                    started:
                                                        std::time::Instant::now(),
                                                },
                                            );
                                            eprintln!(
                                                "[poc] queued liked load request {}",
                                                request_id
                                            );
                                        }

                                        dirty = true;
                                    }
                                }
                                touch_down = false;
                            }
                        }
                        _ => {}
                    }
                }
                Err(ref e) if e.kind() == std::io::ErrorKind::WouldBlock => break,
                Err(_) => break,
            }
        }

        // Drain physical power-button events (event0), if available. A press
        // wakes only an asleep display and is never treated as a UI action.
        if let Some(ref mut pin) = power_input {
            loop {
                match pin.read_exact(&mut ev) {
                    Ok(()) => {
                        let etype = le_u16(&ev[8..10]);
                        let code = le_u16(&ev[10..12]);
                        let value = le_i32(&ev[12..16]);

                        if etype == EV_KEY
                            && code == KEY_POWER
                            && value == 1
                            && screen_asleep
                        {
                            screen_asleep = false;
                            last_user_input = std::time::Instant::now();
                            panel_wake(fb.file.as_raw_fd());
                            apply_brightness(brightness_idx);
                            fb.flush().ok();
                            last_flush = std::time::Instant::now();
                            eprintln!("[poc] screen woke from power button");
                        }
                    }
                    Err(ref e)
                        if e.kind() == std::io::ErrorKind::WouldBlock =>
                    {
                        break;
                    }
                    Err(_) => break,
                }
            }
        }

        // Drain volume button events (event2), if available.
        if let Some(ref mut vin) = vol_input {
            loop {
                match vin.read_exact(&mut ev) {
                    Ok(()) => {
                        let etype = le_u16(&ev[8..10]);
                        let code = le_u16(&ev[10..12]);
                        let value = le_i32(&ev[12..16]);
                        // Act on key press (value == 1) only.
                        if etype == EV_KEY && value == 1 {
                            last_user_input = std::time::Instant::now();
                            if code == KEY_VOLUMEUP {
                                let reply = daemon_request("VOL_UP");

                                match reply
                                    .as_deref()
                                    .and_then(parse_volume_reply)
                                {
                                    Some(percent) => {
                                        volume_popup = Some((
                                            percent,
                                            std::time::Instant::now(),
                                        ));
                                        now_playing_dirty = true;
                                        eprintln!(
                                            "[poc] volume up -> {}%",
                                            percent
                                        );
                                    }
                                    None => {
                                        eprintln!(
                                            "[poc] volume up reply unavailable"
                                        );
                                    }
                                }
                            } else if code == KEY_VOLUMEDOWN {
                                let reply = daemon_request("VOL_DOWN");

                                match reply
                                    .as_deref()
                                    .and_then(parse_volume_reply)
                                {
                                    Some(percent) => {
                                        volume_popup = Some((
                                            percent,
                                            std::time::Instant::now(),
                                        ));
                                        now_playing_dirty = true;
                                        eprintln!(
                                            "[poc] volume down -> {}%",
                                            percent
                                        );
                                    }
                                    None => {
                                        eprintln!(
                                            "[poc] volume down reply unavailable"
                                        );
                                    }
                                }
                            }
                        }
                    }
                    Err(ref e) if e.kind() == std::io::ErrorKind::WouldBlock => break,
                    Err(_) => break,
                }
            }
        }

        // Apply the saved brightness once, after the UI has been running
// long enough for the initial draw and keepalive flushes to settle.
if !startup_brightness_applied
&& startup_time.elapsed().as_millis() >= 2000
{
apply_brightness(brightness_idx);
startup_brightness_applied = true;
eprintln!(
"[poc] restored saved brightness -> {}",
BRIGHTNESS_LABELS[brightness_idx]
);
}

        if pending_load_command
            .as_ref()
            .map(|pending| {
                pending.started.elapsed()
                    >= std::time::Duration::from_millis(600)
            })
            .unwrap_or(false)
        {
            if let Some(pending) = pending_load_command.take() {
                daemon_send(&pending.command);
                eprintln!(
                    "[poc] dispatched load request {}",
                    pending.request_id
                );
            }
        }

        if search_in_progress {
            let completed = search_receiver
                .as_ref()
                .and_then(|receiver| receiver.try_recv().ok());

            if let Some(fetched) = completed {
                let result_count = fetched.len();
                remember_search(&mut recent_searches, &search_query);
                search_results = if fetched.is_empty() {
                    vec![TrackItem {
                        id: String::new(),
                        name: "No search results".to_string(),
                        artist: String::new(),
                    }]
                } else {
                    fetched
                };
                search_scroll = 0;
                search_selected = None;
                search_in_progress = false;
                search_receiver = None;
                app_view = AppView::SearchResults;
                dirty = true;
                eprintln!(
                    "[poc] loaded {} search results",
                    result_count
                );
            }
        }

// If we haven't loaded real tracks yet (daemon wasn't ready at
        // startup), retry the LIKED fetch every ~2s until it succeeds. This
        // makes the UI robust to being launched before the daemon is up.
        if !tracks_loaded && last_liked_retry.elapsed().as_millis() >= 2000 {
            last_liked_retry = std::time::Instant::now();
            dirty = true;
            let updated_stage = if !wifi_has_default_route() {
                StartupStage::Wifi
            } else if daemon_playback_state().is_some() {
                StartupStage::Library
            } else {
                StartupStage::Spotify
            };

            if startup_stage != Some(updated_stage) {
                startup_stage = Some(updated_stage);
                startup_stage_started_at = std::time::Instant::now();
                startup_retry_at = None;
                dirty = true;
                eprintln!(
                    "[poc] startup stage -> {:?}",
                    updated_stage
                );
            }

            let fetched = daemon_query("LIKED");
            if !fetched.is_empty() {
                items = fetched;
                tracks_loaded = true;
                startup_stage = None;
                startup_retry_at = None;
                scroll = 0;
                selected = None;
                dirty = true;
                eprintln!("[poc] loaded {} liked tracks (retry)", items.len());
            }
        }

        if startup_retry_at
            .map(|started| started.elapsed().as_millis() >= 1500)
            .unwrap_or(false)
        {
            startup_retry_at = None;
            dirty = true;
        }

        // Synchronize playback state, current-track metadata, and position
        // with the daemon once a second.
        if last_status_check.elapsed().as_millis() >= 1000 {
            last_status_check = std::time::Instant::now();

            match daemon_playback_state() {
                Some(updated_state) => {
                    consecutive_status_failures = 0;
                    if updated_state != playback_state {
                        playback_state = updated_state;
                        dirty = true;
                        eprintln!(
                            "[poc] playback state -> {:?}",
                            playback_state
                        );
                    }
                }
                None => {
                    consecutive_status_failures =
                        consecutive_status_failures.saturating_add(1);

                    if consecutive_status_failures >= 2
                        && playback_state != PlaybackState::Unknown
                    {
                        playback_state = PlaybackState::Unknown;
                        now_playing = None;
                        playback_position = None;
                        active_queue_status = None;
                        pending_queue_selection = None;
                        dirty = true;
                        eprintln!(
                            "[poc] daemon unavailable -> reconnecting"
                        );
                    }
                }
            }

            if let Some(updated_now_playing) = daemon_now_playing() {
                if updated_now_playing != now_playing {
                    now_playing = updated_now_playing;
                    now_playing_dirty = true;

                    match now_playing.as_ref() {
                        Some(item) => eprintln!(
                            "[poc] now playing -> {} - {} [{}] ({} ms)",
                            item.title,
                            item.artist,
                            item.id,
                            item.duration_ms
                        ),
                        None => eprintln!("[poc] now playing -> none"),
                    }
                }
            }

            if let Some(updated_queue) = daemon_queue_status() {
                if pending_queue_selection
                    .as_ref()
                    .map(|pending| {
                        pending.started.elapsed().as_secs() >= 3
                    })
                    .unwrap_or(false)
                {
                    eprintln!("[poc] pending queue selection timed out");
                    pending_queue_selection = None;
                }

                let queue_matches_pending = match (
                    pending_queue_selection.as_ref(),
                    updated_queue.as_ref(),
                ) {
                    (None, _) => true,
                    (Some(_), None) => false,
                    (Some(pending), Some(queue)) => {
                        pending.request_id == queue.request_id
                            && pending.source == queue.source
                            && pending.track_id == queue.track_id
                    }
                };

                if queue_matches_pending {
                    active_queue_status = updated_queue.clone();

                    if pending_queue_selection.is_some() {
                        eprintln!(
                            "[poc] queue request {} acknowledged",
                            updated_queue
                                .as_ref()
                                .map(|queue| queue.request_id)
                                .unwrap_or(0)
                        );
                        pending_queue_selection = None;
                    }

                    let mut updated_liked_selected = None;
                    let mut updated_playlist_track_selected = None;
                    let mut updated_search_selected = None;

                    if let Some(queue) = updated_queue.as_ref() {
                        match &queue.source {
                            QueueSource::Liked => {
                                let indexed_match = items
                                    .get(queue.index)
                                    .map(|item| {
                                        item.id.as_str()
                                            == queue.track_id.as_str()
                                    })
                                    .unwrap_or(false);

                                updated_liked_selected =
                                    if indexed_match {
                                        Some(queue.index)
                                    } else {
                                        items.iter().position(|item| {
                                            item.id.as_str()
                                                == queue.track_id.as_str()
                                        })
                                    };
                            }
                            QueueSource::Playlist(
                                queue_playlist_id,
                            ) => {
                                let displayed_playlist_matches =
                                    playlist_selected
                                        .and_then(|selected_index| {
                                            playlists.get(selected_index)
                                        })
                                        .map(|playlist| {
                                            playlist.id.as_str()
                                                == queue_playlist_id.as_str()
                                        })
                                        .unwrap_or(false);

                                if displayed_playlist_matches {
                                    let indexed_match = playlist_tracks
                                        .get(queue.index)
                                        .map(|item| {
                                            item.id.as_str()
                                                == queue.track_id.as_str()
                                        })
                                        .unwrap_or(false);

                                    updated_playlist_track_selected =
                                        if indexed_match {
                                            Some(queue.index)
                                        } else {
                                            playlist_tracks
                                                .iter()
                                                .position(|item| {
                                                    item.id.as_str()
                                                        == queue
                                                            .track_id
                                                            .as_str()
                                                })
                                        };
                                }
                            }
                            QueueSource::Search => {
                                let indexed_match = search_results
                                    .get(queue.index)
                                    .map(|item| {
                                        item.id.as_str()
                                            == queue.track_id.as_str()
                                    })
                                    .unwrap_or(false);

                                updated_search_selected =
                                    if indexed_match {
                                        Some(queue.index)
                                    } else {
                                        search_results
                                            .iter()
                                            .position(|item| {
                                                item.id.as_str()
                                                    == queue
                                                        .track_id
                                                        .as_str()
                                            })
                                    };
                            }
                        }
                    }

                    if updated_liked_selected != selected {
                        selected = updated_liked_selected;
                        dirty = true;

                        match (
                            selected,
                            updated_queue.as_ref(),
                        ) {
                            (
                                Some(index),
                                Some(QueueStatus {
                                    source: QueueSource::Liked,
                                    length,
                                    track_id,
                                    ..
                                }),
                            ) => {
                                eprintln!(
                                    "[poc] liked queue selection -> {}/{} ({})",
                                    index + 1,
                                    length,
                                    track_id
                                );
                            }
                            _ => {
                                eprintln!(
                                    "[poc] liked queue selection -> none"
                                );
                            }
                        }
                    }

                    if updated_playlist_track_selected
                        != playlist_track_selected
                    {
                        playlist_track_selected =
                            updated_playlist_track_selected;
                        dirty = true;

                        match (
                            playlist_track_selected,
                            updated_queue.as_ref(),
                        ) {
                            (
                                Some(index),
                                Some(QueueStatus {
                                    source:
                                        QueueSource::Playlist(
                                            playlist_id,
                                        ),
                                    length,
                                    track_id,
                                    ..
                                }),
                            ) => {
                                eprintln!(
                                    "[poc] playlist {} queue selection -> {}/{} ({})",
                                    playlist_id,
                                    index + 1,
                                    length,
                                    track_id
                                );
                            }
                            _ => {
                                eprintln!(
                                    "[poc] playlist queue selection -> none"
                                );
                            }
                        }
                    }

                    if updated_search_selected != search_selected {
                        search_selected = updated_search_selected;
                        dirty = true;

                        match (
                            search_selected,
                            updated_queue.as_ref(),
                        ) {
                            (
                                Some(index),
                                Some(QueueStatus {
                                    source: QueueSource::Search,
                                    length,
                                    track_id,
                                    ..
                                }),
                            ) => {
                                eprintln!(
                                    "[poc] search queue selection -> {}/{} ({})",
                                    index + 1,
                                    length,
                                    track_id
                                );
                            }
                            _ => {
                                eprintln!(
                                    "[poc] search queue selection -> none"
                                );
                            }
                        }
                    }
                }
            }

            if let Some(updated_position) = daemon_playback_position() {
                if updated_position != playback_position {
                    let should_log = match (playback_position, updated_position) {
                        (None, None) => false,
                        (Some(old), Some(new)) => old / 5000 != new / 5000,
                        _ => true,
                    };

                    playback_position = updated_position;
                    now_playing_dirty = true;

                    if should_log {
                        match playback_position {
                            Some(position_ms) => {
                                eprintln!(
                                    "[poc] playback position -> {} ms",
                                    position_ms
                                )
                            }
                            None => {
                                eprintln!("[poc] playback position -> none")
                            }
                        }
                    }
                }
            }

            if app_view == AppView::UpNext {
                if let Some(mut updated_page) =
                    daemon_queue_page(up_next_offset)
                {
                    if let Some(page) = updated_page.as_ref() {
                        let page_end = page.start + page.items.len();
                        if page.current_position < page.start
                            || page.current_position >= page_end
                        {
                            up_next_offset = page.current_position
                                / VISIBLE_ROWS
                                * VISIBLE_ROWS;
                            updated_page = daemon_queue_page(up_next_offset)
                                .flatten();
                        }
                    }

                    if updated_page != up_next_page {
                        up_next_page = updated_page;
                        dirty = true;
                    }
                }
            }

        }

        // Refresh the battery value every 30 seconds.
        if last_battery_check.elapsed().as_secs() >= 30 {
            last_battery_check = std::time::Instant::now();
            let updated = read_battery_percent();
            if updated != battery_percent {
                battery_percent = updated;
                dirty = true;
                eprintln!("[poc] battery -> {:?}", battery_percent);
            }
        }

        // Refresh available /usr/data space every 30 seconds.
        if last_storage_check.elapsed().as_secs() >= 30 {
            last_storage_check = std::time::Instant::now();
            let updated = read_storage_free_mb();

            if updated != storage_free_mb {
                storage_free_mb = updated;
                dirty = true;
                eprintln!(
                    "[poc] storage free -> {:?} MB",
                    storage_free_mb
                );
            }
        }

        // Refresh available memory every 30 seconds.
        if last_memory_check.elapsed().as_secs() >= 30 {
            last_memory_check = std::time::Instant::now();
            let updated = read_memory_available_mb();

            if updated != memory_available_mb {
                memory_available_mb = updated;
                dirty = true;
                eprintln!(
                    "[poc] memory available -> {:?} MB",
                    memory_available_mb
                );
            }
        }

        if matches!(
            theme,
            Theme::ElKayKay
                | Theme::Tidepool
                | Theme::CitrusGrove
                | Theme::MonochromeStatic
                | Theme::PaperLantern
                | Theme::ArcadeBloom
                | Theme::DesertBloom
                | Theme::AlNoor
                | Theme::NightMarket
        )
            && pending_load_command.is_none()
            && pending_queue_selection.is_none()
            && playback_state != PlaybackState::Loading
            && last_user_input.elapsed().as_millis() >= AMBIENT_IDLE_MS
            && last_static_refresh.elapsed().as_millis() >= 1000
        {
            last_static_refresh = std::time::Instant::now();
            static_seed = static_seed
                .wrapping_mul(1_664_525)
                .wrapping_add(1_013_904_223);
            animation_frame = animation_frame.wrapping_add(1);
            dirty = true;
        }

        let volume_popup_expired = volume_popup
            .as_ref()
            .map(|(_, shown_at)| {
                shown_at.elapsed().as_millis()
                    >= VOLUME_POPUP_MS
            })
            .unwrap_or(false);

        if volume_popup_expired {
            volume_popup = None;
            now_playing_dirty = true;
        }

        if diagnostics_refreshed_at
            .as_ref()
            .map(|refreshed_at| {
                refreshed_at.elapsed().as_millis() >= 1500
            })
            .unwrap_or(false)
        {
            diagnostics_refreshed_at = None;
            dirty = true;
        }

        let screen_sleep_due = SCREEN_SLEEP_TIMEOUTS[screen_sleep_idx]
            .map(|timeout_ms| {
                last_user_input.elapsed().as_millis() >= timeout_ms
            })
            .unwrap_or(false);

        if !screen_asleep && startup_stage.is_none() && screen_sleep_due {
            write_sysfs(BL_BRIGHTNESS, "0");
            screen_asleep = true;
            dirty = false;
            now_playing_dirty = false;
            eprintln!(
                "[poc] screen slept after {}",
                SETTINGS_LABELS[screen_sleep_idx]
            );
        }

        // Re-check the output jack four times per second. Removing an active
        // output immediately requests a pause. Connecting a jack selects its
        // route but does not resume automatically.
        if last_jack_check.elapsed().as_millis() >= 250 {
            last_jack_check = std::time::Instant::now();

            let detected = if switch_active(SW_BALANCE) {
                Some(PORT_44MM)
            } else if switch_active(SW_HEADSET) {
                Some(PORT_35MM)
            } else {
                None
            };

            if detected != last_port {
                let previous_port = last_port;

                if previous_port.is_some() {
                    // PAUSE is safe to request even if the cached UI state is
                    // slightly stale or playback is already paused.
                    daemon_send("PAUSE");

                    if matches!(
                        playback_state,
                        PlaybackState::Unknown
                            | PlaybackState::Loading
                            | PlaybackState::Playing
                    ) {
                        playback_state = PlaybackState::Paused;
                        dirty = true;
                    }

                    eprintln!(
                        "[poc] jack removed or switched -> pause requested"
                    );
                }

                match detected {
                    Some(port) => {
                        set_output_port(port);
                        eprintln!(
                            "[poc] jack change -> output port {}",
                            port
                        );
                    }
                    None => {
                        eprintln!(
                            "[poc] jack change -> no jack detected"
                        );
                    }
                }

                last_port = detected;
            }
        }

        // The dedicated Now Playing view uses the whole content area, so its
        // metadata and progress updates require a full view redraw. Other
        // screens keep the cheaper strip-only update path.
        if app_view == AppView::NowPlaying && now_playing_dirty {
            dirty = true;
            now_playing_dirty = false;
        }

        // Re-render the whole interface only when global state changes.
        // Position and volume updates redraw just the 60-pixel track strip.
        if !screen_asleep && dirty {
            draw_list(
                &mut fb,
                &items,
                &playlists,
                &playlist_tracks,
                &search_results,
                &recent_searches,
                &search_query,
                search_in_progress,
                up_next_page.as_ref(),
                scroll,
                playlist_scroll,
                playlist_track_scroll,
                search_scroll,
                selected,
                playlist_selected,
                playlist_track_selected,
                search_selected,
                title,
                battery_percent,
                storage_free_mb,
                memory_available_mb,
                brightness_idx,
                screen_sleep_idx,
                playback_state,
                startup_stage,
                startup_stage_started_at.elapsed().as_secs(),
                startup_retry_at.is_some(),
                playback_modes,
                active_queue_status.as_ref(),
                diagnostics_refreshed_at.is_some(),
                now_playing.as_ref(),
                playback_position,
                &palette,
                theme,
                static_seed,
                animation_frame,
                app_view,
                exit_armed,
            );

            if let Some((percent, _)) = volume_popup.as_ref() {
                draw_volume_popup(
                    &mut fb,
                    *percent,
                    &palette,
                );
            }

            now_playing_dirty = false;
            last_flush = std::time::Instant::now();
            dirty = false;
        } else if !screen_asleep && now_playing_dirty {
            draw_now_playing_strip(
                &mut fb,
                now_playing.as_ref(),
                playback_position,
                playback_state,
                &palette,
            );

            if let Some((percent, _)) = volume_popup.as_ref() {
                draw_volume_popup(
                    &mut fb,
                    *percent,
                    &palette,
                );
            } else {
                fb.flush().ok();
            }

            now_playing_dirty = false;
            last_flush = std::time::Instant::now();
        } else if !screen_asleep
            && last_flush.elapsed().as_millis() as u64 >= keepalive_ms
        {
            // Keep the panel lit between changes with a periodic flush.
            fb.keepalive();
            last_flush = std::time::Instant::now();
        }

        // Short sleep so the input loop stays responsive without busy-spinning.
        std::thread::sleep(std::time::Duration::from_millis(20));
    }
}
