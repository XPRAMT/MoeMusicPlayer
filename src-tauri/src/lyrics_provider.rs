//! Bounded, unauthenticated adapters for remote line-synced lyric providers.
//!
//! This module only searches and fetches provider data. Candidate ranking, matching, parsing,
//! persistence, and UI policy belong to the lyrics service/Core.

use std::{fmt, time::Duration};

use base64::{engine::general_purpose, Engine as _};
use player_core::lyrics::{LyricProvider, LyricsTrackMetadata};
use reqwest::{Client, Method};
use serde_json::{json, Value};
use tokio_util::sync::CancellationToken;

pub const MAX_RESPONSE_BYTES: usize = 512 * 1024;
const REQUEST_TIMEOUT: Duration = Duration::from_secs(5);
const NETEASE_SEARCH_URL: &str = "https://music.163.com/api/search/get/web";
const NETEASE_LYRIC_URL: &str = "https://music.163.com/api/song/lyric";
const QQ_API_URL: &str = "https://u.y.qq.com/cgi-bin/musicu.fcg";
const QQ_LYRIC_URL: &str = "https://c.y.qq.com/lyric/fcgi-bin/fcg_query_lyric_new.fcg";
const NETEASE_CLOUD_SEARCH_URL: &str = "https://music.163.com/api/cloudsearch/pc";
const NETEASE_SEARCH_LIMIT: usize = 5;
const QQ_SEARCH_LIMIT: usize = 20;
const QQ_REFERER: &str = "https://y.qq.com/portal/player.html";
const QQ_USER_AGENT: &str =
    "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 Chrome/124.0.0.0 Safari/537.36";

/// Search result metadata. `candidate_id` is stable across search and fetch calls.
#[derive(Clone, Debug, PartialEq)]
pub struct ProviderCandidate {
    pub candidate_id: String,
    pub provider: LyricProvider,
    pub provider_track_id: String,
    pub title: Option<String>,
    pub artist: Option<String>,
    pub album: Option<String>,
    pub duration_ms: Option<u64>,
    /// Trace-only provider URL. Callers should not expose it in UI or persist it as cache data.
    pub source_url: String,
    /// QQ's lyric endpoint requires the numeric song ID in addition to songmid.
    pub(crate) qq_song_id: Option<u64>,
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct RawKaraokeData {
    /// Canonical Base64 QRC bytes; plain text responses are encoded and no QRC is decrypted.
    pub qrc_base64: Option<String>,
    /// Canonical Base64 encoding of the provider translation payload.
    pub translation_base64: Option<String>,
    /// Canonical Base64 encoding of the provider romanization payload.
    pub romanization_base64: Option<String>,
}

/// Raw provider result passed to the matching/persistence service for parsing and ranking.
#[derive(Clone, Debug, PartialEq)]
pub struct RawLyricCandidate {
    pub candidate_id: String,
    pub provider: LyricProvider,
    pub provider_track_id: String,
    pub title: Option<String>,
    pub artist: Option<String>,
    pub album: Option<String>,
    pub duration_ms: Option<u64>,
    pub source_url: String,
    pub synced_lyrics: Option<String>,
    pub plain_lyrics: Option<String>,
    pub translation: Option<String>,
    pub romanization: Option<String>,
    /// NetEase's original word-timed payload, retained even when line-synced LRC is preferred.
    pub raw_yrc: Option<String>,
    pub raw_karaoke: Option<RawKaraokeData>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ProviderError {
    Cancelled,
    Timeout,
    ResponseTooLarge,
    Transport(String),
    HttpStatus(u16),
    InvalidResponse(String),
    UnsupportedProvider,
}

impl fmt::Display for ProviderError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Cancelled => write!(formatter, "lyrics request was cancelled"),
            Self::Timeout => write!(formatter, "lyrics provider request timed out"),
            Self::ResponseTooLarge => write!(formatter, "lyrics provider response exceeds 512 KiB"),
            Self::Transport(message) => write!(formatter, "lyrics transport failed: {message}"),
            Self::HttpStatus(status) => write!(formatter, "lyrics provider returned HTTP {status}"),
            Self::InvalidResponse(message) => {
                write!(formatter, "invalid lyrics response: {message}")
            }
            Self::UnsupportedProvider => {
                write!(formatter, "provider does not support remote search")
            }
        }
    }
}

