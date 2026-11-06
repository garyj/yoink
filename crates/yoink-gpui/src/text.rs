//! Text shown in the list and the preview.

use std::ops::Range;

pub const TITLE_CHARS: usize = 200;
pub const PREVIEW_CHARS: usize = 20_000;

/// The text on one line: runs of whitespace, including newlines, collapse
/// to one space.
pub fn title(text: &str) -> String {
    let mut title = String::new();
    for word in text.split_whitespace() {
        if !title.is_empty() {
            title.push(' ');
        }
        title.push_str(word);
        if title.len() > TITLE_CHARS * 4 {
            break;
        }
    }
    if title.chars().count() > TITLE_CHARS {
        title = title.chars().take(TITLE_CHARS).collect::<String>() + "…";
    }
    title
}

/// Byte ranges in `text` that match `query` under the store's folding, so a
/// highlight lands where the search matched.
pub fn match_ranges(text: &str, query: &str) -> Vec<Range<usize>> {
    let needle = yoink_core::fold(query);
    if needle.is_empty() {
        return Vec::new();
    }
    let mut folded = String::new();
    // For each byte of `folded`, the source byte range it came from.
    let mut origin = Vec::new();
    for (start, ch) in text.char_indices() {
        let piece = yoink_core::fold(ch.encode_utf8(&mut [0; 4]));
        let range = start..start + ch.len_utf8();
        origin.extend(std::iter::repeat_n(range, piece.len()));
        folded.push_str(&piece);
    }
    let mut ranges: Vec<Range<usize>> = Vec::new();
    let mut from = 0;
    while let Some(found) = folded[from..].find(&needle) {
        let begin = from + found;
        let end = begin + needle.len();
        let range = origin[begin].start..origin[end - 1].end;
        match ranges.last_mut() {
            Some(last) if last.end >= range.start => last.end = last.end.max(range.end),
            _ => ranges.push(range),
        }
        from = end;
    }
    ranges
}

pub fn ago_short(now_ms: i64, then_ms: i64) -> String {
    let minutes = (now_ms - then_ms).max(0) / 60_000;
    match minutes {
        0 => "now".into(),
        1..60 => format!("{minutes}m"),
        60..1440 => format!("{}h", minutes / 60),
        1440..10080 => format!("{}d", minutes / 1440),
        10080..43200 => format!("{}w", minutes / 10080),
        43200..525600 => format!("{}mo", minutes / 43200),
        _ => format!("{}y", minutes / 525600),
    }
}

pub fn ago_long(now_ms: i64, then_ms: i64) -> String {
    let minutes = (now_ms - then_ms).max(0) / 60_000;
    let (count, unit) = match minutes {
        0 => return "just now".into(),
        1..60 => (minutes, "minute"),
        60..1440 => (minutes / 60, "hour"),
        1440..10080 => (minutes / 1440, "day"),
        10080..43200 => (minutes / 10080, "week"),
        43200..525600 => (minutes / 43200, "month"),
        _ => (minutes / 525600, "year"),
    };
    let plural = if count == 1 { "" } else { "s" };
    format!("{count} {unit}{plural} ago")
}

/// Local wall-clock time: "Today at 14:22", "Yesterday at 09:10",
/// "Mon 3 Nov at 10:30", or "3 Nov 2025".
pub fn clock(now_ms: i64, then_ms: i64) -> String {
    const DAYS: [&str; 7] = ["Sun", "Mon", "Tue", "Wed", "Thu", "Fri", "Sat"];
    const MONTHS: [&str; 12] = [
        "Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec",
    ];
    let (Some(now), Some(then)) = (local_time(now_ms), local_time(then_ms)) else {
        return String::new();
    };
    let time = format!("{:02}:{:02}", then.tm_hour, then.tm_min);
    let month = MONTHS[then.tm_mon.clamp(0, 11) as usize];
    let day_gap = if now.tm_year == then.tm_year {
        now.tm_yday - then.tm_yday
    } else {
        i32::MAX
    };
    match day_gap {
        0 => format!("Today at {time}"),
        1 => format!("Yesterday at {time}"),
        2..=6 => format!("{} at {time}", DAYS[then.tm_wday.clamp(0, 6) as usize]),
        _ if now.tm_year == then.tm_year => format!("{} {month} at {time}", then.tm_mday),
        _ => format!("{} {month} {}", then.tm_mday, then.tm_year + 1900),
    }
}

fn local_time(ms: i64) -> Option<libc::tm> {
    let seconds = (ms / 1000) as libc::time_t;
    // SAFETY: localtime_r writes only into the zeroed `tm` it is given.
    unsafe {
        let mut tm: libc::tm = std::mem::zeroed();
        (!libc::localtime_r(&seconds, &mut tm).is_null()).then_some(tm)
    }
}

/// `#rgb` or `#rrggbb`, as 0xRRGGBB.
pub fn hex_color(text: &str) -> Option<u32> {
    let hex = text.trim().strip_prefix('#')?;
    if !hex.chars().all(|c| c.is_ascii_hexdigit()) {
        return None;
    }
    match hex.len() {
        6 => u32::from_str_radix(hex, 16).ok(),
        3 => {
            let v = u32::from_str_radix(hex, 16).ok()?;
            let (r, g, b) = ((v >> 8) & 0xf, (v >> 4) & 0xf, v & 0xf);
            Some((r * 17) << 16 | (g * 17) << 8 | (b * 17))
        }
        _ => None,
    }
}

/// A link as its host and last path segment, such as
/// "github.com/…/state.rs", so links stay distinct in a narrow list.
pub fn link_title(link: &str) -> String {
    let link = link.trim();
    let bare = link
        .strip_prefix("https://")
        .or_else(|| link.strip_prefix("http://"))
        .unwrap_or(link);
    let bare = bare.strip_prefix("www.").unwrap_or(bare);
    let bare = bare.split(['?', '#']).next().unwrap_or(bare);
    let mut parts = bare.split('/').filter(|part| !part.is_empty());
    let host = parts.next().unwrap_or(bare);
    let segments: Vec<&str> = parts.collect();
    match segments.as_slice() {
        [] => host.to_string(),
        [only] => format!("{host}/{only}"),
        [.., last] => format!("{host}/…/{last}"),
    }
}

pub fn is_link(text: &str) -> bool {
    let text = text.trim();
    (text.starts_with("https://") || text.starts_with("http://"))
        && !text.chars().any(char::is_whitespace)
}

pub fn thousands(n: usize) -> String {
    let digits = n.to_string();
    let mut out = String::new();
    for (i, c) in digits.chars().enumerate() {
        if i > 0 && (digits.len() - i).is_multiple_of(3) {
            out.push(',');
        }
        out.push(c);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn highlights_follow_accent_and_case_folding() {
        let text = "Café notes: CAFE au lait";
        let ranges = match_ranges(text, "cafe");
        let matched: Vec<&str> = ranges.iter().map(|r| &text[r.clone()]).collect();
        assert_eq!(matched, ["Café", "CAFE"]);
        assert!(match_ranges(text, "").is_empty());
        assert_eq!(
            title("{\n  \"name\":\t\"yoink\"\n}\n"),
            r#"{ "name": "yoink" }"#
        );
        assert_eq!(hex_color("#f90"), Some(0xff9900));
        assert_eq!(hex_color("#ff9900"), Some(0xff9900));
        assert_eq!(hex_color("ff9900"), None);
        assert_eq!(
            link_title("https://github.com/longbridge/gpui-kit/blob/main/state.rs?plain=1"),
            "github.com/…/state.rs"
        );
    }
}
