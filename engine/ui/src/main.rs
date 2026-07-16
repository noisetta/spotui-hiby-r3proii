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
// Run the spotui_daemon (piped to aplay) separately; this just drives it.

use std::{
    fs::{File, OpenOptions},
    io::{Read, Write},
    os::unix::io::AsRawFd,
    os::unix::net::UnixStream,
};

use embedded_graphics::{
    mono_font::{ascii::FONT_9X15, ascii::FONT_9X15_BOLD, MonoTextStyle},
    pixelcolor::{Rgb565, WebColors},
    prelude::*,
    primitives::{PrimitiveStyle, Rectangle},
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

/// Send a command line to the daemon's unix socket (fire and forget).
fn daemon_send(cmd: &str) {
    if let Ok(mut s) = UnixStream::connect(DAEMON_SOCK) {
        let _ = s.write_all(cmd.as_bytes());
        let _ = s.write_all(b"\n");
        // read a short reply so the daemon processes it, then drop
        let mut buf = [0u8; 128];
        let _ = s.read(&mut buf);
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

/// A track fetched from the daemon's browse commands.
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

const BACKLIGHT_BRIGHTNESS: &str = "/sys/class/backlight/backlight_pwm0/brightness";
const BATTERY_CAPACITY: &str = "/sys/class/power_supply/battery/capacity";
const BRIGHTNESS_STATE_FILE: &str = "/usr/data/spotui_brightness";
const BRIGHTNESS_LEVELS: [u32; 5] = [101, 80, 60, 40, 25];
const BRIGHTNESS_LABELS: [&str; 5] = ["100%", "80%", "60%", "40%", "25%"];


fn read_battery_percent() -> Option<u8> {
    std::fs::read_to_string(BATTERY_CAPACITY)
        .ok()
        .and_then(|s| s.trim().parse::<u8>().ok())
        .filter(|value| *value <= 100)
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

/// Number of track rows visible on screen at once (below the header).
/// (720 - 40 header) / 60 per row = ~11.
const VISIBLE_ROWS: usize = 10;

/// Draw the track list with scrolling. `scroll` is the index of the first
/// visible item; `selected` highlights one row (absolute index).
fn draw_list(
    fb: &mut Framebuffer,
    items: &[TrackItem],
    scroll: usize,
    selected: Option<usize>,
    title: &str,
    battery_percent: Option<u8>,
    brightness_idx: usize,
    playback_state: PlaybackState,
    exit_armed: bool,
) {
    // Clear to dark blue.
    Rectangle::new(Point::zero(), Size::new(WIDTH as u32, HEIGHT as u32))
        .into_styled(PrimitiveStyle::with_fill(Rgb565::new(0, 0, 12)))
        .draw(fb)
        .ok();

    // Green header bar with a title.
    Rectangle::new(Point::zero(), Size::new(WIDTH as u32, 40))
        .into_styled(PrimitiveStyle::with_fill(Rgb565::new(0, 63, 0)))
        .draw(fb)
        .ok();
    let header_style = MonoTextStyle::new(&FONT_9X15_BOLD, Rgb565::BLACK);
    Text::with_baseline(title, Point::new(6, 12), header_style, Baseline::Top)
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

    let text_style = MonoTextStyle::new(&FONT_9X15, Rgb565::WHITE);
    let sel_style = MonoTextStyle::new(&FONT_9X15_BOLD, Rgb565::BLACK);

    // Render the visible window of items.
    let end = (scroll + VISIBLE_ROWS).min(items.len());
    for (row, i) in (scroll..end).enumerate() {
        let item = &items[i];
        let mut label = item.label();
        let y = 40 + row as i32 * ROW_HEIGHT;
        let is_sel = selected == Some(i);

        label = truncate_label(&label, if is_sel { 50 } else { 52 });

        if is_sel {
            Rectangle::new(Point::new(0, y), Size::new(WIDTH as u32, ROW_HEIGHT as u32))
                .into_styled(PrimitiveStyle::with_fill(Rgb565::new(0, 45, 45)))
                .draw(fb)
                .ok();
            let selected_label = format!("> {}", label);
              Text::with_baseline(&selected_label, Point::new(10, y + 22), sel_style, Baseline::Top)
                  .draw(fb)
                .ok();
        } else {
            Text::with_baseline(&label, Point::new(10, y + 22), text_style, Baseline::Top)
                .draw(fb)
                .ok();
        }
        // separator line
        Rectangle::new(Point::new(0, y + ROW_HEIGHT - 1), Size::new(WIDTH as u32, 1))
            .into_styled(PrimitiveStyle::with_fill(Rgb565::CSS_DARK_GRAY))
            .draw(fb)
            .ok();
    }

    // Header scroll indicators.
    // Tapping most of the header pages up; the far-right section pages down.
    if scroll > 0 {
        Text::with_baseline(
            "^",
            Point::new(WIDTH as i32 - 112, 12),
            header_style,
            Baseline::Top,
        )
        .draw(fb)
        .ok();
    }

    if end < items.len() {
        Text::with_baseline(
            "v",
            Point::new(WIDTH as i32 - 88, 12),
            header_style,
            Baseline::Top,
        )
        .draw(fb)
        .ok();
    }

    // Non-interactive separator immediately above the toolbar.
    let down_strip_y = HEIGHT as i32 - 80;
    Rectangle::new(
        Point::new(0, down_strip_y),
        Size::new(WIDTH as u32, 20),
    )
    .into_styled(PrimitiveStyle::with_fill(Rgb565::new(0, 24, 0)))
    .draw(fb)
    .ok();

    // Fixed four-button toolbar.
    let toolbar_y = HEIGHT as i32 - 60;
    Rectangle::new(
        Point::new(0, toolbar_y),
        Size::new(WIDTH as u32, 60),
    )
    .into_styled(PrimitiveStyle::with_fill(Rgb565::new(0, 38, 0)))
    .draw(fb)
    .ok();

    for x in [120, 240, 360] {
        Rectangle::new(Point::new(x, toolbar_y), Size::new(1, 60))
            .into_styled(PrimitiveStyle::with_fill(Rgb565::CSS_DARK_GRAY))
            .draw(fb)
            .ok();
    }

    let exit_label = if exit_armed { "Confirm" } else { "Exit" };
    let brightness_label =
        format!("Bright {}", BRIGHTNESS_LABELS[brightness_idx]);
    let playback_label = if playback_state.is_paused() { "Resume" } else { "Pause" };
    let button_style = MonoTextStyle::new(&FONT_9X15_BOLD, Rgb565::WHITE);

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

    Text::with_baseline(
        "Refresh",
        Point::new(385, toolbar_y + 22),
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

    let mut brightness_idx: usize = load_brightness_idx();
    let mut selected: Option<usize> = None;
    let mut scroll: usize = 0;
    let mut playback_state = PlaybackState::Unknown;
    let mut exit_armed = false;
    let title = "Liked Songs";
    let mut battery_percent = read_battery_percent();
    draw_list(
        &mut fb,
        &items,
        scroll,
        selected,
        title,
        battery_percent,
        brightness_idx,
        playback_state,
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
    let mut last_status_check = std::time::Instant::now();
    let mut last_battery_check = std::time::Instant::now();
    let startup_time = std::time::Instant::now();
    let mut startup_brightness_applied = false;

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
                                // Header = page up; strip above toolbar = page down.
                                // The bottom 60px are four equal-width controls.
                                const DOWN_STRIP_TOP: i32 = 640;
                                const TOOLBAR_TOP: i32 = 660;
                                const BUTTON_WIDTH: i32 = 120;

                                if cur_y < LIST_TOP {
                                    let page =
                                        VISIBLE_ROWS.saturating_sub(1).max(1);
                                    let max_scroll =
                                        items.len().saturating_sub(VISIBLE_ROWS);

                                    if cur_x < WIDTH as i32 - 120 {
                                        scroll = scroll.saturating_sub(page);
                                        eprintln!(
                                            "[poc] header scroll up -> {scroll}"
                                        );
                                    } else {
                                        scroll = (scroll + page).min(max_scroll);
                                        eprintln!(
                                            "[poc] header scroll down -> {scroll}"
                                        );
                                    }

                                    exit_armed = false;
                                    dirty = true;
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
                                            eprintln!("[poc] toolbar refresh");
                                            let fetched = daemon_query("LIKED");
                                            if fetched.is_empty() {
                                                eprintln!(
                                                    "[poc] refresh returned no tracks; keeping current list"
                                                );
                                            } else {
                                                items = fetched;
                                                tracks_loaded = true;
                                                scroll = 0;
                                                selected = None;
                                                dirty = true;
                                                eprintln!(
                                                    "[poc] refresh loaded {} tracks",
                                                    items.len()
                                                );
                                            }
                                        }
                                        _ => {}
                                    }
                                } else if cur_y >= DOWN_STRIP_TOP {
                                    // Separator area intentionally does nothing.
                                    exit_armed = false;
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
                                            daemon_send(&format!("LOAD {}", item_id));
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
                            if code == KEY_VOLUMEUP {
                                daemon_send("VOL_UP");
                                eprintln!("[poc] volume up");
                            } else if code == KEY_VOLUMEDOWN {
                                daemon_send("VOL_DOWN");
                                eprintln!("[poc] volume down");
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

// If we haven't loaded real tracks yet (daemon wasn't ready at
        // startup), retry the LIKED fetch every ~2s until it succeeds. This
        // makes the UI robust to being launched before the daemon is up.
        if !tracks_loaded && last_liked_retry.elapsed().as_millis() >= 2000 {
            last_liked_retry = std::time::Instant::now();
            let fetched = daemon_query("LIKED");
            if !fetched.is_empty() {
                items = fetched;
                            tracks_loaded = true;
                scroll = 0;
                selected = None;
                dirty = true;
                eprintln!("[poc] loaded {} liked tracks (retry)", items.len());
            }
        }

        // Synchronize playback state with the daemon once a second.
        if last_status_check.elapsed().as_millis() >= 1000 {
            last_status_check = std::time::Instant::now();
            if let Some(updated_state) = daemon_playback_state() {
                if updated_state != playback_state {
                    playback_state = updated_state;
                    dirty = true;
                    eprintln!("[poc] playback state -> {:?}", playback_state);
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

        // Re-check the output jack roughly once a second so plugging into a
        // different jack (3.5mm <-> 4.4mm) re-routes automatically. Only calls
        // amixer when the detected port actually changes, to avoid churn.
        if last_jack_check.elapsed().as_millis() >= 1000 {
            last_jack_check = std::time::Instant::now();
            let detected = if switch_active(SW_BALANCE) {
                Some(PORT_44MM)
            } else if switch_active(SW_HEADSET) {
                Some(PORT_35MM)
            } else {
                None
            };
            if detected != last_port {
                if let Some(p) = detected {
                    set_output_port(p);
                    eprintln!("[poc] jack change -> output port {p}");
                }
                last_port = detected;
            }
        }

        // Re-render the list only when something changed (a tap). This is the
        // expensive part (embedded-graphics + 691KB flush), so we avoid it when
        // idle.
        if dirty {
            draw_list(
                &mut fb,
                &items,
                scroll,
                selected,
                title,
                battery_percent,
                brightness_idx,
                playback_state,
                exit_armed,
            );
            last_flush = std::time::Instant::now();
            dirty = false;
        } else if last_flush.elapsed().as_millis() as u64 >= keepalive_ms {
            // Keep the panel lit between changes with a periodic flush.
            fb.keepalive();
            last_flush = std::time::Instant::now();
        }

        // Short sleep so the input loop stays responsive without busy-spinning.
        std::thread::sleep(std::time::Duration::from_millis(20));
    }
}
