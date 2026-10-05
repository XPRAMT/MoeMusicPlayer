//! Track-scoped lyric lookup and IPC DTO conversion.

use std::{
    collections::{HashMap, HashSet},
    sync::{
        atomic::{AtomicU64, Ordering},
        Arc, Mutex,
    },
    time::Duration,
};

use player_core::{
    auto_lyric_candidate, explain_lyric_candidate_match, merge_lrc_auxiliary, parse_lrc, parse_yrc,
    preserve_qrc, rank_lyric_candidates, with_raw_karaoke, LyricAuxiliaryKind, LyricCandidate,
    LyricFormat, LyricLine, LyricProvider, LyricsTrackMetadata, ParsedLyrics, TrackId, TrackLyrics,
    TrackSummary,
};
use player_db::Database;
use serde::Serialize;
use tauri::{AppHandle, State};
use tokio::{sync::Semaphore, task::JoinSet};
use tokio_util::sync::CancellationToken;

use crate::lyrics_provider::{
    LyricsProviderClient, ProviderCandidate, ProviderError, RawLyricCandidate, ReqwestHttpTransport,
};

const MAX_PROVIDER_HITS: usize = 20;
const MAX_SEARCH_CANDIDATES: usize = 20;
const MAX_FETCH_CANDIDATES: usize = 8;
const MAX_CONCURRENT_LYRIC_FETCHES: usize = 3;
const HIGH_CONFIDENCE_SCORE: f32 = 0.90;
const MAX_REQUEST_ID_BYTES: usize = 128;
const MAX_PROVIDER_SEARCH_DURATION: Duration = Duration::from_secs(20);

#[cfg(target_os = "windows")]
use player_platform_windows::{find_lyrics, LyricsLookup};

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
pub enum LyricsSourceDto {
    #[serde(rename = "local")]
    Local,
    #[serde(rename = "embedded")]
    Embedded,
    #[serde(rename = "netease")]
    NetEase,
    #[serde(rename = "qqmusic")]
    QqMusic,
    #[serde(rename = "manual")]
    Manual,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LyricLineDto {
    pub start_ms: Option<u64>,
    pub text: String,
    pub translation: Option<String>,
    pub romanization: Option<String>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TrackLyricsDto {
    pub track_id: String,
    pub source: LyricsSourceDto,
    pub title: String,
    pub artist: Option<String>,
    pub album: Option<String>,
    pub duration_ms: Option<u64>,
    /// LRC offsets are already applied by the parser, so the renderer must not apply them again.
    pub offset_ms: i64,
    pub synced: bool,
    pub lines: Vec<LyricLineDto>,
}

#[derive(Clone, Copy, Debug, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum LyricsResultStatusDto {
    Ready,
    Empty,
    Candidates,
    Error,
}

#[derive(Clone, Copy, Debug, Serialize)]
pub enum LyricsProviderDto {
    #[serde(rename = "netease")]
    NetEase,
    #[serde(rename = "qqmusic")]
    QqMusic,
}

#[derive(Clone, Copy, Debug, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum LyricsConfidenceDto {
    High,
    Medium,
    Low,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LyricsCandidateDto {
    pub id: String,
    pub provider: LyricsProviderDto,
    pub title: String,
    pub artist: String,
    pub album: Option<String>,
    pub duration_ms: Option<u64>,
    pub score: f32,
    pub confidence: LyricsConfidenceDto,
    pub reasons: Vec<String>,
    pub preview_lines: Vec<String>,
    pub has_synced_lyrics: bool,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LyricsTrackResultDto {
    pub lyrics: Option<TrackLyricsDto>,
    pub candidates: Vec<LyricsCandidateDto>,
    pub status: LyricsResultStatusDto,
    pub error: Option<String>,
}

struct ActiveLyricsSearch {
    generation: u64,
    cancellation: CancellationToken,
}

pub struct LyricsService<T: crate::lyrics_provider::HttpTransport = ReqwestHttpTransport> {
    client: Arc<LyricsProviderClient<T>>,
    active_searches: Arc<Mutex<HashMap<String, ActiveLyricsSearch>>>,
    next_generation: AtomicU64,
}

impl Default for LyricsService<ReqwestHttpTransport> {
    fn default() -> Self {
        Self::new(LyricsProviderClient::default())
    }
}

impl<T: crate::lyrics_provider::HttpTransport + 'static> LyricsService<T> {
    fn new(client: LyricsProviderClient<T>) -> Self {
        Self {
            client: Arc::new(client),
            active_searches: Arc::new(Mutex::new(HashMap::new())),
            next_generation: AtomicU64::new(1),
        }
    }

    fn begin_search(
        &self,
        request_id: &str,
    ) -> Result<(CancellationToken, LyricsSearchGuard), String> {
        validate_request_id(request_id)?;
        let generation = self.next_generation.fetch_add(1, Ordering::Relaxed);
        let cancellation = CancellationToken::new();
        let mut active = self
            .active_searches
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        // The UI owns one current-track lyrics view. A newer request replaces any abandoned
        // request, keeping both queued searches and provider work bounded to one track.
        for search in active.values() {
            search.cancellation.cancel();
        }
        active.clear();
        active.insert(
            request_id.to_owned(),
            ActiveLyricsSearch {
                generation,
                cancellation: cancellation.clone(),
            },
        );
        let guard = LyricsSearchGuard {
            request_id: request_id.to_owned(),
            generation,
            active_searches: self.active_searches.clone(),
            cancellation: cancellation.clone(),
        };
        Ok((cancellation, guard))
    }

    fn cancel_search(&self, request_id: &str) {
        if request_id.len() > MAX_REQUEST_ID_BYTES {
            return;
        }
        let mut active = self
            .active_searches
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if let Some(search) = active.remove(request_id) {
            search.cancellation.cancel();
        }
    }

    async fn search_track(
        &self,
        database: &Database,
        track_id: TrackId,
        track: &TrackSummary,
        cancellation: CancellationToken,
        allow_auto_apply: bool,
        query_override: Option<String>,
    ) -> Result<LyricsTrackResultDto, String> {
        let metadata = LyricsTrackMetadata {
            title: track.title.clone(),
            artist: track.artist.clone(),
            album: track.album.clone(),
            duration_ms: track.duration_ms,
        };
        let search_metadata = match query_override
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
        {
            Some(query) => LyricsTrackMetadata {
                title: Some(query.to_owned()),
                artist: None,
                album: None,
                duration_ms: metadata.duration_ms,
            },
            None => metadata.clone(),
        };
        let candidates = match tokio::time::timeout(
            MAX_PROVIDER_SEARCH_DURATION,
            fetch_provider_candidates(self.client.clone(), search_metadata, cancellation.clone()),
        )
        .await
        {
            Err(_) => {
                cancellation.cancel();
                return Ok(LyricsTrackResultDto::empty(Some(provider_error_message(
                    &ProviderError::Timeout,
                ))));
            }
            Ok(Ok(candidates)) => candidates,
            Ok(Err(ProviderError::Cancelled)) => return Ok(LyricsTrackResultDto::empty(None)),
            Ok(Err(error)) => {
                return Ok(LyricsTrackResultDto::empty(Some(provider_error_message(
                    &error,
                ))))
            }
        };
        if cancellation.is_cancelled() {
            return Ok(LyricsTrackResultDto::empty(None));
        }
        if candidates.is_empty() {
            return Ok(LyricsTrackResultDto::empty(None));
        }

        let ranked = rank_lyric_candidates(&metadata, candidates);
        let now = super::now_utc_epoch_ms()?;
        database
            .cache_lyric_candidates(track_id, &ranked, now)
            .map_err(|error| error.to_string())?;

        if allow_auto_apply {
            if let Some(best) = auto_lyric_candidate(&ranked)
                .filter(|candidate| candidate.score >= HIGH_CONFIDENCE_SCORE)
            {
                let automatic = TrackLyrics {
                    track_id,
                    provider: best.provider,
                    candidate_id: Some(best.candidate_id.clone()),
                    lyrics: best.lyrics.clone(),
                    manually_selected: false,
                    updated_at_utc_ms: now,
                };
                if database
                    .save_automatic_track_lyrics(&automatic)
                    .map_err(|error| error.to_string())?
                {
                    return Ok(LyricsTrackResultDto::ready(track_lyrics_dto(
                        track_id,
                        track,
                        automatic.provider,
                        false,
                        &automatic.lyrics,
                    )));
                }
                // A manual selection may have landed while the provider request was in flight.
                if let Some(selected) = database
                    .get_track_lyrics(track_id)
                    .map_err(|error| error.to_string())?
                {
                    return Ok(LyricsTrackResultDto::ready(persisted_lyrics_dto(
                        track, selected,
                    )));
                }
            }
        }

        let candidates = ranked
            .iter()
            .take(MAX_SEARCH_CANDIDATES)
            .map(|candidate| candidate_dto(&metadata, candidate))
            .collect::<Vec<_>>();
        if candidates.is_empty() {
            Ok(LyricsTrackResultDto::empty(None))
        } else {
            Ok(LyricsTrackResultDto {
                lyrics: None,
                candidates,
                status: LyricsResultStatusDto::Candidates,
                error: None,
            })
        }
    }
}

async fn fetch_provider_candidates<T: crate::lyrics_provider::HttpTransport + 'static>(
    client: Arc<LyricsProviderClient<T>>,
    metadata: LyricsTrackMetadata,
    cancellation: CancellationToken,
) -> Result<Vec<LyricCandidate>, ProviderError> {
    let (netease, qqmusic) = tokio::join!(
        client.search(LyricProvider::NetEase, &metadata, &cancellation),
        client.search(LyricProvider::Qq, &metadata, &cancellation),
    );
    if cancellation.is_cancelled() {
        return Err(ProviderError::Cancelled);
    }
    let failed_searches = usize::from(netease.is_err()) + usize::from(qqmusic.is_err());
    if failed_searches == 2 {
        return Err(netease.err().unwrap_or(ProviderError::InvalidResponse(
            "provider search failed".to_owned(),
        )));
    }

    let mut hits = Vec::new();
    if let Ok(mut found) = netease {
        found.truncate(MAX_PROVIDER_HITS);
        hits.extend(found);
    }
    if let Ok(mut found) = qqmusic {
        found.truncate(MAX_PROVIDER_HITS);
        hits.extend(found);
    }

    let mut seen = HashSet::new();
    hits.retain(|hit| {
        !hit.candidate_id.is_empty()
            && hit.candidate_id.len() <= 256
            && hit.provider_track_id.len() <= 128
            && seen.insert(hit.candidate_id.clone())
    });
    if hits.is_empty() {
        return Ok(Vec::new());
    }

    let hit_by_id = hits
        .iter()
        .cloned()
        .map(|hit| (hit.candidate_id.clone(), hit))
        .collect::<HashMap<_, _>>();
    let preliminary =
        rank_lyric_candidates(&metadata, hits.iter().map(provider_hit_to_lyric_candidate))
            .into_iter()
            .take(MAX_SEARCH_CANDIDATES)
            .collect::<Vec<_>>();

    let permits = Arc::new(Semaphore::new(MAX_CONCURRENT_LYRIC_FETCHES));
    let mut tasks = JoinSet::new();
    for ranked_hit in preliminary.into_iter().take(MAX_FETCH_CANDIDATES) {
        let Some(hit) = hit_by_id.get(&ranked_hit.candidate_id).cloned() else {
            continue;
        };
        let client = client.clone();
        let cancellation = cancellation.clone();
        let permits = permits.clone();
        tasks.spawn(async move {
            let _permit = tokio::select! {
                _ = cancellation.cancelled() => return Err(ProviderError::Cancelled),
                permit = permits.acquire_owned() => permit.map_err(|_| ProviderError::Cancelled)?,
            };
            let raw = client.fetch_lyrics(&hit, &cancellation).await?;
            raw_to_lyric_candidate(raw)
        });
    }

    let mut candidates = Vec::new();
    let mut first_fetch_error = None;
    while let Some(result) = tasks.join_next().await {
        match result {
            Ok(Ok(candidate)) => {
                if !candidate.lyrics.lines.is_empty()
                    || candidate.lyrics.raw_karaoke.is_some()
                    || !candidate.lyrics.raw_text.is_empty()
                {
                    candidates.push(candidate);
                }
            }
            Ok(Err(ProviderError::Cancelled)) if cancellation.is_cancelled() => {
                return Err(ProviderError::Cancelled)
            }
            Ok(Err(error)) => {
                first_fetch_error.get_or_insert(error);
            }
            Err(error) => {
                first_fetch_error.get_or_insert(ProviderError::Transport(error.to_string()));
            }
        };
    }
    if cancellation.is_cancelled() {
        return Err(ProviderError::Cancelled);
    }
    if candidates.is_empty() {
        if let Some(error) = first_fetch_error {
            return Err(error);
        }
    }
    Ok(candidates)
}