impl std::error::Error for ProviderError {}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct HttpRequest {
    pub method: HttpMethod,
    pub url: String,
    pub body: Option<String>,
    pub headers: Vec<(String, String)>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum HttpMethod {
    Get,
    Post,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct HttpResponse {
    pub status: u16,
    pub body: Vec<u8>,
}

/// Injectable boundary used by fixture tests and the production Reqwest transport.
pub trait HttpTransport: Send + Sync {
    fn send<'a>(
        &'a self,
        request: HttpRequest,
        cancellation: &'a CancellationToken,
    ) -> impl std::future::Future<Output = Result<HttpResponse, ProviderError>> + Send + 'a;
}

#[derive(Clone)]
pub struct ReqwestHttpTransport {
    client: Client,
}

impl Default for ReqwestHttpTransport {
    fn default() -> Self {
        Self {
            // No cookie store or authentication is configured.
            client: Client::builder()
                .connect_timeout(REQUEST_TIMEOUT)
                .build()
                .expect("reqwest client configuration is valid"),
        }
    }
}

impl HttpTransport for ReqwestHttpTransport {
    fn send<'a>(
        &'a self,
        request: HttpRequest,
        cancellation: &'a CancellationToken,
    ) -> impl std::future::Future<Output = Result<HttpResponse, ProviderError>> + Send + 'a {
        async move {
            let mut builder = match request.method {
                HttpMethod::Get => self.client.request(Method::GET, &request.url),
                HttpMethod::Post => self.client.request(Method::POST, &request.url),
            };
            if let Some(body) = request.body {
                builder = builder
                    .header(reqwest::header::CONTENT_TYPE, "application/json")
                    .body(body);
            }
            for (name, value) in request.headers {
                builder = builder.header(name, value);
            }

            let mut response = tokio::select! {
                _ = cancellation.cancelled() => return Err(ProviderError::Cancelled),
                result = builder.send() => result.map_err(|error| ProviderError::Transport(error.to_string()))?,
            };
            let status = response.status().as_u16();
            let mut body = Vec::new();
            loop {
                let chunk = tokio::select! {
                    _ = cancellation.cancelled() => return Err(ProviderError::Cancelled),
                    result = response.chunk() => result.map_err(|error| ProviderError::Transport(error.to_string()))?,
                };
                let Some(chunk) = chunk else { break };
                if body.len().saturating_add(chunk.len()) > MAX_RESPONSE_BYTES {
                    return Err(ProviderError::ResponseTooLarge);
                }
                body.extend_from_slice(&chunk);
            }
            Ok(HttpResponse { status, body })
        }
    }
}

pub struct LyricsProviderClient<T = ReqwestHttpTransport> {
    transport: T,
}

impl Default for LyricsProviderClient<ReqwestHttpTransport> {
    fn default() -> Self {
        Self::new(ReqwestHttpTransport::default())
    }
}

impl<T: HttpTransport> LyricsProviderClient<T> {
    pub fn new(transport: T) -> Self {
        Self { transport }
    }

    /// Return raw search hits in provider order. This method deliberately does not dedupe or rank.
    pub async fn search(
        &self,
        provider: LyricProvider,
        metadata: &LyricsTrackMetadata,
        cancellation: &CancellationToken,
    ) -> Result<Vec<ProviderCandidate>, ProviderError> {
        match provider {
            LyricProvider::NetEase => self.search_netease(metadata, cancellation).await,
            LyricProvider::Qq => {
                let query = search_query(metadata);
                if query.is_empty() {
                    return Ok(Vec::new());
                }
                let request = HttpRequest {
                    method: HttpMethod::Post,
                    url: QQ_API_URL.to_owned(),
                    body: Some(qq_search_body(&query)),
                    headers: qq_headers(),
                };
                let response = self.request(request, cancellation).await?;
                parse_qq_search(&parse_json(&response.body)?)
            }
            LyricProvider::Sidecar | LyricProvider::Embedded => {
                Err(ProviderError::UnsupportedProvider)
            }
        }
    }

    /// Fetch lyrics for a selected search hit. The caller controls how many hits are fetched.
    pub async fn fetch_lyrics(
        &self,
        candidate: &ProviderCandidate,
        cancellation: &CancellationToken,
    ) -> Result<RawLyricCandidate, ProviderError> {
        let request = match candidate.provider {
            LyricProvider::NetEase => HttpRequest {
                method: HttpMethod::Get,
                url: format!(
                    "{NETEASE_LYRIC_URL}?id={}&lv=1&kv=1&tv=1&yv=1&rv=1",
                    form_component(&candidate.provider_track_id)
                ),
                body: None,
                headers: netease_headers(),
            },
            LyricProvider::Qq => HttpRequest {
                method: HttpMethod::Get,
                url: format!(
                    "{QQ_LYRIC_URL}?songmid={}&format=json&nobase64=1&songid={}&qrc=1&trans=1&roma=1",
                    form_component(&candidate.provider_track_id),
                    candidate.qq_song_id.unwrap_or_default()
                ),
                body: None,
                headers: qq_headers(),
            },
            LyricProvider::Sidecar | LyricProvider::Embedded => {
                return Err(ProviderError::UnsupportedProvider)
            }
        };
        let response = self.request(request, cancellation).await?;
        let json = parse_json(&response.body)?;
        match candidate.provider {
            LyricProvider::NetEase => parse_netease_lyrics(candidate, &json),
            LyricProvider::Qq => parse_qq_lyrics(candidate, &json),
            LyricProvider::Sidecar | LyricProvider::Embedded => {
                Err(ProviderError::UnsupportedProvider)
            }
        }
    }

