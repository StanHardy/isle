use std::sync::OnceLock;
use std::time::Duration;

use serde::{Deserialize, Serialize};

use crate::media::MediaState;

const TMDB_BASE: &str = "https://api.themoviedb.org/3";
const TVMAZE_BASE: &str = "https://api.tvmaze.com";
const ITUNES_BASE: &str = "https://itunes.apple.com/search";

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

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MediaInfo {
    pub title: String,
    pub overview: String,
    pub poster: String,
    pub provider: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SearchHit {
    pub title: String,
    pub year: String,
    pub overview: String,
    pub poster: String,
    pub provider: String,
}

// ---------- pure parsing helpers (unit tested) ----------

/// "Suits S03E14 - Heartburn" → ("Suits", 3, 14)
pub fn parse_episode_title(title: &str) -> Option<(String, u32, u32)> {
    let bytes = title.as_bytes();
    let mut i = 0;
    while i + 1 < bytes.len() {
        let b = bytes[i];
        let next = bytes[i + 1];
        let is_s = b == b's' || b == b'S';
        let prev_boundary = i == 0 || !bytes[i - 1].is_ascii_alphanumeric();
        if is_s && prev_boundary && next.is_ascii_digit() {
            let mut j = i + 1;
            while j < bytes.len() && bytes[j].is_ascii_digit() && j - i <= 2 {
                j += 1;
            }
            if j < bytes.len() && (bytes[j] == b'e' || bytes[j] == b'E') {
                let mut k = j + 1;
                while k < bytes.len() && bytes[k].is_ascii_digit() && k - j <= 3 {
                    k += 1;
                }
                if k > j + 1 {
                    let season: u32 = title[i + 1..j].parse().ok()?;
                    let ep: u32 = title[j + 1..k].parse().ok()?;
                    if season > 0 && ep > 0 {
                        let show = clean_show(&title[..i]);
                        if !show.is_empty() {
                            return Some((show, season, ep));
                        }
                    }
                }
            }
        }
        i += 1;
    }
    None
}

/// Strip quality/release tags and normalize separators.
pub fn clean_show(raw: &str) -> String {
    let junk = [
        "1080p", "720p", "480p", "2160p", "web-dl", "webdl", "webrip", "web", "bluray", "brrip",
        "bdrip", "dvdrip", "hdtv", "x264", "x265", "h264", "h.264", "h265", "hevc", "aac5.1",
        "aac2.0", "aac", "ac3", "eac3", "ddp5.1", "dd5.1", "dd+", "hdr", "hdr10", "dolby",
        "proper", "repack", "extended", "remastered", "internal", "dual", "aud",
    ];
    let normalized = raw.replace(['.', '_'], " ");
    let mut kept = Vec::new();
    for word in normalized.split_whitespace() {
        let lower = word.to_lowercase();
        let lower = lower.trim_matches(['(', ')', '[', ']', '-']);
        if lower.is_empty() || junk.contains(&lower) {
            continue;
        }
        kept.push(word);
    }
    kept.join(" ").trim().to_string()
}

/// Extract a leading year "(2019)" and return (rest, year).
pub fn split_year(title: &str) -> (String, Option<u32>) {
    let t = title.trim();
    if t.len() >= 6 && t.ends_with(')') {
        if let Some(open) = t.rfind('(') {
            let inside = &t[open + 1..t.len() - 1];
            if inside.len() == 4 && inside.chars().all(|c| c.is_ascii_digit()) {
                return (
                    t[..open].trim().to_string(),
                    inside.parse::<u32>().ok(),
                );
            }
        }
    }
    (t.to_string(), None)
}

pub fn strip_html(html: &str) -> String {
    let mut out = String::with_capacity(html.len());
    let mut in_tag = false;
    let mut chars = html.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '<' => in_tag = true,
            '>' => in_tag = false,
            _ if !in_tag => {
                if c == '&' {
                    let rest: String = chars.clone().take(10).collect();
                    let mut consumed = 0usize;
                    for (entity, rep) in [
                        ("amp;", '&'),
                        ("quot;", '"'),
                        ("lt;", '<'),
                        ("gt;", '>'),
                        ("nbsp;", ' '),
                    ] {
                        if rest.starts_with(entity) {
                            out.push(rep);
                            consumed = entity.len();
                            break;
                        }
                    }
                    if consumed == 0 {
                        if let Some(code) = parse_numeric_entity(&rest) {
                            if let Some(ch) = char::from_u32(code) {
                                out.push(ch);
                            }
                            consumed = rest.find(';').map(|i| i + 1).unwrap_or(0);
                        }
                    }
                    for _ in 0..consumed {
                        chars.next();
                    }
                    if consumed == 0 {
                        out.push('&');
                    }
                } else {
                    out.push(c);
                }
            }
            _ => {}
        }
    }
    out.trim().to_string()
}

/// "&#x27;" / "&#39;" → code point.
fn parse_numeric_entity(rest: &str) -> Option<u32> {
    let digits = rest.strip_prefix("#x").or_else(|| rest.strip_prefix("#X"))?;
    let end = digits.find(';')?;
    let hex = &digits[..end];
    if hex.is_empty() || hex.len() > 6 || !hex.chars().all(|c| c.is_ascii_hexdigit()) {
        return None;
    }
    u32::from_str_radix(hex, 16).ok()
}