fn provider_hit_to_lyric_candidate(hit: &ProviderCandidate) -> LyricCandidate {
    LyricCandidate {
        candidate_id: hit.candidate_id.clone(),
        provider: hit.provider,
        provider_track_id: Some(hit.provider_track_id.clone()),
        title: hit.title.clone(),
        artist: hit.artist.clone(),
        album: hit.album.clone(),
        duration_ms: hit.duration_ms,
        lyrics: empty_parsed_lyrics(),
        score: 0.0,
    }
}

fn raw_to_lyric_candidate(raw: RawLyricCandidate) -> Result<LyricCandidate, ProviderError> {
    let mut lyrics = if let Some(text) = raw.synced_lyrics.as_deref() {
        if looks_like_yrc(text) {
            parse_yrc(text)
        } else {
            parse_lrc(text)
        }
    } else if let Some(text) = raw.plain_lyrics.as_deref() {
        parse_lrc(text)
    } else if let Some(qrc) = raw
        .raw_karaoke
        .as_ref()
        .and_then(|payload| payload.qrc_base64.as_deref())
    {
        preserve_qrc(qrc)
    } else {
        Ok(empty_parsed_lyrics())
    }
    .map_err(|error| ProviderError::InvalidResponse(error.to_string()))?;

    if lyrics.lines.is_empty() {
        if let Some(plain) = raw.plain_lyrics.as_deref() {
            lyrics = parse_lrc(plain)
                .map_err(|error| ProviderError::InvalidResponse(error.to_string()))?;
        }
    }
    if lyrics.synced {
        if let Some(translation) = raw.translation.as_deref() {
            merge_lrc_auxiliary(&mut lyrics, translation, LyricAuxiliaryKind::Translation)
                .map_err(|error| ProviderError::InvalidResponse(error.to_string()))?;
        }
        if let Some(romanization) = raw.romanization.as_deref() {
            merge_lrc_auxiliary(&mut lyrics, romanization, LyricAuxiliaryKind::Romanization)
                .map_err(|error| ProviderError::InvalidResponse(error.to_string()))?;
        }
    }

    let raw_karaoke_json = preserve_raw_karaoke(&raw)?;
    if raw_karaoke_json.is_some() {
        lyrics = with_raw_karaoke(lyrics, raw_karaoke_json);
    }
    Ok(LyricCandidate {
        candidate_id: raw.candidate_id,
        provider: raw.provider,
        provider_track_id: Some(raw.provider_track_id),
        title: raw.title,
        artist: raw.artist,
        album: raw.album,
        duration_ms: raw.duration_ms,
        lyrics,
        score: 0.0,
    })
}

#[derive(Serialize)]
struct PreservedKaraoke<'a> {
    yrc: Option<&'a str>,
    qrc_base64: Option<&'a str>,
    translation_base64: Option<&'a str>,
    romanization_base64: Option<&'a str>,
}