    async fn search_netease(
        &self,
        metadata: &LyricsTrackMetadata,
        cancellation: &CancellationToken,
    ) -> Result<Vec<ProviderCandidate>, ProviderError> {
        let Some(title) = metadata
            .title
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
        else {
            return Ok(Vec::new());
        };
        let mut variants = Vec::with_capacity(2);
        if let Some(artist) = metadata
            .artist
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
        {
            variants.push(format!("{title} {artist}"));
        }
        if variants.last().is_none_or(|query| query != title) {
            variants.push(title.to_owned());
        }

        let mut results = Vec::new();
        for query in variants {
            let primary = self
                .netease_search_request(NETEASE_SEARCH_URL, &query, cancellation)
                .await;
            let primary_hits = match primary {
                Ok(response) => {
                    match parse_json(&response.body).and_then(|json| parse_netease_search(&json)) {
                        Ok(hits) => Some(hits),
                        Err(error) if is_fallback_error(&error) => None,
                        Err(error) => return Err(error),
                    }
                }
                Err(ProviderError::Cancelled) => return Err(ProviderError::Cancelled),
                Err(error) if is_fallback_error(&error) => None,
                Err(error) => return Err(error),
            };
            let mut hits = primary_hits.unwrap_or_default();
            if hits.is_empty() {
                let fallback = self
                    .netease_search_request(NETEASE_CLOUD_SEARCH_URL, &query, cancellation)
                    .await?;
                hits = parse_netease_search(&parse_json(&fallback.body)?)?;
            }
            results.extend(hits);
        }
        Ok(results)
    }

    async fn netease_search_request(
        &self,
        endpoint: &str,
        query: &str,
        cancellation: &CancellationToken,
    ) -> Result<HttpResponse, ProviderError> {
        self.request(
            HttpRequest {
                method: HttpMethod::Get,
                url: format!(
                    "{endpoint}?s={}&type=1&limit={NETEASE_SEARCH_LIMIT}&offset=0",
                    form_component(query)
                ),
                body: None,
                headers: netease_headers(),
            },
            cancellation,
        )
        .await
    }

    async fn request(
        &self,
        request: HttpRequest,
        cancellation: &CancellationToken,
    ) -> Result<HttpResponse, ProviderError> {
        tokio::select! {
            _ = cancellation.cancelled() => Err(ProviderError::Cancelled),
            result = tokio::time::timeout(REQUEST_TIMEOUT, self.transport.send(request, cancellation)) => {
                match result {
                    Err(_) => Err(ProviderError::Timeout),
                    Ok(Err(error)) => Err(error),
                    Ok(Ok(response)) if response.body.len() > MAX_RESPONSE_BYTES => Err(ProviderError::ResponseTooLarge),
                    Ok(Ok(response)) if !(200..300).contains(&response.status) => Err(ProviderError::HttpStatus(response.status)),
                    Ok(Ok(response)) => Ok(response),
                }
            }
        }
    }
}

fn search_query(metadata: &LyricsTrackMetadata) -> String {
    [metadata.title.as_deref(), metadata.artist.as_deref()]
        .into_iter()
        .flatten()
        .map(str::trim)
        .filter(|part| !part.is_empty())
        .collect::<Vec<_>>()
        .join(" ")
}

fn form_component(value: &str) -> String {
    let mut output = String::with_capacity(value.len());
    for byte in value.bytes() {
        if byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.' | b'~') {
            output.push(byte as char);
        } else if byte == b' ' {
            output.push('+');
        } else {
            use fmt::Write as _;
            let _ = write!(output, "%{byte:02X}");
        }
    }
    output
}

fn qq_search_body(query: &str) -> String {
    json!({
        "comm": {"ct": 24, "cv": 0},
        "req_1": {
            "module": "music.search.SearchCgiService",
            "method": "DoSearchForQQMusicDesktop",
            "param": {
                "query": query,
                "search_type": 0,
                "num_per_page": QQ_SEARCH_LIMIT,
                "page_num": 1
            }
        }
    })
    .to_string()
}

fn qq_headers() -> Vec<(String, String)> {
    vec![
        ("Referer".into(), QQ_REFERER.into()),
        ("Origin".into(), "https://y.qq.com".into()),
        ("User-Agent".into(), QQ_USER_AGENT.into()),
        ("Accept".into(), "application/json, text/plain, */*".into()),
        (
            "Content-Type".into(),
            "application/json;charset=UTF-8".into(),
        ),
    ]
}

fn netease_headers() -> Vec<(String, String)> {
    vec![("Referer".into(), "https://music.163.com/".into())]
}

