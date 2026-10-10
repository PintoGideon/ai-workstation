//! Is the terminal background dark or light? Used by `theme_follow_terminal`.
//!
//! Two sources, cheapest first:
//! 1. `COLORFGBG` (set by rxvt, Konsole, iTerm2 and others): `fg;bg` with
//!    ANSI palette indices.
//! 2. An OSC 11 query ("what is your background colour?") written to the tty
//!    before AIW enters raw/alternate-screen mode. It is followed by a DA1
//!    query, which every terminal answers, so we know when to stop reading:
//!    a terminal that ignores OSC 11 still answers DA1 and we return at once
//!    instead of waiting, and nothing is left behind to leak into the TUI's
//!    input. Terminals that answer neither are covered by a short timeout.

use std::time::Duration;

use super::contrast::relative_luminance;
use super::terminal::TermEnv;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Background {
    Dark,
    Light,
}

impl Background {
    pub fn label(self) -> &'static str {
        match self {
            Background::Dark => "dark",
            Background::Light => "light",
        }
    }

    fn from_rgb(rgb: (u8, u8, u8)) -> Self {
        if relative_luminance(rgb) > 0.4 {
            Background::Light
        } else {
            Background::Dark
        }
    }
}

/// Parse `COLORFGBG` (`"15;0"`, `"0;default;15"`, …): the last field is the
/// background's ANSI index. 7 and 9..=15 are light, 0..=6 and 8 are dark.
pub fn from_colorfgbg(value: &str) -> Option<Background> {
    let bg: u8 = value.rsplit(';').next()?.trim().parse().ok()?;
    match bg {
        7 | 9..=15 => Some(Background::Light),
        0..=6 | 8 => Some(Background::Dark),
        _ => None,
    }
}

/// Parse an OSC 11 reply such as `ESC ] 11 ; rgb:1414/1111/1010 BEL` (or
/// `ST`-terminated, or with 1–4 hex digits per channel).
pub fn parse_osc11_reply(bytes: &[u8]) -> Option<(u8, u8, u8)> {
    let text = String::from_utf8_lossy(bytes);
    let start = text.find("]11;")? + 4;
    let rest = &text[start..];
    let rest = rest
        .strip_prefix("rgba:")
        .or_else(|| rest.strip_prefix("rgb:"))?;
    let end = rest
        .find(|c: char| !(c.is_ascii_hexdigit() || c == '/'))
        .unwrap_or(rest.len());
    let mut channels = rest[..end].split('/').map(|hex| {
        if hex.is_empty() || hex.len() > 4 {
            return None;
        }
        let value = u32::from_str_radix(hex, 16).ok()?;
        let max = (1u32 << (4 * hex.len())) - 1;
        Some(((value * 255 + max / 2) / max) as u8)
    });
    Some((channels.next()??, channels.next()??, channels.next()??))
}

/// Whether `bytes` contains a complete DA1 reply (`ESC [ ? … c`).
fn has_da1_reply(bytes: &[u8]) -> bool {
    bytes.windows(3).enumerate().any(|(i, w)| {
        w == b"\x1b[?"
            && bytes[i + 3..]
                .iter()
                .take_while(|b| b.is_ascii_digit() || **b == b';' || **b == b'c')
                .any(|b| *b == b'c')
    })
}

/// Ask the terminal for its background colour. Unix only; `None` when there
/// is no tty, the terminal does not answer, or anything goes wrong.
#[cfg(unix)]
pub fn query_osc11(timeout: Duration) -> Option<(u8, u8, u8)> {
    use std::io::{Read, Write};
    use std::os::fd::AsRawFd;
    use std::time::Instant;

    let mut tty = std::fs::OpenOptions::new()
        .read(true)
        .write(true)
        .open("/dev/tty")
        .ok()?;
    let fd = tty.as_raw_fd();

    // Raw-ish mode just for the query: no echo, no line buffering.
    let mut original: libc::termios = unsafe { std::mem::zeroed() };
    if unsafe { libc::tcgetattr(fd, &mut original) } != 0 {
        return None;
    }
    let mut raw = original;
    raw.c_lflag &= !(libc::ICANON | libc::ECHO);
    raw.c_cc[libc::VMIN] = 0;
    raw.c_cc[libc::VTIME] = 0;
    if unsafe { libc::tcsetattr(fd, libc::TCSANOW, &raw) } != 0 {
        return None;
    }

    let mut reply = Vec::new();
    let result = (|| {
        tty.write_all(b"\x1b]11;?\x1b\\\x1b[c").ok()?;
        tty.flush().ok()?;
        let deadline = Instant::now() + timeout;
        let mut buf = [0u8; 256];
        while !has_da1_reply(&reply) {
            let left = deadline.saturating_duration_since(Instant::now());
            if left.is_zero() {
                break;
            }
            let mut pfd = libc::pollfd {
                fd,
                events: libc::POLLIN,
                revents: 0,
            };
            let ready = unsafe { libc::poll(&mut pfd, 1, left.as_millis().max(1) as libc::c_int) };
            if ready <= 0 {
                break;
            }
            match tty.read(&mut buf) {
                Ok(0) | Err(_) => break,
                Ok(n) => reply.extend_from_slice(&buf[..n]),
            }
        }
        parse_osc11_reply(&reply)
    })();

    unsafe {
        libc::tcsetattr(fd, libc::TCSANOW, &original);
    }
    result
}