fn preserve_raw_karaoke(raw: &RawLyricCandidate) -> Result<Option<String>, ProviderError> {
    let preserved = PreservedKaraoke {
        yrc: raw.raw_yrc.as_deref(),
        qrc_base64: raw
            .raw_karaoke
            .as_ref()
            .and_then(|payload| payload.qrc_base64.as_deref()),
        translation_base64: raw
            .raw_karaoke
            .as_ref()
            .and_then(|payload| payload.translation_base64.as_deref()),
        romanization_base64: raw
            .raw_karaoke
            .as_ref()
            .and_then(|payload| payload.romanization_base64.as_deref()),
    };
    if preserved.yrc.is_none()
        && preserved.qrc_base64.is_none()
        && preserved.translation_base64.is_none()
        && preserved.romanization_base64.is_none()
    {
        return Ok(None);
    }
    serde_json::to_string(&preserved)
        .map(Some)
        .map_err(|error| ProviderError::InvalidResponse(error.to_string()))
}

fn empty_parsed_lyrics() -> ParsedLyrics {
    ParsedLyrics {
        format: LyricFormat::Plain,
        lines: Vec::new(),
        raw_text: String::new(),
        raw_karaoke: None,
        synced: false,
    }
}

fn looks_like_yrc(text: &str) -> bool {
    text.lines().any(|line| {
        let Some((timing, _)) = line
            .trim()
            .strip_prefix('[')
            .and_then(|line| line.split_once(']'))
        else {
            return false;
        };
        let mut parts = timing.split(',');
        matches!((parts.next(), parts.next(), parts.next()), (Some(start), Some(duration), None)
            if start.parse::<u64>().is_ok() && duration.parse::<u64>().is_ok())
    })
}

fn provider_error_message(_error: &ProviderError) -> String {
    "線上歌詞來源目前無法使用，請稍後再試。".to_owned()
}

fn candidate_dto(metadata: &LyricsTrackMetadata, candidate: &LyricCandidate) -> LyricsCandidateDto {
    let reasons = explain_lyric_candidate_match(metadata, candidate);
    let confidence = if candidate.score >= HIGH_CONFIDENCE_SCORE {
        LyricsConfidenceDto::High
    } else if candidate.score >= 0.72 {
        LyricsConfidenceDto::Medium
    } else {
        LyricsConfidenceDto::Low
    };
    let mut reasons = reasons;
    if candidate.lyrics.format == LyricFormat::Qrc && candidate.lyrics.lines.is_empty() {
        reasons.push("僅保留未解碼的 QRC 原始資料，沒有可顯示的歌詞文字".to_owned());
    } else if !candidate.lyrics.synced {
        reasons.push("此候選沒有逐行時間戳，僅作未同步歌詞顯示".to_owned());
    }
    LyricsCandidateDto {
        id: candidate.candidate_id.clone(),
        provider: match candidate.provider {
            LyricProvider::NetEase => LyricsProviderDto::NetEase,
            LyricProvider::Qq => LyricsProviderDto::QqMusic,
            LyricProvider::Sidecar | LyricProvider::Embedded => LyricsProviderDto::NetEase,
        },
        title: candidate.title.clone().unwrap_or_default(),
        artist: candidate.artist.clone().unwrap_or_default(),
        album: candidate.album.clone(),
        duration_ms: candidate.duration_ms,
        score: candidate.score,
        confidence,
        reasons,
        preview_lines: candidate
            .lyrics
            .lines
            .iter()
            .filter(|line| !line.text.trim().is_empty())
            .take(3)
            .map(|line| line.text.clone())
            .collect(),
        has_synced_lyrics: candidate.lyrics.synced,
    }
}

struct LyricsSearchGuard {
    request_id: String,
    generation: u64,
    active_searches: Arc<Mutex<HashMap<String, ActiveLyricsSearch>>>,
    cancellation: CancellationToken,
}

impl Drop for LyricsSearchGuard {
    fn drop(&mut self) {
        self.cancellation.cancel();
        let mut active = self
            .active_searches
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if active
            .get(&self.request_id)
            .is_some_and(|search| search.generation == self.generation)
        {
            active.remove(&self.request_id);
        }
    }
}

fn validate_request_id(request_id: &str) -> Result<(), String> {
    if request_id.is_empty()
        || request_id.len() > MAX_REQUEST_ID_BYTES
        || request_id.chars().any(char::is_control)
    {
        return Err("歌詞搜尋識別碼無效。".to_owned());
    }
    Ok(())
}

impl LyricsTrackResultDto {
    fn ready(lyrics: TrackLyricsDto) -> Self {
        Self {
            lyrics: Some(lyrics),
            candidates: Vec::new(),
            status: LyricsResultStatusDto::Ready,
            error: None,
        }
    }

    fn empty(error: Option<String>) -> Self {
        Self {
            lyrics: None,
            candidates: Vec::new(),
            status: if error.is_some() {
                LyricsResultStatusDto::Error
            } else {
                LyricsResultStatusDto::Empty
            },
            error,
        }
    }
}

#[tauri::command]
pub async fn lyrics_get_track(
    state: State<'_, super::AppState>,
    app: AppHandle,
    track_id: String,
) -> Result<LyricsTrackResultDto, String> {
    let track_id = TrackId::parse(&track_id).map_err(|_| "曲目識別碼無效。".to_owned())?;
    let database = state.database.as_ref().ok_or_else(|| {
        state
            .database_error
            .clone()
            .unwrap_or_else(|| "曲庫資料庫尚未開啟。".to_owned())
    })?;
    let settings = state
        .settings
        .snapshot()
        .map_err(|error| error.to_string())?;
    load_track_result_with_context(database, track_id, Some(&app), &settings.sources).await
}

#[tauri::command]
pub async fn lyrics_search(
    state: State<'_, super::AppState>,
    app: AppHandle,
    track_id: String,
    request_id: String,
    manual: Option<bool>,
    query: Option<String>,
) -> Result<LyricsTrackResultDto, String> {
    let _database_work = super::enter_database_work(&state)?;
    let track_id = TrackId::parse(&track_id).map_err(|_| "曲目識別碼無效。".to_owned())?;
    validate_request_id(&request_id)?;
    let manual = manual.unwrap_or(false);
    let database = state.database.as_ref().ok_or_else(|| {
        state
            .database_error
            .clone()
            .unwrap_or_else(|| "曲庫資料庫尚未開啟。".to_owned())
    })?;

    let (cancellation, _guard) = state.lyrics_service.begin_search(&request_id)?;
    let settings = state
        .settings
        .snapshot()
        .map_err(|error| error.to_string())?;
    let initial =
        load_track_result_with_context(database, track_id, Some(&app), &settings.sources).await?;
    // Automatic follow-up after an empty get may rediscover local/embedded lyrics.
    // Explicit manual selection must continue to provider candidates even when lyrics exist.
    if !manual && initial.lyrics.is_some() {
        return Ok(initial);
    }
    if cancellation.is_cancelled() {
        return Ok(LyricsTrackResultDto::empty(None));
    }
    let track = database
        .get_track_summary(track_id)
        .map_err(|error| error.to_string())?
        .ok_or_else(|| "找不到這首曲目。".to_owned())?;
    let query = query
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_owned);
    let mut result = state
        .lyrics_service
        .search_track(database, track_id, &track, cancellation, !manual, query)
        .await?;
    if manual && result.lyrics.is_none() {
        // Keep already-playing lyrics so dismiss can restore playback UI without reload.
        result.lyrics = initial.lyrics;
    }
    Ok(result)
}

#[tauri::command]
pub fn lyrics_cancel_search(state: State<'_, super::AppState>, request_id: String) {
    state.lyrics_service.cancel_search(&request_id);
}