fn parse_json(bytes: &[u8]) -> Result<Value, ProviderError> {
    serde_json::from_slice(bytes).map_err(|error| ProviderError::InvalidResponse(error.to_string()))
}

fn ensure_provider_ok(root: &Value) -> Result<(), ProviderError> {
    // NetEase uses 200 for successful API responses; QQ's wrapped API uses 0 or omits `code`.
    if root
        .get("code")
        .and_then(Value::as_i64)
        .is_some_and(|code| !matches!(code, 0 | 200))
    {
        return Err(ProviderError::InvalidResponse(
            "provider reported an error".into(),
        ));
    }
    Ok(())
}

fn is_fallback_error(error: &ProviderError) -> bool {
    matches!(
        error,
        ProviderError::Timeout
            | ProviderError::Transport(_)
            | ProviderError::HttpStatus(_)
            | ProviderError::InvalidResponse(_)
    )
}

fn parse_netease_search(root: &Value) -> Result<Vec<ProviderCandidate>, ProviderError> {
    ensure_provider_ok(root)?;
    let songs = root
        .pointer("/result/songs")
        .and_then(Value::as_array)
        .or_else(|| root.pointer("/data/result/songs").and_then(Value::as_array))
        .ok_or_else(|| ProviderError::InvalidResponse("NetEase search has no song list".into()))?;
    Ok(songs
        .iter()
        .take(NETEASE_SEARCH_LIMIT)
        .filter_map(|song| {
            let id = value_string(song.get("id"))?;
            let artists = song.get("artists").or_else(|| song.get("ar"));
            let artist = artists
                .and_then(Value::as_array)
                .map(|items| {
                    items
                        .iter()
                        .filter_map(|item| value_string(item.get("name")))
                        .collect::<Vec<_>>()
                        .join(" / ")
                })
                .filter(|value| !value.is_empty());
            let album = song
                .get("album")
                .or_else(|| song.get("al"))
                .and_then(|item| value_string(item.get("name")));
            Some(ProviderCandidate {
                candidate_id: format!("netease:{id}"),
                provider: LyricProvider::NetEase,
                provider_track_id: id.clone(),
                title: value_string(song.get("name")),
                artist,
                album,
                duration_ms: value_u64(song.get("duration").or_else(|| song.get("dt"))),
                source_url: format!("https://music.163.com/#/song?id={id}"),
                qq_song_id: None,
            })
        })
        .collect())
}

fn parse_qq_search(root: &Value) -> Result<Vec<ProviderCandidate>, ProviderError> {
    ensure_provider_ok(root)?;
    let songs = root
        .pointer("/req_1/data/body/song/list")
        .or_else(|| root.pointer("/req_1/data/body/song/itemlist"))
        .and_then(Value::as_array)
        .ok_or_else(|| ProviderError::InvalidResponse("QQ search has no song list".into()))?;
    Ok(songs
        .iter()
        .take(QQ_SEARCH_LIMIT)
        .filter_map(|song| {
            let mid = value_string(song.get("songmid"))?;
            let song_id = value_u64(song.get("songid"));
            let artist = song
                .get("singer")
                .and_then(Value::as_array)
                .map(|items| {
                    items
                        .iter()
                        .filter_map(|item| value_string(item.get("name")))
                        .collect::<Vec<_>>()
                        .join(" / ")
                })
                .filter(|value| !value.is_empty());
            let album = value_string(song.get("albumname"));
            Some(ProviderCandidate {
                candidate_id: format!("qqmusic:{mid}"),
                provider: LyricProvider::Qq,
                provider_track_id: mid.clone(),
                title: value_string(song.get("songname")),
                artist,
                album,
                duration_ms: value_u64(song.get("interval"))
                    .and_then(|seconds| seconds.checked_mul(1000)),
                source_url: format!("https://y.qq.com/n/ryqq/songDetail/{mid}"),
                qq_song_id: song_id,
            })
        })
        .collect())
}

fn parse_netease_lyrics(
    candidate: &ProviderCandidate,
    root: &Value,
) -> Result<RawLyricCandidate, ProviderError> {
    ensure_provider_ok(root)?;
    let yrc = nested_lyric(root, "yrc");
    let lrc = nested_lyric(root, "lrc");
    let raw_yrc = yrc.clone();
    let (synced_lyrics, lrc_plain) = match lrc {
        Some(lrc) if contains_lrc_timestamp(&lrc) => (Some(lrc), None),
        Some(lrc) => (yrc, Some(lrc)),
        None => (yrc, None),
    };
    let plain_lyrics = nested_lyric(root, "plainlyric").or(lrc_plain);
    let translation = nested_lyric(root, "tlyric");
    let romanization = nested_lyric(root, "romalrc");
    Ok(raw_candidate(
        candidate,
        synced_lyrics,
        plain_lyrics,
        translation,
        romanization,
        raw_yrc,
        None,
    ))
}