#[cfg(not(unix))]
pub fn query_osc11(_timeout: Duration) -> Option<(u8, u8, u8)> {
    None
}

/// Detect the background: `COLORFGBG` first, then (when `allow_query`) the
/// OSC 11 query. Returns the answer and where it came from.
pub fn detect(env: &TermEnv, allow_query: bool) -> Option<(Background, String)> {
    if let Some(value) = env.get("COLORFGBG")
        && let Some(bg) = from_colorfgbg(value)
    {
        return Some((bg, format!("COLORFGBG={value}")));
    }
    if !allow_query || env.get("TERM") == Some("dumb") {
        return None;
    }
    let is_tty =
        unsafe { libc::isatty(libc::STDIN_FILENO) == 1 && libc::isatty(libc::STDOUT_FILENO) == 1 };
    if !is_tty {
        return None;
    }
    let rgb = query_osc11(Duration::from_millis(250))?;
    Some((
        Background::from_rgb(rgb),
        format!(
            "terminal reported background #{:02x}{:02x}{:02x}",
            rgb.0, rgb.1, rgb.2
        ),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn colorfgbg_parsing() {
        assert_eq!(from_colorfgbg("15;0"), Some(Background::Dark));
        assert_eq!(from_colorfgbg("0;15"), Some(Background::Light));
        assert_eq!(from_colorfgbg("0;default;15"), Some(Background::Light));
        assert_eq!(from_colorfgbg("7;8"), Some(Background::Dark));
        assert_eq!(from_colorfgbg("0;7"), Some(Background::Light));
        assert_eq!(from_colorfgbg("default;default"), None);
        assert_eq!(from_colorfgbg(""), None);
    }

    #[test]
    fn osc11_reply_parsing() {
        assert_eq!(
            parse_osc11_reply(b"\x1b]11;rgb:1414/1111/1010\x07"),
            Some((0x14, 0x11, 0x10))
        );
        assert_eq!(
            parse_osc11_reply(b"\x1b]11;rgb:ffff/ffff/ffff\x1b\\\x1b[?62;22c"),
            Some((255, 255, 255))
        );
        assert_eq!(
            parse_osc11_reply(b"\x1b]11;rgb:f7/f2/e8\x07"),
            Some((0xf7, 0xf2, 0xe8))
        );
        assert_eq!(
            parse_osc11_reply(b"\x1b]11;rgb:f/0/8\x07"),
            Some((255, 0, 136))
        );
        assert_eq!(parse_osc11_reply(b"\x1b[?62;22c"), None);
        assert_eq!(parse_osc11_reply(b"\x1b]11;rgb:zz/00\x07"), None);
    }

    #[test]
    fn da1_reply_detection() {
        assert!(has_da1_reply(b"\x1b[?62;22c"));
        assert!(has_da1_reply(b"\x1b]11;rgb:0/0/0\x07\x1b[?1;2c"));
        assert!(!has_da1_reply(b"\x1b[?62;22"));
        assert!(!has_da1_reply(b"\x1b]11;rgb:0/0/0\x07"));
    }

    #[test]
    fn background_classification() {
        assert_eq!(Background::from_rgb((0x14, 0x11, 0x10)), Background::Dark);
        assert_eq!(Background::from_rgb((0xf7, 0xf2, 0xe8)), Background::Light);
    }

    #[test]
    fn colorfgbg_wins_without_querying() {
        let env = TermEnv::from_pairs(&[("COLORFGBG", "0;15")]);
        assert_eq!(detect(&env, false).map(|d| d.0), Some(Background::Light));
        assert_eq!(detect(&TermEnv::from_pairs(&[]), false), None);
    }
}