#[tauri::command]
pub fn lyrics_select_candidate(
    state: State<'_, super::AppState>,
    track_id: String,
    candidate_id: String,
) -> Result<TrackLyricsDto, String> {
    let _database_work = super::enter_database_work(&state)?;
    let track_id = TrackId::parse(&track_id).map_err(|_| "曲目識別碼無效。".to_owned())?;
    if candidate_id.is_empty() || candidate_id.len() > 256 {
        return Err("歌詞候選識別碼無效，請重新搜尋。".to_owned());
    }
    let database = state.database.as_ref().ok_or_else(|| {
        state
            .database_error
            .clone()
            .unwrap_or_else(|| "曲庫資料庫尚未開啟。".to_owned())
    })?;
    select_candidate_from_database(
        database,
        track_id,
        &candidate_id,
        super::now_utc_epoch_ms()?,
    )
}

fn select_candidate_from_database(
    database: &Database,
    track_id: TrackId,
    candidate_id: &str,
    selected_at_utc_ms: i64,
) -> Result<TrackLyricsDto, String> {
    let track = database
        .get_track_summary(track_id)
        .map_err(|error| error.to_string())?
        .ok_or_else(|| "找不到這首曲目。".to_owned())?;
    let selected = database
        .select_lyric_candidate(track_id, candidate_id, selected_at_utc_ms)
        .map_err(|error| error.to_string())?;
    Ok(persisted_lyrics_dto(&track, selected))
}

#[cfg(test)]
async fn load_track_result(
    database: &Database,
    track_id: TrackId,
) -> Result<LyricsTrackResultDto, String> {
    load_track_result_with_context(database, track_id, None, &[]).await
}

async fn load_track_result_with_context(
    database: &Database,
    track_id: TrackId,
    app: Option<&AppHandle>,
    sources: &[super::settings::SourceEntry],
) -> Result<LyricsTrackResultDto, String> {
    let track = database
        .get_track_summary(track_id)
        .map_err(|error| error.to_string())?
        .ok_or_else(|| "找不到這首曲目。".to_owned())?;
    let cached = database
        .get_track_lyrics(track_id)
        .map_err(|error| error.to_string())?;

    match local_lyrics_for_track(app, sources, database, track_id, &track).await? {
        LocalLyricsLookup::Found(lyrics) => Ok(LyricsTrackResultDto::ready(lyrics)),
        LocalLyricsLookup::Oversized => match cached {
            Some(cached) => Ok(LyricsTrackResultDto::ready(persisted_lyrics_dto(
                &track, cached,
            ))),
            None => Ok(LyricsTrackResultDto::empty(Some(
                "本機歌詞檔超過 2 MiB 限制。".to_owned(),
            ))),
        },
        LocalLyricsLookup::Missing => match cached {
            Some(cached) => Ok(LyricsTrackResultDto::ready(persisted_lyrics_dto(
                &track, cached,
            ))),
            None => Ok(LyricsTrackResultDto::empty(None)),
        },
    }
}

enum LocalLyricsLookup {
    Found(TrackLyricsDto),
    Missing,
    Oversized,
}

#[cfg(target_os = "windows")]
async fn local_lyrics_for_track(
    _app: Option<&AppHandle>,
    _sources: &[super::settings::SourceEntry],
    database: &Database,
    track_id: TrackId,
    track: &TrackSummary,
) -> Result<LocalLyricsLookup, String> {
    let locators = database
        .track_locators(track_id)
        .map_err(|error| error.to_string())?;
    if locators.is_empty() {
        return Ok(LocalLyricsLookup::Missing);
    }
    let lookup = tauri::async_runtime::spawn_blocking(move || find_lyrics(&locators))
        .await
        .map_err(|error| format!("讀取本機歌詞工作失敗：{error}"))?;
    match lookup {
        LyricsLookup::Found(found) => Ok(LocalLyricsLookup::Found(track_lyrics_dto(
            track.id,
            track,
            found.provider,
            false,
            &found.lyrics,
        ))),
        LyricsLookup::Missing => Ok(LocalLyricsLookup::Missing),
        LyricsLookup::Oversized => Ok(LocalLyricsLookup::Oversized),
    }
}

#[cfg(not(any(target_os = "windows", target_os = "android")))]
async fn local_lyrics_for_track(
    _app: Option<&AppHandle>,
    _sources: &[super::settings::SourceEntry],
    _database: &Database,
    _track_id: TrackId,
    _track: &TrackSummary,
) -> Result<LocalLyricsLookup, String> {
    Ok(LocalLyricsLookup::Missing)
}

#[cfg(target_os = "android")]
async fn local_lyrics_for_track(
    app: Option<&AppHandle>,
    sources: &[super::settings::SourceEntry],
    database: &Database,
    track_id: TrackId,
    track: &TrackSummary,
) -> Result<LocalLyricsLookup, String> {
    use lofty::{config::ParseOptions, prelude::TaggedFileExt, probe::Probe, tag::ItemKey};
    use player_core::{parse_lrc, parse_yrc, ParsedLyrics, MAX_LYRIC_PAYLOAD_BYTES};
    use tauri_plugin_media_index::MediaIndexExt;

    let Some(app) = app else {
        return Ok(LocalLyricsLookup::Missing);
    };
    let locators = database
        .track_locators(track_id)
        .map_err(|error| error.to_string())?;
    let Some(audio_uri) = locators.iter().find_map(|locator| match locator {
        player_core::MediaLocator::ContentUri(uri) => Some(uri.clone()),
        player_core::MediaLocator::FileSystem(_) => None,
    }) else {
        return Ok(LocalLyricsLookup::Missing);
    };

    let tree_uris = sources
        .iter()
        .filter(|source| source.enabled)
        .filter_map(|source| match &source.kind {
            super::settings::SourceEntryKind::Folder {
                media_kind: player_core::MediaSourceKind::AndroidSaf,
                path: super::settings::StoredPath::Uri(uri),
            } => Some(uri.clone()),
            super::settings::SourceEntryKind::PlaylistFile {
                tree_uri: Some(uri),
                ..
            } => Some(uri.clone()),
            _ => None,
        })
        .collect::<Vec<_>>();
    for tree_uri in tree_uris {
        let app = app.clone();
        let audio_uri = audio_uri.clone();
        let tree_uri = tree_uri.clone();
        let sidecar = tauri::async_runtime::spawn_blocking(move || {
            app.media_index()
                .read_saf_lyric_sibling(&tree_uri, &audio_uri, MAX_LYRIC_PAYLOAD_BYTES as u64)
                .map_err(|error| error.to_string())
        })
        .await
        .map_err(|error| format!("讀取 Android 歌詞 sidecar 工作失敗：{error}"))??;
        if let Some(text) = sidecar {
            if text.len() > MAX_LYRIC_PAYLOAD_BYTES {
                return Ok(LocalLyricsLookup::Oversized);
            }
            if let Ok(lyrics) = parse_lrc(&text) {
                if !lyrics.lines.is_empty() {
                    return Ok(LocalLyricsLookup::Found(track_lyrics_dto(
                        track.id,
                        track,
                        LyricProvider::Sidecar,
                        false,
                        &lyrics,
                    )));
                }
            }
        }
    }

    let app = app.clone();
    let lookup = tauri::async_runtime::spawn_blocking(
        move || -> Result<(Option<ParsedLyrics>, bool), String> {
            let lease = app
                .media_index()
                .cache_content_uri(&audio_uri, 512 * 1024 * 1024)
                .map_err(|error| error.to_string())?;
            let tagged = Probe::open(lease.path())
                .map_err(|error| format!("無法開啟 Android 曲目標籤：{error}"))?
                .options(ParseOptions::new().read_properties(false))
                .guess_file_type()
                .map_err(|error| format!("無法辨識 Android 曲目格式：{error}"))?
                .read()
                .map_err(|error| format!("無法讀取 Android 曲目標籤：{error}"))?;
            let tags = tagged.tags();
            let mut oversized = false;
            for key in [ItemKey::Lyrics, ItemKey::UnsyncLyrics] {
                for raw in tags.iter().filter_map(|tag| tag.get_string(key)) {
                    if raw.len() > MAX_LYRIC_PAYLOAD_BYTES {
                        oversized = true;
                        continue;
                    }
                    let parsed = if looks_like_yrc(raw) {
                        parse_yrc(raw)
                    } else {
                        parse_lrc(raw)
                    };
                    if let Ok(lyrics) = parsed {
                        if !lyrics.lines.is_empty() {
                            return Ok((Some(lyrics), false));
                        }
                    }
                }
            }
            Ok((None, oversized))
        },
    )
    .await
    .map_err(|error| format!("讀取 Android 內嵌歌詞工作失敗：{error}"))??;
    match lookup {
        (Some(lyrics), _) => Ok(LocalLyricsLookup::Found(track_lyrics_dto(
            track.id,
            track,
            LyricProvider::Embedded,
            false,
            &lyrics,
        ))),
        (None, true) => Ok(LocalLyricsLookup::Oversized),
        (None, false) => Ok(LocalLyricsLookup::Missing),
    }
}