fn parse_qq_lyrics(
    candidate: &ProviderCandidate,
    root: &Value,
) -> Result<RawLyricCandidate, ProviderError> {
    ensure_provider_ok(root)?;
    let lyric = qq_song_lyric_payload(root)?;
    let normal = provider_text(lyric.get("lyric")).filter(|text| !text.is_empty());
    let plain = normal
        .as_deref()
        .filter(|text| !contains_lrc_timestamp(text))
        .map(str::to_owned);
    let synced = normal.filter(|text| contains_lrc_timestamp(&text));
    let qrc = normalized_raw_payload(lyric.get("qrc").or_else(|| lyric.get("qrc_content")));
    let translation = normalized_raw_payload(lyric.get("trans").or_else(|| lyric.get("transl")));
    let romanization = normalized_raw_payload(lyric.get("roma"));
    let raw_karaoke = if qrc.is_some() || translation.is_some() || romanization.is_some() {
        Some(RawKaraokeData {
            qrc_base64: qrc,
            translation_base64: translation.clone(),
            romanization_base64: romanization.clone(),
        })
    } else {
        None
    };
    Ok(raw_candidate(
        candidate,
        synced,
        plain,
        translation.as_deref().and_then(decode_base64_or_plain),
        romanization.as_deref().and_then(decode_base64_or_plain),
        None,
        raw_karaoke,
    ))
}

fn qq_song_lyric_payload(root: &Value) -> Result<&Value, ProviderError> {
    root.get("lyric")
        .map(|_| root)
        .or_else(|| root.pointer("/req_1/data"))
        .or_else(|| root.pointer("/req_1/data/song"))
        .ok_or_else(|| ProviderError::InvalidResponse("QQ lyric response has no data".into()))
}

fn nested_lyric(root: &Value, key: &str) -> Option<String> {
    root.get(key)
        .and_then(|value| value.get("lyric"))
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_owned)
}

fn raw_candidate(
    candidate: &ProviderCandidate,
    synced_lyrics: Option<String>,
    plain_lyrics: Option<String>,
    translation: Option<String>,
    romanization: Option<String>,
    raw_yrc: Option<String>,
    raw_karaoke: Option<RawKaraokeData>,
) -> RawLyricCandidate {
    RawLyricCandidate {
        candidate_id: candidate.candidate_id.clone(),
        provider: candidate.provider,
        provider_track_id: candidate.provider_track_id.clone(),
        title: candidate.title.clone(),
        artist: candidate.artist.clone(),
        album: candidate.album.clone(),
        duration_ms: candidate.duration_ms,
        source_url: candidate.source_url.clone(),
        synced_lyrics,
        plain_lyrics,
        translation,
        romanization,
        raw_yrc,
        raw_karaoke,
    }
}

fn value_string(value: Option<&Value>) -> Option<String> {
    value.and_then(|value| match value {
        Value::String(text) => Some(text.clone()),
        Value::Number(number) => Some(number.to_string()),
        _ => None,
    })
}

fn value_u64(value: Option<&Value>) -> Option<u64> {
    value.and_then(|value| value.as_u64().or_else(|| value.as_str()?.parse().ok()))
}

fn normalized_raw_payload(value: Option<&Value>) -> Option<String> {
    let value = value.and_then(Value::as_str)?.trim();
    if value.is_empty() {
        return None;
    }
    let decoded = general_purpose::STANDARD
        .decode(value)
        .or_else(|_| general_purpose::STANDARD_NO_PAD.decode(value))
        .or_else(|_| general_purpose::URL_SAFE.decode(value))
        .or_else(|_| general_purpose::URL_SAFE_NO_PAD.decode(value))
        .unwrap_or_else(|_| value.as_bytes().to_vec());
    Some(general_purpose::STANDARD.encode(decoded))
}

fn provider_text(value: Option<&Value>) -> Option<String> {
    let raw = value.and_then(Value::as_str)?.trim();
    if raw.is_empty() {
        return None;
    }
    decode_base64_or_plain(raw).or_else(|| Some(raw.to_owned()))
}

fn decode_base64_or_plain(raw: &str) -> Option<String> {
    general_purpose::STANDARD
        .decode(raw)
        .or_else(|_| general_purpose::STANDARD_NO_PAD.decode(raw))
        .or_else(|_| general_purpose::URL_SAFE.decode(raw))
        .or_else(|_| general_purpose::URL_SAFE_NO_PAD.decode(raw))
        .ok()
        .and_then(|decoded| String::from_utf8(decoded).ok())
        .or_else(|| Some(raw.to_owned()))
}

