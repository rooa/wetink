//! Color themes and terminal background detection.

use ratatui::style::Color;
use std::sync::OnceLock;

pub struct Theme {
    pub text: Color,
    pub dim: Color,
    pub border: Color,
    pub h1_fg: Color,
    pub h1_bg: Color,
    pub heading: Color,
    pub h6: Color,
    pub code_fg: Color,
    pub code_bg: Color,
    pub link_text: Color,
    pub link_url: Color,
    pub image_text: Color,
    pub image_url: Color,
    pub important: Color,
    pub warning: Color,
    pub caution: Color,
    pub bar_fg: Color,
    pub bar_bg: Color,
    pub added: Color,
    pub modified: Color,
    pub removed: Color,
    /// Flash backgrounds; they fade to `base`, the terminal background.
    pub added_tint: (u8, u8, u8),
    pub modified_tint: (u8, u8, u8),
    pub base: (u8, u8, u8),
    /// syntect theme for code blocks.
    #[cfg_attr(not(feature = "syntax"), allow(dead_code))]
    pub syntax: &'static str,
}

const DARK: Theme = Theme {
    text: Color::Indexed(252),
    dim: Color::Indexed(244),
    border: Color::Indexed(240),
    h1_fg: Color::Indexed(228),
    h1_bg: Color::Indexed(63),
    heading: Color::Indexed(39),
    h6: Color::Indexed(35),
    code_fg: Color::Indexed(203),
    code_bg: Color::Indexed(236),
    link_text: Color::Indexed(35),
    link_url: Color::Indexed(30),
    image_text: Color::Indexed(243),
    image_url: Color::Indexed(212),
    important: Color::Indexed(141),
    warning: Color::Indexed(214),
    caution: Color::Indexed(203),
    bar_fg: Color::Indexed(250),
    bar_bg: Color::Indexed(236),
    added: Color::Indexed(42),
    modified: Color::Indexed(214),
    removed: Color::Indexed(203),
    added_tint: (30, 95, 50),
    modified_tint: (105, 85, 20),
    base: (0, 0, 0),
    syntax: "base16-ocean.dark",
};

const LIGHT: Theme = Theme {
    text: Color::Indexed(234),
    dim: Color::Indexed(243),
    border: Color::Indexed(250),
    h1_fg: Color::Indexed(228),
    h1_bg: Color::Indexed(63),
    heading: Color::Indexed(27),
    h6: Color::Indexed(29),
    code_fg: Color::Indexed(160),
    code_bg: Color::Indexed(254),
    link_text: Color::Indexed(29),
    link_url: Color::Indexed(31),
    image_text: Color::Indexed(243),
    image_url: Color::Indexed(162),
    important: Color::Indexed(92),
    warning: Color::Indexed(130),
    caution: Color::Indexed(160),
    bar_fg: Color::Indexed(238),
    bar_bg: Color::Indexed(254),
    added: Color::Indexed(28),
    modified: Color::Indexed(172),
    removed: Color::Indexed(160),
    added_tint: (185, 235, 190),
    modified_tint: (250, 222, 150),
    base: (255, 255, 255),
    syntax: "InspiredGitHub",
};

static THEME: OnceLock<Theme> = OnceLock::new();

/// The process-wide theme (dark until `init` is called).
pub fn t() -> &'static Theme {
    THEME.get_or_init(|| DARK)
}

impl Theme {
    /// Flash background at strength `k` (1 = full tint, 0 = terminal background).
    pub fn fade(&self, tint: (u8, u8, u8), k: f32) -> Color {
        let mix = |a: u8, b: u8| (a as f32 + (b as f32 - a as f32) * k) as u8;
        let (b, t) = (self.base, tint);
        Color::Rgb(mix(b.0, t.0), mix(b.1, t.1), mix(b.2, t.2))
    }
}

#[derive(Clone, Copy, PartialEq)]
pub enum Mode {
    Auto,
    Dark,
    Light,
}

/// Pick the theme. `query` asks the terminal for its background color, which
/// also makes flashes fade exactly into it.
pub fn init(mode: Mode, query: bool) {
    let bg = if query { query_bg() } else { None };
    let light = match mode {
        Mode::Dark => false,
        Mode::Light => true,
        Mode::Auto => bg.map(is_light).or_else(colorfgbg).unwrap_or(false),
    };
    let mut theme = if light { LIGHT } else { DARK };
    if let Some(bg) = bg {
        theme.base = bg;
    }
    let _ = THEME.set(theme);
}