// ---------- network lookups ----------

/// Resolve info for the playing media: TV via TVmaze (keyless), movies via
/// TMDB when the user configured a key, else keyless iTunes fallback.
pub async fn lookup_auto(media: &MediaState, tmdb_key: Option<&str>) -> Option<MediaInfo> {
    let query = format!("{} {}", media.title, media.artist);
    if let Some((show, season, ep)) = parse_episode_title(&query) {
        return lookup_tv(&show, season, ep).await.ok().flatten();
    }
    let (title, year) = split_year(&media.title);
    let title = if title.trim().is_empty() { media.title.clone() } else { title };
    if let Some(key) = tmdb_key {
        if let Ok(Some(info)) = lookup_movie_tmdb(&title, year, key).await {
            return Some(info);
        }
    }
    lookup_movie_itunes(&title).await.ok().flatten()
}

#[derive(Deserialize)]
struct TvmazeShow {
    #[serde(default)]
    name: String,
    #[serde(default)]
    image: Option<TvmazeImage>,
    #[serde(default)]
    summary: Option<String>,
    #[serde(default)]
    #[serde(rename = "_embedded")]
    embedded: Option<TvmazeEmbedded>,
}

#[derive(Deserialize)]
struct TvmazeImage {
    #[serde(default)]
    medium: Option<String>,
    #[serde(default)]
    original: Option<String>,
}