fn contains_lrc_timestamp(text: &str) -> bool {
    text.lines().any(|line| {
        let Some(after_open) = line.strip_prefix('[') else {
            return false;
        };
        let Some((time, _)) = after_open.split_once(']') else {
            return false;
        };
        let Some((minutes, seconds)) = time.split_once(':') else {
            return false;
        };
        !minutes.is_empty()
            && minutes.bytes().all(|byte| byte.is_ascii_digit())
            && seconds
                .bytes()
                .take_while(u8::is_ascii_digit)
                .next()
                .is_some()
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{
        collections::VecDeque,
        sync::{Arc, Mutex},
    };

    #[derive(Clone, Default)]
    struct MockTransport {
        responses: Arc<Mutex<VecDeque<Result<HttpResponse, ProviderError>>>>,
        requests: Arc<Mutex<Vec<HttpRequest>>>,
    }

    impl MockTransport {
        fn with_responses(responses: impl IntoIterator<Item = HttpResponse>) -> Self {
            Self::with_results(responses.into_iter().map(Ok))
        }

        fn with_results(
            responses: impl IntoIterator<Item = Result<HttpResponse, ProviderError>>,
        ) -> Self {
            Self {
                responses: Arc::new(Mutex::new(responses.into_iter().collect())),
                requests: Arc::new(Mutex::new(Vec::new())),
            }
        }
    }

    impl HttpTransport for MockTransport {
        fn send<'a>(
            &'a self,
            request: HttpRequest,
            cancellation: &'a CancellationToken,
        ) -> impl std::future::Future<Output = Result<HttpResponse, ProviderError>> + Send + 'a
        {
            async move {
                if cancellation.is_cancelled() {
                    return Err(ProviderError::Cancelled);
                }
                self.requests.lock().expect("requests lock").push(request);
                self.responses
                    .lock()
                    .expect("responses lock")
                    .pop_front()
                    .unwrap_or_else(|| Err(ProviderError::Transport("no mock response".into())))
            }
        }
    }

    struct PendingTransport;

    impl HttpTransport for PendingTransport {
        fn send<'a>(
            &'a self,
            request: HttpRequest,
            cancellation: &'a CancellationToken,
        ) -> impl std::future::Future<Output = Result<HttpResponse, ProviderError>> + Send + 'a
        {
            async move {
                let _ = (request, cancellation);
                std::future::pending().await
            }
        }
    }

    fn response(body: &str) -> HttpResponse {
        HttpResponse {
            status: 200,
            body: body.as_bytes().to_vec(),
        }
    }

    fn metadata() -> LyricsTrackMetadata {
        LyricsTrackMetadata {
            title: Some("Blue Song".into()),
            artist: Some("Moe".into()),
            album: Some("Album".into()),
            duration_ms: Some(185_000),
        }
    }

    fn candidate(provider: LyricProvider) -> ProviderCandidate {
        ProviderCandidate {
            candidate_id: "fixture:candidate".into(),
            provider,
            provider_track_id: if provider == LyricProvider::Qq {
                "song-mid"
            } else {
                "42"
            }
            .into(),
            title: Some("Blue Song".into()),
            artist: Some("Moe".into()),
            album: Some("Album".into()),
            duration_ms: Some(185_000),
            source_url: "https://example.invalid/song".into(),
            qq_song_id: Some(42),
        }
    }

    #[tokio::test]
    async fn netease_search_returns_raw_hits_in_provider_order_without_ranking() {
        let transport = MockTransport::with_responses([
            response(
                r#"{"code":200,"result":{"songs":[{"id":9,"name":"Second","artists":[{"name":"B"}],"album":{"name":"X"},"duration":92000},{"id":7,"name":"First","artists":[{"name":"A"}],"album":{"name":"Y"},"duration":81000}]}}"#,
            ),
            response(
                r#"{"code":200,"result":{"songs":[{"id":8,"name":"Title variant","artists":[{"name":"C"}],"album":{"name":"Z"},"duration":83000}]}}"#,
            ),
        ]);
        let client = LyricsProviderClient::new(transport);
        let hits = client
            .search(
                LyricProvider::NetEase,
                &metadata(),
                &CancellationToken::new(),
            )
            .await
            .unwrap();
        assert_eq!(
            hits.iter()
                .map(|hit| hit.candidate_id.as_str())
                .collect::<Vec<_>>(),
            ["netease:9", "netease:7", "netease:8"]
        );
        assert_eq!(hits[0].duration_ms, Some(92_000));
    }

    #[tokio::test]
    async fn netease_uses_cloudsearch_when_web_search_is_empty_and_caps_each_hit_page() {
        let transport = MockTransport::with_responses([
            response(r#"{"code":200,"result":{"songs":[]}}"#),
            response(
                r#"{"code":200,"result":{"songs":[{"id":5,"name":"Fallback","artists":[],"album":{},"duration":1000}]}}"#,
            ),
        ]);
        let mut local = metadata();
        local.artist = None;
        let client = LyricsProviderClient::new(transport.clone());
        let hits = client
            .search(LyricProvider::NetEase, &local, &CancellationToken::new())
            .await
            .unwrap();
        assert_eq!(hits.len(), 1);
        let requests = transport.requests.lock().unwrap();
        assert!(requests[0].url.starts_with(NETEASE_SEARCH_URL));
        assert!(requests[1].url.starts_with(NETEASE_CLOUD_SEARCH_URL));
        assert!(requests.iter().all(|request| request
            .headers
            .iter()
            .any(|(name, value)| name == "Referer" && value == "https://music.163.com/")));
        assert!(requests
            .iter()
            .all(|request| request.url.contains("limit=5")));
    }

    #[tokio::test]
    async fn netease_uses_cloudsearch_after_primary_transport_http_or_parse_failure() {
        let failures = [
            Err(ProviderError::Transport("fixture transport failure".into())),
            Ok(HttpResponse {
                status: 503,
                body: b"upstream unavailable".to_vec(),
            }),
            Ok(response("not json")),
        ];
        for failure in failures {
            let transport = MockTransport::with_results([
                failure,
                Ok(response(
                    r#"{"code":200,"result":{"songs":[{"id":6,"name":"Recovered","artists":[],"album":{},"duration":1000}]}}"#,
                )),
            ]);
            let mut local = metadata();
            local.artist = None;
            let client = LyricsProviderClient::new(transport.clone());
            let hits = client
                .search(LyricProvider::NetEase, &local, &CancellationToken::new())
                .await
                .unwrap();
            assert_eq!(hits[0].candidate_id, "netease:6");
            let requests = transport.requests.lock().unwrap();
            assert!(requests[0].url.starts_with(NETEASE_SEARCH_URL));
            assert!(requests[1].url.starts_with(NETEASE_CLOUD_SEARCH_URL));
        }
    }

    #[tokio::test]
    async fn netease_prefers_lrc_and_retains_yrc_translation_and_romanization() {
        let transport = MockTransport::with_responses([
            response(
                r#"{"code":200,"yrc":{"lyric":"[1200,1000](1200,1000,0)word"},"lrc":{"lyric":"[00:01.00]line"},"tlyric":{"lyric":"[00:01.00]譯文"},"romalrc":{"lyric":"[00:01.00]roma"}}"#,
            ),
            response(r#"{"code":200,"yrc":{"lyric":"[1200,1000](1200,1000,0)word"}}"#),
        ]);
        let client = LyricsProviderClient::new(transport.clone());
        let token = CancellationToken::new();
        let yrc = client
            .fetch_lyrics(&candidate(LyricProvider::NetEase), &token)
            .await
            .unwrap();
        assert_eq!(yrc.synced_lyrics.as_deref(), Some("[00:01.00]line"));
        assert_eq!(yrc.raw_yrc.as_deref(), Some("[1200,1000](1200,1000,0)word"));
        assert_eq!(yrc.translation.as_deref(), Some("[00:01.00]譯文"));
        assert_eq!(yrc.romanization.as_deref(), Some("[00:01.00]roma"));
        let lrc = client
            .fetch_lyrics(&candidate(LyricProvider::NetEase), &token)
            .await
            .unwrap();
        assert_eq!(
            lrc.synced_lyrics.as_deref(),
            Some("[1200,1000](1200,1000,0)word")
        );
        let requests = transport.requests.lock().unwrap();
        assert!(requests
            .iter()
            .all(|request| request.url.contains("lv=1&kv=1&tv=1&yv=1&rv=1")));
        assert!(requests.iter().all(|request| request
            .headers
            .iter()
            .any(|(name, value)| name == "Referer" && value == "https://music.163.com/")));
    }

    #[tokio::test]
    async fn qq_search_reads_metadata_and_keeps_song_mid_stable_for_fetch() {
        let transport = MockTransport::with_responses([response(
            r#"{"req_1":{"data":{"body":{"song":{"list":[{"songmid":"mid-a","songid":45,"songname":"Blue Song","singer":[{"name":"Moe"}],"albumname":"Album","interval":185}]}}}}}"#,
        )]);
        let client = LyricsProviderClient::new(transport.clone());
        let hits = client
            .search(LyricProvider::Qq, &metadata(), &CancellationToken::new())
            .await
            .unwrap();
        assert_eq!(hits[0].candidate_id, "qqmusic:mid-a");
        assert_eq!(hits[0].duration_ms, Some(185_000));
        assert_eq!(hits[0].qq_song_id, Some(45));
        let request = &transport.requests.lock().unwrap()[0];
        assert!(request
            .body
            .as_deref()
            .unwrap()
            .contains("music.search.SearchCgiService"));
        assert!(request
            .headers
            .iter()
            .any(|(name, value)| name == "Referer" && value == QQ_REFERER));
    }

    #[tokio::test]
    async fn qq_decodes_plain_lrc_but_preserves_raw_qrc_translation_and_roma_base64() {
        let trans = general_purpose::STANDARD_NO_PAD.encode("translation");
        let roma = general_purpose::URL_SAFE_NO_PAD.encode("romanization");
        let body = json!({
            "lyric":"[00:01.00]hello",
            "qrc":general_purpose::STANDARD_NO_PAD.encode(b"opaque-qrc"),
            "trans":trans,
            "roma":roma
        })
        .to_string();
        let plain_body = json!({"lyric":"untimed text"}).to_string();
        let raw_aux_body =
            json!({"lyric":"", "qrc":"qrc text", "trans":"translated text", "roma":"romanized"})
                .to_string();
        let transport = MockTransport::with_responses([
            response(&body),
            response(&plain_body),
            response(&raw_aux_body),
        ]);
        let client = LyricsProviderClient::new(transport.clone());
        let token = CancellationToken::new();
        let raw = client
            .fetch_lyrics(&candidate(LyricProvider::Qq), &token)
            .await
            .unwrap();
        assert_eq!(raw.synced_lyrics.as_deref(), Some("[00:01.00]hello"));
        assert_eq!(
            raw.raw_karaoke.as_ref().unwrap().qrc_base64.as_deref(),
            Some(general_purpose::STANDARD.encode(b"opaque-qrc").as_str())
        );
        assert_eq!(raw.translation.as_deref(), Some("translation"));
        assert_eq!(raw.romanization.as_deref(), Some("romanization"));
        let raw = client
            .fetch_lyrics(&candidate(LyricProvider::Qq), &token)
            .await
            .unwrap();
        assert_eq!(raw.plain_lyrics.as_deref(), Some("untimed text"));
        assert!(raw.synced_lyrics.is_none());
        let raw_aux = client
            .fetch_lyrics(&candidate(LyricProvider::Qq), &token)
            .await
            .unwrap();
        assert_eq!(
            raw_aux.raw_karaoke.as_ref().unwrap().qrc_base64.as_deref(),
            Some(general_purpose::STANDARD.encode("qrc text").as_str())
        );
        assert_eq!(raw_aux.translation.as_deref(), Some("translated text"));
        assert_eq!(raw_aux.romanization.as_deref(), Some("romanized"));
        let requests = transport.requests.lock().unwrap();
        let request = &requests[0];
        assert_eq!(request.method, HttpMethod::Get);
        assert!(request.url.starts_with(QQ_LYRIC_URL));
        assert!(request.url.contains("nobase64=1"));
        assert!(request
            .headers
            .iter()
            .any(|(name, value)| name == "Origin" && value == "https://y.qq.com"));
        assert!(request
            .headers
            .iter()
            .any(|(name, value)| name == "Accept" && value == "application/json, text/plain, */*"));
        assert!(request.headers.iter().any(
            |(name, value)| name == "Content-Type" && value == "application/json;charset=UTF-8"
        ));
    }

    #[tokio::test]
    async fn cancellation_invalid_json_http_errors_and_size_cap_are_fail_soft_errors() {
        let canceled = CancellationToken::new();
        canceled.cancel();
        let client = LyricsProviderClient::new(MockTransport::with_responses([]));
        assert_eq!(
            client
                .search(LyricProvider::NetEase, &metadata(), &canceled)
                .await,
            Err(ProviderError::Cancelled)
        );
        assert!(matches!(
            parse_json(b"{"),
            Err(ProviderError::InvalidResponse(_))
        ));
        let oversized = HttpResponse {
            status: 200,
            body: vec![0; MAX_RESPONSE_BYTES + 1],
        };
        let client = LyricsProviderClient::new(MockTransport::with_responses([oversized]));
        assert_eq!(
            client
                .search(
                    LyricProvider::NetEase,
                    &metadata(),
                    &CancellationToken::new()
                )
                .await,
            Err(ProviderError::ResponseTooLarge)
        );
        let client = LyricsProviderClient::new(MockTransport::with_responses([HttpResponse {
            status: 503,
            body: vec![],
        }]));
        assert_eq!(
            client
                .search(LyricProvider::Qq, &metadata(), &CancellationToken::new())
                .await,
            Err(ProviderError::HttpStatus(503))
        );
    }

    #[tokio::test]
    async fn transport_wait_is_bounded_by_five_second_timeout() {
        let client = LyricsProviderClient::new(PendingTransport);
        assert_eq!(
            client
                .search(LyricProvider::Qq, &metadata(), &CancellationToken::new())
                .await,
            Err(ProviderError::Timeout)
        );
    }

    #[tokio::test]
    async fn local_lyric_providers_are_not_sent_to_remote_transport() {
        let client = LyricsProviderClient::new(MockTransport::with_responses([]));
        assert_eq!(
            client
                .search(
                    LyricProvider::Sidecar,
                    &metadata(),
                    &CancellationToken::new()
                )
                .await,
            Err(ProviderError::UnsupportedProvider)
        );
    }
}