fn is_light((r, g, b): (u8, u8, u8)) -> bool {
    0.2126 * r as f32 + 0.7152 * g as f32 + 0.0722 * b as f32 > 127.5
}

/// `COLORFGBG` ("fg;bg", set by rxvt, Konsole and others): background 7 or 15 is light.
fn colorfgbg() -> Option<bool> {
    let v = std::env::var("COLORFGBG").ok()?;
    let bg: u8 = v.rsplit(';').next()?.parse().ok()?;
    Some(matches!(bg, 7 | 15))
}

/// Parse an OSC 11 reply such as `ESC ] 11 ; rgb:ffff/ffff/ffff ESC \`.
#[cfg_attr(not(unix), allow(dead_code))]
fn parse_osc11(reply: &[u8]) -> Option<(u8, u8, u8)> {
    let s = String::from_utf8_lossy(reply);
    let rgb = &s[s.find("rgb:")? + 4..];
    let end = rgb.find(['\x07', '\x1b'])?;
    let mut parts = rgb[..end].split('/').map(|h| {
        let v = u32::from_str_radix(h, 16)
            .ok()
            .filter(|_| (1..=4).contains(&h.len()))?;
        Some((v * 255 / ((1 << (4 * h.len())) - 1)) as u8)
    });
    Some((parts.next()??, parts.next()??, parts.next()??))
}

/// Ask the terminal for its background color. The query is followed by a
/// device-attributes request that every terminal answers, so terminals that
/// ignore OSC 11 cost one round trip rather than the full timeout.
#[cfg(unix)]
fn query_bg() -> Option<(u8, u8, u8)> {
    use ratatui::crossterm::terminal;
    use std::io::{Read, Write};
    use std::os::fd::AsRawFd;
    use std::os::unix::fs::OpenOptionsExt;
    use std::time::{Duration, Instant};

    let mut tty = std::fs::OpenOptions::new()
        .read(true)
        .write(true)
        .custom_flags(libc::O_NONBLOCK)
        .open("/dev/tty")
        .ok()?;
    terminal::enable_raw_mode().ok()?;
    let mut buf = Vec::new();
    if tty.write_all(b"\x1b]11;?\x1b\\\x1b[c").is_ok() {
        let deadline = Instant::now() + Duration::from_millis(300);
        loop {
            let left = deadline
                .saturating_duration_since(Instant::now())
                .as_millis() as i32;
            let mut pfd = libc::pollfd {
                fd: tty.as_raw_fd(),
                events: libc::POLLIN,
                revents: 0,
            };
            // SAFETY: `pfd` is a valid pollfd for an open descriptor, and the count is 1.
            if left == 0 || unsafe { libc::poll(&mut pfd, 1, left) } <= 0 {
                break;
            }
            let mut chunk = [0; 256];
            match tty.read(&mut chunk) {
                Ok(0) => break,
                Ok(n) => buf.extend_from_slice(&chunk[..n]),
                Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {}
                Err(_) => break,
            }
            // The device-attributes reply is `ESC [ ? … c`.
            if buf
                .windows(3)
                .position(|w| w == b"\x1b[?")
                .is_some_and(|i| buf[i..].contains(&b'c'))
            {
                break;
            }
        }
    }
    let _ = terminal::disable_raw_mode();
    parse_osc11(&buf)
}

#[cfg(not(unix))]
fn query_bg() -> Option<(u8, u8, u8)> {
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_osc11_replies() {
        assert_eq!(
            parse_osc11(b"\x1b]11;rgb:ffff/ffff/ffff\x1b\\\x1b[?62c"),
            Some((255, 255, 255))
        );
        assert_eq!(parse_osc11(b"\x1b]11;rgb:1e/1e/2e\x07"), Some((30, 30, 46)));
        assert_eq!(parse_osc11(b"\x1b[?62;22c"), None);
        assert_eq!(parse_osc11(b"\x1b]11;rgb:zz/00/00\x07"), None);
    }

    #[test]
    fn classifies_backgrounds() {
        assert!(is_light((255, 255, 255)) && is_light((253, 246, 227)));
        assert!(!is_light((0, 0, 0)) && !is_light((30, 30, 46)));
    }

    #[test]
    fn fade_runs_from_tint_to_base() {
        let th = Theme {
            base: (10, 20, 30),
            ..DARK
        };
        assert_eq!(th.fade((110, 120, 130), 1.0), Color::Rgb(110, 120, 130));
        assert_eq!(th.fade((110, 120, 130), 0.0), Color::Rgb(10, 20, 30));
    }
}
