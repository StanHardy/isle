use std::sync::OnceLock;
use std::time::Duration;

use serde::{Deserialize, Serialize};

const BASE: &str = "https://lrclib.net/api";

fn client() -> &'static reqwest::Client {
    static CLIENT: OnceLock<reqwest::Client> = OnceLock::new();
    CLIENT.get_or_init(|| {
        reqwest::Client::builder()
            .user_agent("Isle/1.0 (local-first island)")
            .timeout(Duration::from_secs(10))
            .build()
            .expect("reqwest client builds")
    })
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LyricLine {
    pub t_ms: u32,
    pub text: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LyricsPayload {
    pub synced: Vec<LyricLine>,
    pub plain: Option<String>,
    pub provider: String,
}

#[derive(Deserialize)]
struct LrcLibHit {
    #[serde(default)]
    synced_lyrics: Option<String>,
    #[serde(default)]
    plain_lyrics: Option<String>,
    #[serde(default)]
    duration: Option<f64>,
}

/// Fetch lyrics: exact match first, then closest search result. None = miss.
pub async fn fetch(
    artist: &str,
    title: &str,
    duration_s: u64,
    album: &str,
) -> Result<Option<LyricsPayload>, String> {
    let mut hits = Vec::new();
    let exact = client()
        .get(format!("{BASE}/get"))
        .query(&[
            ("artist_name", artist),
            ("track_name", title),
            ("album_name", album),
            ("duration", &duration_s.to_string()),
        ])
        .send()
        .await;
    if let Ok(resp) = exact {
        if resp.status().is_success() {
            if let Ok(hit) = resp.json::<LrcLibHit>().await {
                hits.push(hit);
            }
        }
    }
    if hits.is_empty() {
        let search = client()
            .get(format!("{BASE}/search"))
            .query(&[("track_name", title), ("artist_name", artist)])
            .send()
            .await
            .and_then(|r| r.error_for_status());
        if let Ok(resp) = search {
            if let Ok(mut list) = resp.json::<Vec<LrcLibHit>>().await {
                // Prefer the hit closest to the playing duration.
                list.sort_by_key(|h| {
                    h.duration
                        .map(|d| (d - duration_s as f64).abs() as i64)
                        .unwrap_or(i64::MAX)
                });
                hits = list;
            }
        }
    }
    for hit in hits {
        if let Some(payload) = payload_from(&hit) {
            return Ok(Some(payload));
        }
    }
    Ok(None)
}

fn payload_from(hit: &LrcLibHit) -> Option<LyricsPayload> {
    if let Some(lrc) = &hit.synced_lyrics {
        let lines = parse_lrc(lrc);
        if !lines.is_empty() {
            return Some(LyricsPayload {
                synced: lines,
                plain: hit.plain_lyrics.clone(),
                provider: "LRCLIB".into(),
            });
        }
    }
    hit.plain_lyrics
        .as_ref()
        .filter(|t| !t.trim().is_empty())
        .map(|t| LyricsPayload {
            synced: Vec::new(),
            plain: Some(t.clone()),
            provider: "LRCLIB".into(),
        })
}

/// Parse "[mm:ss.xx] text" LRC lines; supports multiple stamps per line.
pub fn parse_lrc(text: &str) -> Vec<LyricLine> {
    let mut out = Vec::new();
    for line in text.lines() {
        let bytes = line.trim_start();
        if !bytes.starts_with('[') {
            continue;
        }
        // Metadata tags like [ar:...] have a colon before digits end.
        let mut stamps: Vec<u32> = Vec::new();
        let mut rest = bytes;
        loop {
            let Some(close) = rest.find(']') else { break };
            let tag = &rest[1..close];
            if let Some(ms) = parse_stamp(tag) {
                stamps.push(ms);
                rest = &rest[close + 1..];
            } else {
                break;
            }
        }
        let text = rest.trim().trim_start_matches('-').trim().to_string();
        if stamps.is_empty() || text.is_empty() {
            continue;
        }
        for ms in stamps {
            out.push(LyricLine {
                t_ms: ms,
                text: text.clone(),
            });
        }
    }
    out.sort_by_key(|l| l.t_ms);
    out.dedup_by(|a, b| a.t_ms == b.t_ms && a.text == b.text);
    out
}

/// "[mm:ss.xx]" → milliseconds.
fn parse_stamp(tag: &str) -> Option<u32> {
    let mut parts = tag.split(':');
    let mins: u32 = parts.next()?.trim().parse().ok()?;
    let secs_part = parts.next()?;
    if secs_part.is_empty() || secs_part.len() > 5 || parts.next().is_some() {
        return None;
    }
    let mut secs_iter = secs_part.split('.');
    let secs: u32 = secs_iter.next()?.trim().parse().ok()?;
    let frac_str = secs_iter.next().unwrap_or("0");
    if frac_str.len() > 2 {
        return None;
    }
    let frac: u32 = format!("{frac_str:0<2}").parse().unwrap_or(0);
    if mins > 99 || secs > 59 {
        return None;
    }
    Some(mins * 60_000 + secs * 1000 + frac * 10)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_basic_lrc() {
        let lrc = "[00:12.50]Hello world\n[00:15.00]Second line\n[ar:Artist]\n";
        let lines = parse_lrc(lrc);
        assert_eq!(lines.len(), 2);
        assert_eq!(lines[0].t_ms, 12500);
        assert_eq!(lines[0].text, "Hello world");
        assert_eq!(lines[1].t_ms, 15000);
    }

    #[test]
    fn handles_multiple_stamps_and_hyphens() {
        let lrc = "[00:01.00][00:04.00]- Instrumental hook";
        let lines = parse_lrc(lrc);
        assert_eq!(lines.len(), 2);
        assert_eq!(lines[0].text, "Instrumental hook");
    }

    #[test]
    fn rejects_bad_stamps() {
        assert_eq!(parse_lrc("[ar:Someone]Lyric"), Vec::<LyricLine>::new());
        assert_eq!(parse_lrc("[00:99]broken"), Vec::<LyricLine>::new());
    }

    #[test]
    fn stamp_roundtrip() {
        assert_eq!(parse_stamp("01:23.45"), Some(83450));
        assert_eq!(parse_stamp("00:00.5"), Some(500));
        assert_eq!(parse_stamp("59:59.99"), Some(3_599_990));
        assert_eq!(parse_stamp("aa:bb"), None);
    }
}