fn persisted_lyrics_dto(track: &TrackSummary, lyrics: TrackLyrics) -> TrackLyricsDto {
    track_lyrics_dto(
        track.id,
        track,
        lyrics.provider,
        lyrics.manually_selected,
        &lyrics.lyrics,
    )
}

fn track_lyrics_dto(
    track_id: TrackId,
    track: &TrackSummary,
    provider: LyricProvider,
    manually_selected: bool,
    lyrics: &ParsedLyrics,
) -> TrackLyricsDto {
    let source = if manually_selected {
        LyricsSourceDto::Manual
    } else {
        match provider {
            LyricProvider::Sidecar => LyricsSourceDto::Local,
            LyricProvider::Embedded => LyricsSourceDto::Embedded,
            LyricProvider::NetEase => LyricsSourceDto::NetEase,
            LyricProvider::Qq => LyricsSourceDto::QqMusic,
        }
    };
    TrackLyricsDto {
        track_id: track_id.to_string(),
        source,
        title: track.title.clone().unwrap_or_default(),
        artist: track.artist.clone(),
        album: track.album.clone(),
        duration_ms: track.duration_ms,
        offset_ms: 0,
        synced: lyrics.synced,
        lines: lyrics.lines.iter().map(line_dto).collect(),
    }
}

fn line_dto(line: &LyricLine) -> LyricLineDto {
    LyricLineDto {
        start_ms: line.start_ms,
        text: line.text.clone(),
        translation: line.translation.clone(),
        romanization: line.romanization.clone(),
    }
}

#[cfg(test)]
mod tests {
    use super::{
        fetch_provider_candidates, raw_to_lyric_candidate, LyricsService, LyricsSourceDto,
        MAX_SEARCH_CANDIDATES,
    };
    use crate::lyrics_provider::{
        HttpMethod, HttpRequest, HttpResponse, HttpTransport, LyricsProviderClient, ProviderError,
        RawKaraokeData, RawLyricCandidate,
    };
    use player_core::{
        parse_lrc, FileFingerprint, LibraryRepository, LibraryRoot, LyricProvider,
        LyricsTrackMetadata, MediaLocator, MediaSourceKind, MediaTrackRecord, SourceId,
        SourceScanState, TrackId, TrackIdentity, TrackMetadata, TrackSummary,
    };
    use player_db::Database;
    use std::{
        fs,
        future::Future,
        path::PathBuf,
        sync::{Arc, Mutex},
        time::{SystemTime, UNIX_EPOCH},
    };
    use tokio::sync::Notify;
    use tokio_util::sync::CancellationToken;

    #[derive(Clone, Default)]
    struct FixtureTransport {
        requests: Arc<Mutex<Vec<HttpRequest>>>,
    }