#[derive(Deserialize)]
struct TvmazeEmbedded {
    #[serde(default, rename = "episodes")]
    episodes: Vec<TvmazeEpisode>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct TvmazeEpisode {
    name: Option<String>,
    season: u32,
    number: u32,
    #[serde(default)]
    summary: Option<String>,
}

pub async fn lookup_tv(show: &str, season: u32, ep: u32) -> Result<Option<MediaInfo>, String> {
    let resp = client()
        .get(format!("{TVMAZE_BASE}/singlesearch/shows"))
        .query(&[("q", show), ("embed", "episodes")])
        .send()
        .await
        .and_then(|r| r.error_for_status())
        .map_err(|e| e.to_string())?;
    let show_data: TvmazeShow = resp.json().await.map_err(|e| e.to_string())?;
    let poster = show_data
        .image
        .as_ref()
        .and_then(|i| i.original.clone().or_else(|| i.medium.clone()))
        .unwrap_or_default();
    let episode = show_data
        .embedded
        .as_ref()
        .and_then(|e| {
            e.episodes
                .iter()
                .find(|e| e.season == season && e.number == ep)
        });
    let overview = episode
        .and_then(|e| e.summary.clone())
        .or(show_data.summary.clone())
        .map(|s| strip_html(&s))
        .unwrap_or_default();
    let title = match episode.and_then(|e| e.name.clone()) {
        Some(name) => format!("{} · S{season:02}E{ep:02} — {name}", show_data.name),
        None => format!("{} · S{season:02}E{ep:02}", show_data.name),
    };
    if title.trim().is_empty() {
        return Ok(None);
    }
    Ok(Some(MediaInfo {
        title,
        overview,
        poster,
        provider: "TVmaze".into(),
    }))
}

#[derive(Deserialize)]
struct TmdbSearch {
    results: Option<Vec<TmdbHit>>,
}

#[derive(Deserialize)]
#[serde(rename_all = "snake_case")]
struct TmdbHit {
    title: Option<String>,
    name: Option<String>,
    overview: Option<String>,
    poster_path: Option<String>,
    release_date: Option<String>,
}

pub async fn lookup_movie_tmdb(
    title: &str,
    year: Option<u32>,
    key: &str,
) -> Result<Option<MediaInfo>, String> {
    let mut query = vec![
        ("query".to_string(), title.to_string()),
        ("api_key".to_string(), key.to_string()),
    ];
    if let Some(y) = year {
        query.push(("year".to_string(), y.to_string()));
    }
    let resp = client()
        .get(format!("{TMDB_BASE}/search/movie"))
        .query(&query)
        .send()
        .await
        .and_then(|r| r.error_for_status())
        .map_err(|e| e.to_string())?
        .json::<TmdbSearch>()
        .await
        .map_err(|e| e.to_string())?;
    let hit = resp.results.unwrap_or_default().into_iter().next();
    Ok(hit.map(|h| MediaInfo {
        title: h.title.unwrap_or_else(|| title.to_string()),
        overview: h.overview.unwrap_or_default(),
        poster: h
            .poster_path
            .map(|p| format!("https://image.tmdb.org/t/p/w342{p}"))
            .unwrap_or_default(),
        provider: "TMDB".into(),
    }))
}

/// Manual lookup for the search UI. kind: "tv" | "movie".
pub async fn search(kind: &str, query: &str, tmdb_key: Option<&str>) -> Result<Vec<SearchHit>, String> {
    match kind {
        "tv" => {
            let resp = client()
                .get(format!("{TVMAZE_BASE}/searchshows"))
                .query(&[("q", query)])
                .send()
                .await
                .and_then(|r| r.error_for_status())
                .map_err(|e| e.to_string())?
                .json::<Vec<TvmazeSearchWrap>>()
                .await
                .map_err(|e| e.to_string())?;
            Ok(resp
                .into_iter()
                .filter_map(|w| w.show)
                .map(|s| SearchHit {
                    year: s.premiered.unwrap_or_default().chars().take(4).collect(),
                    title: s.name,
                    overview: s.summary.map(|x| strip_html(&x)).unwrap_or_default(),
                    poster: s.image.and_then(|i| i.medium.or(i.original)).unwrap_or_default(),
                    provider: "TVmaze".into(),
                })
                .collect())
        }
        _ => {
            if let Some(key) = tmdb_key {
                let resp = client()
                    .get(format!("{TMDB_BASE}/search/movie"))
                    .query(&[("query", query), ("api_key", key)])
                    .send()
                    .await
                    .and_then(|r| r.error_for_status())
                    .map_err(|e| e.to_string())?
                    .json::<TmdbSearch>()
                    .await
                    .map_err(|e| e.to_string())?;
                Ok(resp
                    .results
                    .unwrap_or_default()
                    .into_iter()
                    .map(|h| SearchHit {
                        title: h.title.unwrap_or_default(),
                        year: h
                            .release_date
                            .unwrap_or_default()
                            .chars()
                            .take(4)
                            .collect(),
                        overview: h.overview.unwrap_or_default(),
                        poster: h
                            .poster_path
                            .map(|p| format!("https://image.tmdb.org/t/p/w342{p}"))
                            .unwrap_or_default(),
                        provider: "TMDB".into(),
                    })
                    .collect())
            } else {
                search_movie_itunes(query).await
            }
        }
    }
}

#[derive(Deserialize)]
struct TvmazeSearchWrap {
    #[serde(default)]
    show: Option<TvmazeShowHit>,
}

#[derive(Deserialize)]
struct TvmazeShowHit {
    #[serde(default)]
    name: String,
    #[serde(default)]
    premiered: Option<String>,
    #[serde(default)]
    summary: Option<String>,
    #[serde(default)]
    image: Option<TvmazeImage>,
}

#[derive(Deserialize)]
struct ItunesSearch {
    results: Option<Vec<ItunesHit>>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct ItunesHit {
    track_name: Option<String>,
    long_description: Option<String>,
    short_description: Option<String>,
    artwork_url_100: Option<String>,
    release_date: Option<String>,
}

pub async fn lookup_movie_itunes(title: &str) -> Result<Option<MediaInfo>, String> {
    let hits = search_movie_itunes(title).await?;
    Ok(hits.into_iter().next().map(|hit| MediaInfo {
        title: hit.title,
        overview: hit.overview,
        poster: hit.poster,
        provider: hit.provider,
    }))
}

async fn search_movie_itunes(query: &str) -> Result<Vec<SearchHit>, String> {
    let resp = client()
        .get(ITUNES_BASE)
        .query(&[("term", query), ("entity", "movie"), ("limit", "5")])
        .send()
        .await
        .and_then(|r| r.error_for_status())
        .map_err(|e| e.to_string())?
        .json::<ItunesSearch>()
        .await
        .map_err(|e| e.to_string())?;
    Ok(resp
        .results
        .unwrap_or_default()
        .into_iter()
        .map(|h| SearchHit {
            title: h.track_name.unwrap_or_default(),
            year: h
                .release_date
                .unwrap_or_default()
                .chars()
                .take(4)
                .collect(),
            overview: h
                .long_description
                .or(h.short_description)
                .unwrap_or_default(),
            poster: h
                .artwork_url_100
                .map(|u| u.replace("100x100", "600x600"))
                .unwrap_or_default(),
            provider: "iTunes".into(),
        })
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_episode_patterns() {
        assert_eq!(
            parse_episode_title("Suits S03E14 - Heartburn"),
            Some(("Suits".into(), 3, 14))
        );
        assert_eq!(
            parse_episode_title("show.name.S02E05.720p.WEB-DL"),
            Some(("show name".into(), 2, 5))
        );
        assert_eq!(parse_episode_title("Season 3 episode 14"), None);
        assert_eq!(parse_episode_title("ASAP S1E1"), Some(("ASAP".into(), 1, 1)));
    }

    #[test]
    fn strips_quality_tags() {
        assert_eq!(clean_show("Show.Name.1080p.WEB-DL.x264"), "Show Name");
        assert_eq!(clean_show("Normal Title"), "Normal Title");
    }

    #[test]
    fn splits_years() {
        assert_eq!(split_year("Blade Runner 2049"), ("Blade Runner 2049".into(), None));
        assert_eq!(split_year("Arrival (2016)"), ("Arrival".into(), Some(2016)));
    }

    #[test]
    fn strips_html_entities_and_tags() {
        assert_eq!(
            strip_html("<p>Mike &amp; Harvey</p>"),
            "Mike & Harvey"
        );
        assert_eq!(strip_html("<b>Bold</b> &#x27;quoted&#x27;"), "Bold 'quoted'");
    }
}