    impl HttpTransport for FixtureTransport {
        #[allow(clippy::manual_async_fn)]
        fn send<'a>(
            &'a self,
            request: HttpRequest,
            cancellation: &'a CancellationToken,
        ) -> impl Future<Output = Result<HttpResponse, ProviderError>> + Send + 'a {
            async move {
                if cancellation.is_cancelled() {
                    return Err(ProviderError::Cancelled);
                }
                self.requests
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner)
                    .push(request.clone());
                let response = if request.url.contains("/api/search/get/web")
                    || request.url.contains("/api/cloudsearch/pc")
                {
                    netease_search_fixture()
                } else if request.method == HttpMethod::Post {
                    qq_search_fixture()
                } else if request.url.contains("/api/song/lyric") {
                    serde_json::json!({
                        "lrc": {"lyric": "[00:01.00]primary"},
                        "tlyric": {"lyric": "[00:01.00]譯文"},
                        "yrc": {"lyric": "[1000,500](1000,500,0)primary"}
                    })
                } else if request.url.contains("c.y.qq.com/lyric/") {
                    serde_json::json!({
                        "lyric": "[00:01.00]primary",
                        "trans": "[00:01.00]譯文",
                        "roma": "[00:01.00]roma",
                        "qrc": "raw-qrc-payload"
                    })
                } else {
                    return Err(ProviderError::InvalidResponse(
                        "unexpected fixture request".to_owned(),
                    ));
                };
                Ok(HttpResponse {
                    status: 200,
                    body: serde_json::to_vec(&response)
                        .map_err(|error| ProviderError::InvalidResponse(error.to_string()))?,
                })
            }
        }
    }

    #[derive(Clone, Default)]
    struct CancellationTransport {
        started: Arc<Notify>,
    }

    impl HttpTransport for CancellationTransport {
        #[allow(clippy::manual_async_fn)]
        fn send<'a>(
            &'a self,
            _request: HttpRequest,
            cancellation: &'a CancellationToken,
        ) -> impl Future<Output = Result<HttpResponse, ProviderError>> + Send + 'a {
            async move {
                self.started.notify_one();
                cancellation.cancelled().await;
                Err(ProviderError::Cancelled)
            }
        }
    }

    fn netease_search_fixture() -> serde_json::Value {
        serde_json::json!({
            "result": {
                "songs": (0..25).map(|index| serde_json::json!({
                    "id": index.to_string(),
                    "name": "Song",
                    "artists": [{"name": "Artist"}],
                    "album": {"name": "Album"},
                    "duration": 180_000
                })).collect::<Vec<_>>()
            }
        })
    }

    fn qq_search_fixture() -> serde_json::Value {
        serde_json::json!({
            "req_1": {
                "data": {
                    "body": {
                        "song": {
                            "list": (0..25).map(|index| serde_json::json!({
                                "songmid": format!("qq-{index}"),
                                "songid": index,
                                "songname": "Song",
                                "singer": [{"name": "Artist"}],
                                "albumname": "Album",
                                "interval": 180
                            })).collect::<Vec<_>>()
                        }
                    }
                }
            }
        })
    }

    fn track() -> TrackSummary {
        TrackSummary {
            id: TrackId::new(),
            title: Some("測試曲目".to_owned()),
            artist: Some("測試演出者".to_owned()),
            album: Some("測試專輯".to_owned()),
            album_artist: None,
            track_number: None,
            disc_number: None,
            duration_ms: Some(180_000),
            codec: None,
            bitrate_bps: None,
            sample_rate_hz: None,
            year: None,
            bit_depth: None,
            played_ms: 0,
        }
    }

    struct TestDirectory(PathBuf);

    impl TestDirectory {
        fn new() -> Self {
            let nonce = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .expect("clock")
                .as_nanos();
            let path = std::env::temp_dir()
                .join(format!("moe-lyrics-service-{}-{nonce}", std::process::id()));
            fs::create_dir_all(&path).expect("create test directory");
            Self(path)
        }
    }

    impl Drop for TestDirectory {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    fn seeded_test_database(
        title: &str,
        artist: &str,
        album: &str,
    ) -> (Database, TestDirectory, TrackSummary) {
        let directory = TestDirectory::new();
        let audio_path = directory.0.join("track.mp3");
        fs::write(&audio_path, b"lyrics service test media").expect("write media placeholder");
        let source_id = SourceId::new();
        let root = LibraryRoot {
            id: source_id,
            kind: MediaSourceKind::WindowsFilesystem,
            display_name: "lyrics service test source".to_owned(),
            locator: MediaLocator::FileSystem(directory.0.clone()),
            enabled: true,
        };
        let mut database = Database::open_in_memory().expect("open isolated test database");
        database.save_library_root(&root).expect("save test source");
        let record = MediaTrackRecord {
            identity: TrackIdentity {
                source_id,
                source_item_id: "lyrics-service-track".to_owned(),
                locator_key: None,
            },
            locator: MediaLocator::FileSystem(audio_path.clone()),
            fingerprint: FileFingerprint {
                size_bytes: fs::metadata(&audio_path)
                    .expect("read media metadata")
                    .len(),
                modified_at_utc_ms: None,
            },
            metadata: Some(TrackMetadata {
                title: Some(title.to_owned()),
                artist: Some(artist.to_owned()),
                album: Some(album.to_owned()),
                duration_ms: Some(180_000),
                ..TrackMetadata::default()
            }),
        };
        database
            .apply_source_scan(
                &root,
                &SourceScanState::Complete,
                std::slice::from_ref(&record),
                std::slice::from_ref(&record),
                &[],
                1_800_000_000_000,
            )
            .expect("persist test track mapping");
        let track_id = database
            .resolve_track_id_for_locator(&MediaLocator::FileSystem(audio_path))
            .expect("resolve indexed track")
            .expect("track mapping exists");
        let track = database
            .get_track_summary(track_id)
            .expect("read track summary")
            .expect("track summary exists");
        (database, directory, track)
    }

    #[test]
    fn lyrics_dto_uses_the_frontend_camel_case_contract_and_adjusted_timestamps() {
        let track = track();
        let parsed = parse_lrc("[offset:250]\n[00:01.00]第一行").expect("parse LRC");
        let dto = super::track_lyrics_dto(track.id, &track, LyricProvider::Sidecar, false, &parsed);
        let value = serde_json::to_value(&dto).expect("serialize DTO");
        assert_eq!(value["trackId"], track.id.to_string());
        assert_eq!(value["source"], "local");
        assert_eq!(value["title"], "測試曲目");
        assert_eq!(value["durationMs"], 180_000);
        assert_eq!(value["offsetMs"], 0);
        assert_eq!(value["lines"][0]["startMs"], 1_250);
        assert_eq!(value["lines"][0]["translation"], serde_json::Value::Null);
    }

    #[test]
    fn manual_database_result_is_reported_as_manual_source() {
        use player_core::TrackLyrics;

        let track = track();
        let lyrics = parse_lrc("[00:01.00]人工選取").expect("parse LRC");
        let persisted = TrackLyrics {
            track_id: track.id,
            provider: LyricProvider::Qq,
            candidate_id: Some("qqmusic:123".to_owned()),
            lyrics,
            manually_selected: true,
            updated_at_utc_ms: 1_800_000_000_000,
        };
        let dto = super::persisted_lyrics_dto(&track, persisted);
        assert_eq!(serde_json::to_value(dto).unwrap()["source"], "manual");
    }

    #[test]
    fn provider_sources_match_the_typescript_union_exactly() {
        let track = track();
        let parsed = parse_lrc("[00:01.00]歌詞").expect("parse LRC");
        let netease =
            super::track_lyrics_dto(track.id, &track, LyricProvider::NetEase, false, &parsed);
        let qqmusic = super::track_lyrics_dto(track.id, &track, LyricProvider::Qq, false, &parsed);
        assert_eq!(serde_json::to_value(netease).unwrap()["source"], "netease");
        assert_eq!(serde_json::to_value(qqmusic).unwrap()["source"], "qqmusic");
        assert_eq!(
            serde_json::to_value(super::LyricsProviderDto::NetEase).unwrap(),
            "netease"
        );
        assert_eq!(
            serde_json::to_value(super::LyricsProviderDto::QqMusic).unwrap(),
            "qqmusic"
        );
    }

    #[test]
    fn lyrics_result_json_keys_match_the_typescript_contract() {
        use player_core::LyricCandidate;
        use serde_json::Value;
        use std::collections::BTreeSet;

        fn keys(value: &Value) -> BTreeSet<&str> {
            value
                .as_object()
                .expect("DTO serializes as an object")
                .keys()
                .map(String::as_str)
                .collect()
        }

        let track = track();
        let parsed = parse_lrc("[00:01.00]歌詞").expect("parse LRC");
        let lyrics =
            super::track_lyrics_dto(track.id, &track, LyricProvider::Sidecar, false, &parsed);
        let metadata = LyricsTrackMetadata {
            title: track.title.clone(),
            artist: track.artist.clone(),
            album: track.album.clone(),
            duration_ms: track.duration_ms,
        };
        let candidate = LyricCandidate {
            candidate_id: "netease:1".to_owned(),
            provider: LyricProvider::NetEase,
            provider_track_id: Some("1".to_owned()),
            title: Some("Song".to_owned()),
            artist: Some("Artist".to_owned()),
            album: None,
            duration_ms: None,
            lyrics: parsed,
            score: 0.5,
        };
        let result = super::LyricsTrackResultDto {
            lyrics: Some(lyrics),
            candidates: vec![super::candidate_dto(&metadata, &candidate)],
            status: super::LyricsResultStatusDto::Candidates,
            error: None,
        };
        let value = serde_json::to_value(result).expect("serialize result DTO");
        assert_eq!(
            keys(&value),
            ["candidates", "error", "lyrics", "status"]
                .into_iter()
                .collect()
        );
        assert_eq!(value["status"], "candidates");
        assert_eq!(
            keys(&value["lyrics"]),
            [
                "album",
                "artist",
                "durationMs",
                "lines",
                "offsetMs",
                "source",
                "synced",
                "title",
                "trackId",
            ]
            .into_iter()
            .collect()
        );
        assert_eq!(
            keys(&value["lyrics"]["lines"][0]),
            ["romanization", "startMs", "text", "translation"]
                .into_iter()
                .collect()
        );
        assert_eq!(value["lyrics"]["lines"][0]["romanization"], Value::Null);
        assert_eq!(
            keys(&value["candidates"][0]),
            [
                "album",
                "artist",
                "confidence",
                "durationMs",
                "hasSyncedLyrics",
                "id",
                "previewLines",
                "provider",
                "reasons",
                "score",
                "title",
            ]
            .into_iter()
            .collect()
        );
        assert_eq!(value["candidates"][0]["provider"], "netease");
        assert_eq!(value["candidates"][0]["confidence"], "low");
    }

    #[test]
    fn lrc_translation_and_romanization_merge_only_on_exact_timestamps_and_keep_yrc_raw() {
        let raw = RawLyricCandidate {
            candidate_id: "netease:1".to_owned(),
            provider: LyricProvider::NetEase,
            provider_track_id: "1".to_owned(),
            title: Some("Song".to_owned()),
            artist: Some("Artist".to_owned()),
            album: None,
            duration_ms: Some(180_000),
            source_url: "https://music.163.com/#/song?id=1".to_owned(),
            synced_lyrics: Some("[00:01.00]primary".to_owned()),
            plain_lyrics: None,
            translation: Some("[00:01.00]譯文".to_owned()),
            romanization: Some("[00:09.00]錯誤時間".to_owned()),
            raw_yrc: Some("[1000,500](1000,500,0)primary".to_owned()),
            raw_karaoke: None,
        };
        let candidate = raw_to_lyric_candidate(raw).expect("convert provider response");
        assert_eq!(candidate.lyrics.format, player_core::LyricFormat::Lrc);
        assert!(candidate.lyrics.synced);
        assert_eq!(
            candidate.lyrics.lines[0].translation.as_deref(),
            Some("譯文")
        );
        assert_eq!(candidate.lyrics.lines[0].romanization, None);
        let preserved: serde_json::Value = serde_json::from_str(
            candidate
                .lyrics
                .raw_karaoke
                .as_deref()
                .expect("retained YRC payload"),
        )
        .expect("parse retained karaoke metadata");
        assert_eq!(preserved["yrc"], "[1000,500](1000,500,0)primary");
    }

    #[test]
    fn yrc_only_collapses_to_line_start_and_qrc_only_stays_unsynced() {
        let yrc = raw_to_lyric_candidate(RawLyricCandidate {
            candidate_id: "netease:yrc".to_owned(),
            provider: LyricProvider::NetEase,
            provider_track_id: "yrc".to_owned(),
            title: Some("Song".to_owned()),
            artist: Some("Artist".to_owned()),
            album: None,
            duration_ms: None,
            source_url: String::new(),
            synced_lyrics: Some("[1200,700](1200,200,0)逐(1400,500,0)行".to_owned()),
            plain_lyrics: None,
            translation: None,
            romanization: None,
            raw_yrc: Some("[1200,700](1200,200,0)逐(1400,500,0)行".to_owned()),
            raw_karaoke: None,
        })
        .expect("convert YRC response");
        assert_eq!(yrc.lyrics.format, player_core::LyricFormat::Yrc);
        assert!(yrc.lyrics.synced);
        assert_eq!(yrc.lyrics.lines[0].start_ms, Some(1_200));
        assert_eq!(yrc.lyrics.lines[0].text, "逐行");
        assert!(yrc
            .lyrics
            .raw_karaoke
            .as_deref()
            .unwrap()
            .contains("\"yrc\""));

        let qrc = raw_to_lyric_candidate(RawLyricCandidate {
            candidate_id: "qqmusic:qrc".to_owned(),
            provider: LyricProvider::Qq,
            provider_track_id: "qrc".to_owned(),
            title: Some("Song".to_owned()),
            artist: Some("Artist".to_owned()),
            album: None,
            duration_ms: None,
            source_url: String::new(),
            synced_lyrics: None,
            plain_lyrics: None,
            translation: None,
            romanization: None,
            raw_yrc: None,
            raw_karaoke: Some(RawKaraokeData {
                qrc_base64: Some("cmF3LXFyYw==".to_owned()),
                translation_base64: None,
                romanization_base64: None,
            }),
        })
        .expect("convert QRC-only response");
        assert_eq!(qrc.lyrics.format, player_core::LyricFormat::Qrc);
        assert!(!qrc.lyrics.synced);
        assert!(qrc.lyrics.lines.is_empty());
        assert!(qrc.lyrics.raw_karaoke.is_some());
        let qrc_candidate = super::candidate_dto(
            &LyricsTrackMetadata {
                title: Some("Song".to_owned()),
                artist: Some("Artist".to_owned()),
                album: None,
                duration_ms: None,
            },
            &qrc,
        );
        assert!(!qrc_candidate.has_synced_lyrics);
        assert!(qrc_candidate
            .reasons
            .iter()
            .any(|reason| reason.contains("沒有可顯示的歌詞文字")));
    }

    #[tokio::test]
    async fn provider_results_are_bounded_and_fixture_transport_stays_offline() {
        let transport = FixtureTransport::default();
        let client = Arc::new(LyricsProviderClient::new(transport.clone()));
        let candidates = fetch_provider_candidates(
            client,
            LyricsTrackMetadata {
                title: Some("Song".to_owned()),
                artist: Some("Artist".to_owned()),
                album: Some("Album".to_owned()),
                duration_ms: Some(180_000),
            },
            CancellationToken::new(),
        )
        .await
        .expect("fixture provider search");
        let requests = transport
            .requests
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let lyric_fetches = requests
            .iter()
            .filter(|request| {
                request.url.contains("/api/song/lyric") || request.url.contains("c.y.qq.com/lyric/")
            })
            .count();
        let search_requests = requests
            .iter()
            .filter(|request| {
                request.url.contains("/api/search/get/web")
                    || request.url.contains("/api/cloudsearch/pc")
                    || request
                        .body
                        .as_deref()
                        .is_some_and(|body| body.contains("SearchCgiService"))
            })
            .count();
        assert!(candidates.len() <= MAX_SEARCH_CANDIDATES);
        assert!(candidates.len() <= 8);
        assert!(lyric_fetches <= 8);
        assert_eq!(search_requests, 3, "one QQ and at most two NetEase queries");
        assert!(requests
            .iter()
            .all(|request| request.url.starts_with("https://")));
    }

    #[tokio::test]
    async fn low_confidence_search_returns_candidates_without_saving_selected_lyrics() {
        let (database, _directory, track) =
            seeded_test_database("完全不同的本機曲名", "本機演出者", "本機專輯");
        let service = LyricsService::new(LyricsProviderClient::new(FixtureTransport::default()));
        let result = service
            .search_track(&database, track.id, &track, CancellationToken::new(), true, None)
            .await
            .expect("complete fixture search");

        assert!(matches!(
            result.status,
            super::LyricsResultStatusDto::Candidates
        ));
        assert!(result.lyrics.is_none());
        assert!(!result.candidates.is_empty());
        assert!(result
            .candidates
            .iter()
            .all(|candidate| candidate.score < super::HIGH_CONFIDENCE_SCORE));
        assert!(database
            .get_track_lyrics(track.id)
            .expect("read selected lyric cache")
            .is_none());
    }

    #[tokio::test]
    async fn high_confidence_search_persists_automatic_match_and_get_returns_it() {
        let (database, _directory, track) = seeded_test_database("Song", "Artist", "Album");
        let service = LyricsService::new(LyricsProviderClient::new(FixtureTransport::default()));
        let result = service
            .search_track(&database, track.id, &track, CancellationToken::new(), true, None)
            .await
            .expect("complete fixture search");

        assert!(matches!(result.status, super::LyricsResultStatusDto::Ready));
        let result_lyrics = result.lyrics.expect("high-confidence lyrics returned");
        assert!(matches!(
            result_lyrics.source,
            super::LyricsSourceDto::NetEase | super::LyricsSourceDto::QqMusic
        ));
        let stored = database
            .get_track_lyrics(track.id)
            .expect("read automatic lyric cache")
            .expect("automatic match is persisted");
        assert!(!stored.manually_selected);
        let candidate_id = stored
            .candidate_id
            .as_deref()
            .expect("matched candidate ID");
        match stored.provider {
            LyricProvider::NetEase => assert!(candidate_id.starts_with("netease:")),
            LyricProvider::Qq => assert!(candidate_id.starts_with("qqmusic:")),
            LyricProvider::Sidecar | LyricProvider::Embedded => {
                panic!("automatic provider cache must be remote")
            }
        }
        assert!(stored.lyrics.synced);

        let loaded = super::load_track_result(&database, track.id)
            .await
            .expect("load cached lyrics through get path");
        assert!(matches!(loaded.status, super::LyricsResultStatusDto::Ready));
        let loaded_lyrics = loaded.lyrics.expect("get returns cached lyrics");
        assert_eq!(loaded_lyrics.source, result_lyrics.source);
        assert_eq!(loaded_lyrics.lines, result_lyrics.lines);
    }

    #[tokio::test]
    async fn manual_search_skips_auto_apply_and_returns_candidates() {
        let (database, _directory, track) = seeded_test_database("Song", "Artist", "Album");
        let service = LyricsService::new(LyricsProviderClient::new(FixtureTransport::default()));
        let result = service
            .search_track(&database, track.id, &track, CancellationToken::new(), false, None)
            .await
            .expect("manual search without auto-apply");

        assert!(matches!(
            result.status,
            super::LyricsResultStatusDto::Candidates
        ));
        assert!(result.lyrics.is_none());
        assert!(!result.candidates.is_empty());
        assert!(database
            .get_track_lyrics(track.id)
            .expect("read lyric cache")
            .is_none());
    }

    #[tokio::test]
    async fn searched_candidate_is_cached_and_manual_selection_persists_auxiliary_and_raw_yrc() {
        let (database, _directory, track) =
            seeded_test_database("完全不同的本機曲名", "本機演出者", "本機專輯");
        let service = LyricsService::new(LyricsProviderClient::new(FixtureTransport::default()));
        let result = service
            .search_track(&database, track.id, &track, CancellationToken::new(), true, None)
            .await
            .expect("complete fixture search");
        let selected_candidate = result
            .candidates
            .iter()
            .find(|candidate| candidate.id == "netease:0")
            .expect("NetEase fixture candidate is offered")
            .id
            .clone();

        assert!(database
            .get_track_lyrics(track.id)
            .expect("read selected lyric cache")
            .is_none());
        let selected = super::select_candidate_from_database(
            &database,
            track.id,
            &selected_candidate,
            1_800_000_000_100,
        )
        .expect("select cached fixture candidate");
        assert_eq!(selected.source, super::LyricsSourceDto::Manual);
        assert_eq!(selected.lines[0].text, "primary");
        assert_eq!(selected.lines[0].start_ms, Some(1_000));
        assert_eq!(selected.lines[0].translation.as_deref(), Some("譯文"));

        let persisted = database
            .get_track_lyrics(track.id)
            .expect("read persisted selection")
            .expect("manual selection is stored");
        assert!(persisted.manually_selected);
        assert_eq!(persisted.candidate_id.as_deref(), Some("netease:0"));
        let raw: serde_json::Value = serde_json::from_str(
            persisted
                .lyrics
                .raw_karaoke
                .as_deref()
                .expect("raw YRC payload is persisted"),
        )
        .expect("parse preserved karaoke metadata");
        assert_eq!(raw["yrc"], "[1000,500](1000,500,0)primary");

        let later_automatic = player_core::TrackLyrics {
            track_id: track.id,
            provider: LyricProvider::NetEase,
            candidate_id: Some("netease:later".to_owned()),
            lyrics: parse_lrc("[00:02.00]later automatic").expect("parse later LRC"),
            manually_selected: false,
            updated_at_utc_ms: 1_800_000_000_200,
        };
        assert!(!database
            .save_automatic_track_lyrics(&later_automatic)
            .expect("attempt later automatic cache"));
        assert_eq!(
            database
                .get_track_lyrics(track.id)
                .expect("re-read selected lyric cache")
                .expect("manual lyrics remain")
                .candidate_id
                .as_deref(),
            Some("netease:0")
        );
    }

    #[tokio::test]
    async fn cancellation_during_provider_search_does_not_write_lyrics_or_candidates() {
        let (database, _directory, track) =
            seeded_test_database("完全不同的本機曲名", "本機演出者", "本機專輯");
        let transport = CancellationTransport::default();
        let started = transport.started.clone();
        let service = Arc::new(LyricsService::new(LyricsProviderClient::new(transport)));
        let database = Arc::new(database);
        let cancellation = CancellationToken::new();
        let search = tokio::spawn({
            let service = service.clone();
            let database = database.clone();
            let track = track.clone();
            let cancellation = cancellation.clone();
            async move {
                service
                    .search_track(&database, track.id, &track, cancellation, true, None)
                    .await
            }
        });

        started.notified().await;
        cancellation.cancel();
        let result = search
            .await
            .expect("provider search task completes")
            .expect("cancelled search returns an empty result");
        assert!(matches!(result.status, super::LyricsResultStatusDto::Empty));
        assert!(database
            .get_track_lyrics(track.id)
            .expect("read selected lyric cache")
            .is_none());
        assert!(database
            .select_lyric_candidate(track.id, "netease:0", 1_800_000_000_300)
            .is_err());
    }

    #[test]
    fn cancellation_is_keyed_by_request_id_and_replaces_prior_search() {
        let service = LyricsService::new(LyricsProviderClient::new(FixtureTransport::default()));
        let (first_token, first_guard) = service.begin_search("request-1").expect("start");
        let (second_token, second_guard) = service.begin_search("request-2").expect("replace");
        assert!(first_token.is_cancelled());
        assert!(!second_token.is_cancelled());
        service.cancel_search("request-2");
        assert!(second_token.is_cancelled());
        drop((first_guard, second_guard));
        assert!(service
            .active_searches
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .is_empty());
    }

    #[cfg(target_os = "windows")]
    #[tokio::test]
    async fn get_track_follows_enabled_database_locator_to_lrc_sidecar() {
        use player_core::{
            FileFingerprint, LibraryRepository, LibraryRoot, MediaLocator, MediaSourceKind,
            MediaTrackRecord, SourceId, SourceScanState, TrackIdentity, TrackMetadata,
        };
        use player_db::Database;
        use std::{
            fs,
            path::PathBuf,
            time::{SystemTime, UNIX_EPOCH},
        };

        struct TestDirectory(PathBuf);
        impl TestDirectory {
            fn new() -> Self {
                let nonce = SystemTime::now()
                    .duration_since(UNIX_EPOCH)
                    .expect("clock")
                    .as_nanos();
                let path = std::env::temp_dir()
                    .join(format!("moe-lyrics-get-{}-{nonce}", std::process::id()));
                fs::create_dir_all(&path).expect("create test directory");
                Self(path)
            }
        }
        impl Drop for TestDirectory {
            fn drop(&mut self) {
                let _ = fs::remove_dir_all(&self.0);
            }
        }

        let directory = TestDirectory::new();
        let audio_path = directory.0.join("local.mp3");
        fs::write(&audio_path, b"sidecar lookup does not need audio decoding")
            .expect("write local media placeholder");
        fs::write(
            directory.0.join("local.lrc"),
            "[offset:250]\n[00:01.00]本機逐行歌詞",
        )
        .expect("write local LRC sidecar");

        let source_id = SourceId::new();
        let root = LibraryRoot {
            id: source_id,
            kind: MediaSourceKind::WindowsFilesystem,
            display_name: "lyrics test source".to_owned(),
            locator: MediaLocator::FileSystem(directory.0.clone()),
            enabled: true,
        };
        let mut database = Database::open_in_memory().expect("open isolated test database");
        database.save_library_root(&root).expect("save test source");
        let fingerprint = FileFingerprint {
            size_bytes: fs::metadata(&audio_path).expect("media metadata").len(),
            modified_at_utc_ms: None,
        };
        let record = MediaTrackRecord {
            identity: TrackIdentity {
                source_id,
                source_item_id: "local-track".to_owned(),
                locator_key: None,
            },
            locator: MediaLocator::FileSystem(audio_path.clone()),
            fingerprint,
            metadata: Some(TrackMetadata {
                title: Some("測試曲目".to_owned()),
                artist: Some("測試演出者".to_owned()),
                album: Some("測試專輯".to_owned()),
                duration_ms: Some(180_000),
                ..TrackMetadata::default()
            }),
        };
        database
            .apply_source_scan(
                &root,
                &SourceScanState::Complete,
                std::slice::from_ref(&record),
                std::slice::from_ref(&record),
                &[],
                1_800_000_000_000,
            )
            .expect("persist test track mapping");
        let track_id = database
            .resolve_track_id_for_locator(&MediaLocator::FileSystem(audio_path))
            .expect("resolve indexed track")
            .expect("track mapping exists");

        let result = super::load_track_result(&database, track_id)
            .await
            .expect("load track lyrics");
        assert!(matches!(result.status, super::LyricsResultStatusDto::Ready));
        let lyrics = result.lyrics.expect("sidecar lyrics returned");
        assert_eq!(lyrics.source, LyricsSourceDto::Local);
        assert_eq!(lyrics.lines[0].text, "本機逐行歌詞");
        assert_eq!(lyrics.lines[0].start_ms, Some(1_250));
    }
}
