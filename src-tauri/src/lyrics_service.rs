//! Track-scoped lyric lookup and IPC DTO conversion.

use std::{
    collections::{HashMap, HashSet, VecDeque},
    sync::{
        atomic::{AtomicU64, Ordering},
        Arc, Mutex,
    },
    time::Duration,
};

use player_core::{
    auto_lyric_candidate, merge_lrc_auxiliary, merge_lrc_auxiliary_nearest, merge_same_timestamp_lines, parse_any_lyrics, parse_lrc, parse_yrc,
    restore_words_from_raw_karaoke, traditional_word_texts, yrc_matches_lrc, LyricsContent,
    preserve_qrc, rank_lyric_candidates, with_raw_karaoke, LyricAuxiliaryKind, LyricCandidate, LyricFormat, LyricLine, LyricProvider,
    LyricsTrackMetadata, ParsedLyrics, TrackId, TrackLyrics, TrackSummary,
};
use player_db::Database;
use serde::Serialize;
use tauri::{AppHandle, State};
use tokio::{
    sync::{OnceCell, Semaphore},
    task::JoinSet,
};
use tokio_util::sync::CancellationToken;

use crate::lyrics_provider::{
    LyricsProviderClient, ProviderCandidate, ProviderError, RawLyricCandidate, ReqwestHttpTransport,
};

const LYRIC_BATCH_SIZE: usize = 10;
const MAX_CONCURRENT_LYRIC_FETCHES: usize = 3;
const HIGH_CONFIDENCE_SCORE: f32 = 0.90;
const MAX_REQUEST_ID_BYTES: usize = 128;
const MAX_PROVIDER_SEARCH_DURATION: Duration = Duration::from_secs(20);
const QQ_MISSING_RETRY_DELAY: Duration = Duration::from_millis(500);

#[cfg(target_os = "windows")]
use player_platform_windows::{find_all_lyrics, find_embedded_lyrics, find_lyrics, LyricsLookup};

const LOCAL_SIDECAR_CANDIDATE_ID: &str = "local:sidecar";
const LOCAL_EMBEDDED_CANDIDATE_ID: &str = "local:embedded";
/// Persisted after 「移除歌詞」: blocks local rediscovery and automatic provider search until
/// the user manually picks lyrics again.
const CLEARED_LYRICS_CANDIDATE_ID: &str = "cleared:";

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
    /// `text` in Taiwan Traditional glyphs (player-zh, ADR 0030), only when that differs; the
    /// Simplified-to-Traditional display preference picks it without another round trip.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub text_traditional: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub translation_traditional: Option<String>,
    /// The end of the line when its source gives one (SRT, ASS, yrc).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub end_ms: Option<u64>,
    /// Word timing (ADR 0041), short keys to keep the IPC small; empty for line-level lyrics.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub words: Vec<LyricWordDto>,
    /// The Traditional text of each word, cut at the same places, only when it differs.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub words_traditional: Option<Vec<String>>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct LyricWordDto {
    pub s: u64,
    pub e: u64,
    pub t: String,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TrackLyricsDto {
    pub track_id: String,
    pub source: LyricsSourceDto,
    /// Underlying provider even when `source` is `manual` (the song panel's import button
    /// only accepts online providers).
    pub provider: LyricsProviderDto,
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
    /// User cleared lyrics for this track; do not auto-load local or auto-search providers.
    Cleared,
    Candidates,
    Error,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
pub enum LyricsProviderDto {
    #[serde(rename = "netease")]
    NetEase,
    #[serde(rename = "qqmusic")]
    QqMusic,
    #[serde(rename = "local")]
    Local,
    #[serde(rename = "embedded")]
    Embedded,
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
    pub has_word_synced_lyrics: bool,
    pub has_translation: bool,
    pub has_romanization: bool,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LyricsTrackResultDto {
    pub lyrics: Option<TrackLyricsDto>,
    pub candidates: Vec<LyricsCandidateDto>,
    pub status: LyricsResultStatusDto,
    pub error: Option<String>,
    /// More provider songs can still be downloaded, ten lyrics at a time.
    pub has_more: bool,
}

struct ActiveLyricsSearch {
    generation: u64,
    cancellation: CancellationToken,
}

struct LyricBrowseSession {
    track_id: TrackId,
    score_metadata: LyricsTrackMetadata,
    search_metadata: Vec<LyricsTrackMetadata>,
    pending: VecDeque<ProviderCandidate>,
    seen: HashSet<String>,
    /// Next 1-based provider page to request once `pending` is empty.
    next_page: u32,
    exhausted: bool,
    generation: u64,
}

/// What one automatic search produced, shared by every window that asked for the same track.
#[derive(Clone)]
struct AutoSearchOutcome {
    result: Result<LyricsTrackResultDto, String>,
    /// The search was cancelled before it finished: it says nothing about the lyrics, so a window that was only
    /// waiting for it must search again itself.
    cancelled: bool,
}

pub struct LyricsService<T: crate::lyrics_provider::HttpTransport = ReqwestHttpTransport> {
    client: Arc<LyricsProviderClient<T>>,
    active_searches: Arc<Mutex<HashMap<String, ActiveLyricsSearch>>>,
    next_generation: AtomicU64,
    browse: Mutex<Option<LyricBrowseSession>>,
    /// Automatic searches in flight, one per track. The main window's lyrics view and the taskbar-lyrics window both
    /// look for lyrics when a track starts, and a second search would cancel the first (`begin_search` keeps one
    /// search alive), which showed "no lyrics found" in the window whose search was cancelled.
    auto_flights: Mutex<HashMap<TrackId, Arc<OnceCell<AutoSearchOutcome>>>>,
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
            browse: Mutex::new(None),
            auto_flights: Mutex::new(HashMap::new()),
        }
    }

    /// Run `search` for an automatic request on `track_id`, or wait for the identical search another window already
    /// started and share its result. `search` returns the result and whether its search was cancelled; a cancelled
    /// search someone else started is not trusted, the caller then searches itself.
    async fn shared_auto_search<F, Fut>(
        &self,
        track_id: TrackId,
        search: F,
    ) -> Result<LyricsTrackResultDto, String>
    where
        F: Fn() -> Fut,
        Fut: std::future::Future<Output = (Result<LyricsTrackResultDto, String>, bool)>,
    {
        loop {
            let (flight, leader) = {
                let mut flights = self
                    .auto_flights
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner);
                match flights.get(&track_id) {
                    Some(flight) => (Arc::clone(flight), false),
                    None => {
                        let flight = Arc::new(OnceCell::new());
                        flights.insert(track_id, Arc::clone(&flight));
                        (flight, true)
                    }
                }
            };
            let outcome = flight
                .get_or_init(|| async {
                    let (result, cancelled) = search().await;
                    AutoSearchOutcome { result, cancelled }
                })
                .await
                .clone();
            {
                let mut flights = self
                    .auto_flights
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner);
                if flights
                    .get(&track_id)
                    .is_some_and(|current| Arc::ptr_eq(current, &flight))
                {
                    flights.remove(&track_id);
                }
            }
            if outcome.cancelled && !leader {
                continue;
            }
            return outcome.result;
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

    /// Provider search for one track. QQ Music and NetEase index mainly Simplified text, so the
    /// query (or the track's title, artist and album) is always converted to Simplified first and
    /// only that one spelling is searched (ADR 0030); Japanese text with kana stays as it is.
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
        let simplified = |value: &Option<String>| {
            value
                .as_deref()
                .map(|value| player_zh::to_simplified(value).into_owned())
        };
        let search_metadata = match query_override
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
        {
            Some(query) => vec![LyricsTrackMetadata {
                title: Some(player_zh::to_simplified(query).into_owned()),
                artist: None,
                album: None,
                duration_ms: metadata.duration_ms,
            }],
            None => vec![LyricsTrackMetadata {
                title: simplified(&metadata.title),
                artist: simplified(&metadata.artist),
                album: simplified(&metadata.album),
                duration_ms: metadata.duration_ms,
            }],
        };
        self.open_browse(track_id, metadata.clone(), search_metadata);
        let (candidates, has_more) = match tokio::time::timeout(
            MAX_PROVIDER_SEARCH_DURATION,
            self.next_lyric_batch(&cancellation, LYRIC_BATCH_SIZE),
        )
        .await
        {
            Err(_) => {
                cancellation.cancel();
                return Ok(LyricsTrackResultDto::empty(Some(provider_error_message(
                    &ProviderError::Timeout,
                ))));
            }
            Ok(Ok(batch)) => batch,
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
            if let Some(best) = auto_lyric_candidate(&metadata, &ranked) {
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
                    if let Some(lyrics) = manual_lyrics_dto(database, track, &selected)? {
                        return Ok(LyricsTrackResultDto::ready(lyrics));
                    }
                }
            }
        }

        let candidates = ranked
            .iter()
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
                has_more,
            })
        }
    }

    fn open_browse(
        &self,
        track_id: TrackId,
        score_metadata: LyricsTrackMetadata,
        search_metadata: Vec<LyricsTrackMetadata>,
    ) {
        *self
            .browse
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = Some(LyricBrowseSession {
            track_id,
            score_metadata,
            search_metadata,
            pending: VecDeque::new(),
            seen: HashSet::new(),
            next_page: 1,
            exhausted: false,
            generation: self.next_generation.fetch_add(1, Ordering::Relaxed),
        });
    }

    async fn next_lyric_batch(
        &self,
        cancellation: &CancellationToken,
        limit: usize,
    ) -> Result<(Vec<LyricCandidate>, bool), ProviderError> {
        let mut session = self
            .browse
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .take()
            .ok_or_else(|| ProviderError::InvalidResponse("lyric search is not active".into()))?;
        let generation = session.generation;
        let pulled = pull_lyric_batch(self.client.clone(), &mut session, cancellation, limit).await;
        let has_more = !session.pending.is_empty() || !session.exhausted;
        let mut slot = self
            .browse
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if slot
            .as_ref()
            .is_none_or(|current| current.generation == generation)
        {
            *slot = Some(session);
        }
        pulled.map(|candidates| (candidates, has_more))
    }

    pub async fn load_more_track(
        &self,
        database: &Database,
        track_id: TrackId,
        cancellation: CancellationToken,
    ) -> Result<LyricsTrackResultDto, String> {
        if self
            .browse
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .as_ref()
            .is_none_or(|session| session.track_id != track_id)
        {
            return Ok(LyricsTrackResultDto::empty(None));
        }
        let (candidates, has_more) = match tokio::time::timeout(
            MAX_PROVIDER_SEARCH_DURATION,
            self.next_lyric_batch(&cancellation, LYRIC_BATCH_SIZE),
        )
        .await
        {
            Err(_) => {
                cancellation.cancel();
                return Ok(LyricsTrackResultDto::empty(Some(provider_error_message(
                    &ProviderError::Timeout,
                ))));
            }
            Ok(Ok(batch)) => batch,
            Ok(Err(ProviderError::Cancelled)) => return Ok(LyricsTrackResultDto::empty(None)),
            Ok(Err(error)) => {
                return Ok(LyricsTrackResultDto::empty(Some(provider_error_message(
                    &error,
                ))))
            }
        };
        if candidates.is_empty() {
            return Ok(LyricsTrackResultDto {
                lyrics: None,
                candidates: Vec::new(),
                status: LyricsResultStatusDto::Candidates,
                error: None,
                has_more: false,
            });
        }
        let now = super::now_utc_epoch_ms()?;
        database
            .append_lyric_candidates(track_id, &candidates, now)
            .map_err(|error| error.to_string())?;
        let metadata = self
            .browse
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .as_ref()
            .map(|session| session.score_metadata.clone())
            .unwrap_or_else(|| LyricsTrackMetadata {
                title: None,
                artist: None,
                album: None,
                duration_ms: None,
            });
        Ok(LyricsTrackResultDto {
            lyrics: None,
            candidates: candidates
                .iter()
                .map(|candidate| candidate_dto(&metadata, candidate))
                .collect(),
            status: LyricsResultStatusDto::Candidates,
            error: None,
            has_more,
        })
    }
}

#[cfg(test)]
async fn fetch_provider_candidates<T: crate::lyrics_provider::HttpTransport + 'static>(
    client: Arc<LyricsProviderClient<T>>,
    metadata: LyricsTrackMetadata,
    cancellation: CancellationToken,
) -> Result<Vec<LyricCandidate>, ProviderError> {
    let mut session = LyricBrowseSession {
        track_id: TrackId::new(),
        score_metadata: metadata.clone(),
        search_metadata: vec![metadata],
        pending: VecDeque::new(),
        seen: HashSet::new(),
        next_page: 1,
        exhausted: false,
        generation: 0,
    };
    pull_lyric_batch(client, &mut session, &cancellation, LYRIC_BATCH_SIZE).await
}

async fn pull_lyric_batch<T: crate::lyrics_provider::HttpTransport + 'static>(
    client: Arc<LyricsProviderClient<T>>,
    session: &mut LyricBrowseSession,
    cancellation: &CancellationToken,
    limit: usize,
) -> Result<Vec<LyricCandidate>, ProviderError> {
    let mut batch = Vec::new();
    while batch.len() < limit {
        if cancellation.is_cancelled() {
            return Err(ProviderError::Cancelled);
        }
        if let Some(hit) = session.pending.pop_front() {
            batch.push(hit);
            continue;
        }
        if session.exhausted {
            break;
        }
        let page = session.next_page;
        let (fresh, might_have_more) =
            load_search_page(client.clone(), &session.search_metadata, page, cancellation).await?;
        session.next_page = page.saturating_add(1);
        let mut added = Vec::new();
        for hit in fresh {
            if hit.candidate_id.is_empty()
                || hit.candidate_id.len() > 256
                || hit.provider_track_id.len() > 128
            {
                continue;
            }
            if session.seen.insert(hit.candidate_id.clone()) {
                added.push(hit);
            }
        }
        if added.is_empty() || !might_have_more {
            session.exhausted = true;
        }
        if added.is_empty() {
            break;
        }
        let mut queued: Vec<_> = session.pending.drain(..).collect();
        queued.extend(added);
        let ranked = rank_lyric_candidates(
            &session.score_metadata,
            queued.iter().map(provider_hit_to_lyric_candidate),
        );
        let mut by_id = queued
            .into_iter()
            .map(|hit| (hit.candidate_id.clone(), hit))
            .collect::<HashMap<_, _>>();
        for candidate in ranked {
            if let Some(hit) = by_id.remove(&candidate.candidate_id) {
                session.pending.push_back(hit);
            }
        }
    }
    fetch_lyrics_for_hits(client, batch, cancellation).await
}

async fn load_search_page<T: crate::lyrics_provider::HttpTransport + 'static>(
    client: Arc<LyricsProviderClient<T>>,
    metadata_variants: &[LyricsTrackMetadata],
    page: u32,
    cancellation: &CancellationToken,
) -> Result<(Vec<ProviderCandidate>, bool), ProviderError> {
    let mut variant_hits = vec![Vec::new(); metadata_variants.len()];
    let mut might_have_more = false;
    let mut successful_requests = 0usize;
    let mut first_error = None;

    // Keep script variants in the supplied order (Traditional before Simplified),
    // while querying both lyric providers concurrently within each variant.
    for (variant_index, metadata) in metadata_variants.iter().enumerate() {
        if cancellation.is_cancelled() {
            return Err(ProviderError::Cancelled);
        }
        let (netease, mut qqmusic) = tokio::join!(
            client.search_page(LyricProvider::NetEase, metadata, page, cancellation),
            client.search_page(LyricProvider::Qq, metadata, page, cancellation),
        );
        let qq_missing = match &qqmusic {
            Ok(hits) => hits.is_empty(),
            Err(ProviderError::Cancelled) => return Err(ProviderError::Cancelled),
            Err(_) => true,
        };
        if page <= 1 && qq_missing && !cancellation.is_cancelled() {
            tokio::select! {
                _ = cancellation.cancelled() => return Err(ProviderError::Cancelled),
                _ = tokio::time::sleep(QQ_MISSING_RETRY_DELAY) => {}
            }
            qqmusic = client
                .search_page(LyricProvider::Qq, metadata, page, cancellation)
                .await;
        }

        for result in [netease, qqmusic] {
            match result {
                Ok(mut provider_hits) => {
                    successful_requests += 1;
                    might_have_more |= provider_hits.len() >= 10;
                    variant_hits[variant_index].append(&mut provider_hits);
                }
                Err(ProviderError::Cancelled) => return Err(ProviderError::Cancelled),
                Err(error) => {
                    if first_error.is_none() {
                        first_error = Some(error);
                    }
                }
            }
        }
    }
    if successful_requests == 0 {
        return Err(first_error.unwrap_or(ProviderError::InvalidResponse(
            "provider search failed".to_owned(),
        )));
    }
    let hits = variant_hits.into_iter().flatten().collect();
    Ok((hits, might_have_more))
}

async fn fetch_lyrics_for_hits<T: crate::lyrics_provider::HttpTransport + 'static>(
    client: Arc<LyricsProviderClient<T>>,
    hits: Vec<ProviderCandidate>,
    cancellation: &CancellationToken,
) -> Result<Vec<LyricCandidate>, ProviderError> {
    if hits.is_empty() {
        return Ok(Vec::new());
    }
    let permits = Arc::new(Semaphore::new(MAX_CONCURRENT_LYRIC_FETCHES));
    let mut tasks = JoinSet::new();
    for hit in hits {
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
    // NetEase word timing (ADR 0041): the yrc becomes the lyrics when it covers the LRC; the
    // translation and romanization aligned to it (`ytlrc`/`yromalrc`) are matched exactly, the
    // LRC-aligned ones (`tlyric`/`romalrc`) to the nearest line within 300 ms.
    if let Some(yrc) = raw
        .raw_yrc
        .as_deref()
        .and_then(|text| parse_yrc(text).ok())
        .filter(|yrc| lyrics.format != LyricFormat::Yrc && yrc_matches_lrc(yrc, &lyrics))
    {
        lyrics = yrc;
    }
    if lyrics.format == LyricFormat::Yrc {
        let to_error = |error: player_core::lyrics::LyricsParseError| {
            ProviderError::InvalidResponse(error.to_string())
        };
        for (aligned, fallback, kind) in [
            (
                raw.yrc_translation.as_deref(),
                raw.translation.as_deref(),
                LyricAuxiliaryKind::Translation,
            ),
            (
                raw.yrc_romanization.as_deref(),
                raw.romanization.as_deref(),
                LyricAuxiliaryKind::Romanization,
            ),
        ] {
            if let Some(aligned) = aligned {
                merge_lrc_auxiliary(&mut lyrics, aligned, kind).map_err(to_error)?;
            }
            if let Some(fallback) = fallback {
                merge_lrc_auxiliary_nearest(&mut lyrics, fallback, kind, 300).map_err(to_error)?;
            }
        }
    } else if lyrics.synced {
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

fn candidate_dto(_metadata: &LyricsTrackMetadata, candidate: &LyricCandidate) -> LyricsCandidateDto {
    let confidence = if candidate.score >= HIGH_CONFIDENCE_SCORE {
        LyricsConfidenceDto::High
    } else if candidate.score >= 0.72 {
        LyricsConfidenceDto::Medium
    } else {
        LyricsConfidenceDto::Low
    };
    // Only notes about the candidate itself; the title / artist / length / album comparison is not
    // listed (user decision 2026-10-22: the picker no longer shows a match-explanation line).
    let mut reasons = Vec::new();
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
            LyricProvider::Sidecar => LyricsProviderDto::Local,
            LyricProvider::Embedded => LyricsProviderDto::Embedded,
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
        has_word_synced_lyrics: has_word_synced_lyrics(&candidate.lyrics),
        has_translation: has_translation(&candidate.lyrics),
        has_romanization: has_romanization(&candidate.lyrics),
    }
}

/// Whether the lyrics a player would show have word timing. Encrypted QRC kept alongside a QQ
/// result does not count: it is never decoded (ADR 0041).
fn has_word_synced_lyrics(lyrics: &ParsedLyrics) -> bool {
    lyrics.lines.iter().any(|line| !line.words.is_empty())
}

fn has_translation(lyrics: &ParsedLyrics) -> bool {
    lyrics.lines.iter().any(|line| {
        line.translation
            .as_deref()
            .is_some_and(|text| !text.trim().is_empty())
    }) || preserved_karaoke_has_nonempty_field(lyrics, &["translation_base64", "translationBase64"])
}

fn has_romanization(lyrics: &ParsedLyrics) -> bool {
    lyrics.lines.iter().any(|line| {
        line.romanization
            .as_deref()
            .is_some_and(|text| !text.trim().is_empty())
    }) || preserved_karaoke_has_nonempty_field(
        lyrics,
        &["romanization_base64", "romanizationBase64"],
    )
}

fn preserved_karaoke_has_nonempty_field(lyrics: &ParsedLyrics, keys: &[&str]) -> bool {
    let Some(raw_karaoke) = lyrics.raw_karaoke.as_deref() else {
        return false;
    };
    let Ok(preserved) = serde_json::from_str::<serde_json::Value>(raw_karaoke) else {
        return false;
    };
    keys.iter().any(|key| {
        preserved
            .get(key)
            .and_then(serde_json::Value::as_str)
            .is_some_and(|payload| !payload.trim().is_empty())
    })
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
            has_more: false,
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
            has_more: false,
        }
    }

    fn cleared() -> Self {
        Self {
            lyrics: None,
            candidates: Vec::new(),
            status: LyricsResultStatusDto::Cleared,
            error: None,
            has_more: false,
        }
    }
}

fn is_cleared_lyrics(lyrics: &TrackLyrics) -> bool {
    lyrics.manually_selected
        && lyrics.candidate_id.as_deref() == Some(CLEARED_LYRICS_CANDIDATE_ID)
        && lyrics.lyrics.lines.is_empty()
        && lyrics.lyrics.raw_text.is_empty()
}

#[tauri::command]
pub async fn lyrics_get_track(
    state: State<'_, super::AppState>,
    app: AppHandle,
    track_id: String,
) -> Result<LyricsTrackResultDto, String> {
    let track_id = TrackId::parse(&track_id).map_err(|_| "曲目識別碼無效。".to_owned())?;
    let database = super::lyrics_database_for_track(&state, track_id)?;
    let settings = state
        .settings
        .snapshot()
        .map_err(|error| error.to_string())?;
    let started = std::time::Instant::now();
    let result = load_track_result_with_context(
        database.as_database(),
        track_id,
        Some(&app),
        &settings.sources,
    )
    .await;
    eprintln!("[lyrics] lyrics_get_track took {} ms", started.elapsed().as_millis());
    result
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
    if manual.unwrap_or(false) {
        let never_cancelled = Mutex::new(None);
        return lyrics_search_body(
            &state,
            &app,
            track_id,
            &request_id,
            true,
            query,
            &never_cancelled,
        )
        .await;
    }
    // Automatic: every window that asks about this track shares one provider search.
    let state_ref: &super::AppState = &state;
    state
        .lyrics_service
        .shared_auto_search(track_id, || async {
            let token_slot = Mutex::new(None);
            let result = lyrics_search_body(
                state_ref,
                &app,
                track_id,
                &request_id,
                false,
                query.clone(),
                &token_slot,
            )
            .await;
            let cancelled = token_slot
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .as_ref()
                .is_some_and(CancellationToken::is_cancelled);
            (result, cancelled)
        })
        .await
}

#[allow(clippy::too_many_arguments)]
async fn lyrics_search_body(
    state: &super::AppState,
    app: &AppHandle,
    track_id: TrackId,
    request_id: &str,
    manual: bool,
    query: Option<String>,
    token_slot: &Mutex<Option<CancellationToken>>,
) -> Result<LyricsTrackResultDto, String> {
    let database = super::lyrics_database_for_track(state, track_id)?;

    let (cancellation, _guard) = state.lyrics_service.begin_search(request_id)?;
    *token_slot
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner) = Some(cancellation.clone());
    let settings = state
        .settings
        .snapshot()
        .map_err(|error| error.to_string())?;
    let initial = load_track_result_with_context(
        database.as_database(),
        track_id,
        Some(app),
        &settings.sources,
    )
    .await?;
    // Automatic follow-up after an empty get may rediscover local/embedded lyrics.
    // Explicit manual selection must continue to provider candidates even when lyrics exist.
    // Cleared tracks skip automatic provider search until the user searches manually.
    if !manual
        && (initial.lyrics.is_some() || matches!(initial.status, LyricsResultStatusDto::Cleared))
    {
        return Ok(initial);
    }
    if cancellation.is_cancelled() {
        return Ok(LyricsTrackResultDto::empty(None));
    }
    let track = database
        .as_database()
        .get_track_summary(track_id)
        .map_err(|error| error.to_string())?
        .ok_or_else(|| "找不到這首曲目。".to_owned())?;
    let query = query
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_owned);
    let local_candidates = if manual {
        local_source_candidates(database.as_database(), track_id, &track)
            .await
            .unwrap_or_default()
    } else {
        Vec::new()
    };
    let searched = state
        .lyrics_service
        .search_track(
            database.as_database(),
            track_id,
            &track,
            cancellation,
            !manual,
            query,
        )
        .await;
    let mut result = match searched {
        Ok(result) => result,
        Err(_) if manual && !local_candidates.is_empty() => LyricsTrackResultDto {
            lyrics: None,
            candidates: Vec::new(),
            status: LyricsResultStatusDto::Candidates,
            error: None,
            has_more: false,
        },
        Err(error) => return Err(error),
    };
    if manual {
        let mut merged = local_candidates;
        merged.append(&mut result.candidates);
        result.candidates = merged;
        if result.lyrics.is_none() {
            // Keep already-playing lyrics so dismiss can restore playback UI without reload.
            result.lyrics = initial.lyrics;
        }
    }
    Ok(result)
}

#[tauri::command]
pub async fn lyrics_load_more(
    state: State<'_, super::AppState>,
    track_id: String,
) -> Result<LyricsTrackResultDto, String> {
    let _database_work = super::enter_database_work(&state)?;
    let track_id = TrackId::parse(&track_id).map_err(|_| "曲目識別碼無效。".to_owned())?;
    let database = super::lyrics_database_for_track(&state, track_id)?;
    state
        .lyrics_service
        .load_more_track(database.as_database(), track_id, CancellationToken::new())
        .await
}

#[tauri::command]
pub fn lyrics_cancel_search(state: State<'_, super::AppState>, request_id: String) {
    state.lyrics_service.cancel_search(&request_id);
}

#[tauri::command]
pub async fn lyrics_clear_track(
    state: State<'_, super::AppState>,
    app: AppHandle,
    track_id: String,
) -> Result<LyricsTrackResultDto, String> {
    let _database_work = super::enter_database_work(&state)?;
    let track_id = TrackId::parse(&track_id).map_err(|_| "曲目識別碼無效。".to_owned())?;
    let database = super::lyrics_database_for_track(&state, track_id)?;
    let _ = app;
    let now = super::now_utc_epoch_ms()?;
    // Persist an empty manual sentinel so local sidecar/embedded lyrics and automatic provider
    // search stay suppressed until the user manually picks lyrics again.
    database
        .as_database()
        .clear_track_lyrics(track_id)
        .map_err(|error| error.to_string())?;
    database
        .as_database()
        .save_manual_track_lyrics(&TrackLyrics {
            track_id,
            provider: LyricProvider::Sidecar,
            candidate_id: Some(CLEARED_LYRICS_CANDIDATE_ID.to_owned()),
            lyrics: empty_parsed_lyrics(),
            manually_selected: true,
            updated_at_utc_ms: now,
        })
        .map_err(|error| error.to_string())?;
    Ok(LyricsTrackResultDto::cleared())
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
    let database = super::lyrics_database_for_track(&state, track_id)?;
    select_candidate_from_database(
        database.as_database(),
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
    if let Some(provider) = local_candidate_provider(candidate_id) {
        return select_local_lyric_candidate(
            database,
            track_id,
            provider,
            candidate_id,
            selected_at_utc_ms,
        );
    }
    let track = database
        .get_track_summary(track_id)
        .map_err(|error| error.to_string())?
        .ok_or_else(|| "找不到這首曲目。".to_owned())?;
    let selected = database
        .select_lyric_candidate(track_id, candidate_id, selected_at_utc_ms)
        .map_err(|error| error.to_string())?;
    Ok(persisted_lyrics_dto(&track, selected))
}

fn local_candidate_provider(candidate_id: &str) -> Option<LyricProvider> {
    match candidate_id {
        LOCAL_SIDECAR_CANDIDATE_ID => Some(LyricProvider::Sidecar),
        LOCAL_EMBEDDED_CANDIDATE_ID => Some(LyricProvider::Embedded),
        _ => None,
    }
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

    // Resolution order: a lyric candidate the user explicitly picked is linked to this TrackId in
    // the library database and wins over local sidecar/embedded lyrics until it is cleared or
    // re-picked. A cleared sentinel (empty manual row) suppresses local rediscovery and auto
    // search. Without a manual link, local lyrics beat the automatic provider cache.
    // A pick of the song's own `.lrc` or embedded lyrics is a reference, read live (ADR 0013 lyrics
    // note, 2026-10-22): when that source is empty now, the normal order below applies, and the
    // row's stored text (rows from before this rule kept a snapshot) is never used.
    let mut cached = cached;
    if let Some(selected) = cached.as_ref().filter(|cached| cached.manually_selected) {
        if is_cleared_lyrics(selected) {
            return Ok(LyricsTrackResultDto::cleared());
        }
        match manual_lyrics_dto(database, &track, selected)? {
            Some(lyrics) => return Ok(LyricsTrackResultDto::ready(lyrics)),
            None => cached = None,
        }
    }

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

/// What the lookup of the local lyrics (beside and inside the audio file) found, before it becomes a DTO.
#[derive(Clone, Debug, serde::Deserialize, serde::Serialize)]
#[serde(tag = "state", rename_all = "camelCase")]
enum RawLocalLyrics {
    Found {
        provider: LyricProvider,
        lyrics: ParsedLyrics,
    },
    Missing,
    Oversized,
}

#[cfg(target_os = "windows")]
async fn raw_local_lyrics(
    _app: Option<&AppHandle>,
    database: &Database,
    track_id: TrackId,
    embedded_only: bool,
) -> Result<RawLocalLyrics, String> {
    let locators = database
        .track_locators(track_id)
        .map_err(|error| error.to_string())?;
    if locators.is_empty() {
        return Ok(RawLocalLyrics::Missing);
    }
    let lookup = tauri::async_runtime::spawn_blocking(move || {
        if embedded_only {
            find_embedded_lyrics(&locators)
        } else {
            find_lyrics(&locators)
        }
    })
        .await
        .map_err(|error| format!("讀取本機歌詞工作失敗：{error}"))?;
    Ok(match lookup {
        LyricsLookup::Found(found) => RawLocalLyrics::Found {
            provider: found.provider,
            lyrics: found.lyrics,
        },
        LyricsLookup::Missing => RawLocalLyrics::Missing,
        LyricsLookup::Oversized => RawLocalLyrics::Oversized,
    })
}

/// The lyrics beside and inside the track's file.
///
/// The `.lrc` beside a track is a library fact the sync keeps (found while the source is scanned, stored with its
/// fingerprint and text), so playing a track never looks for it in a folder or on a server. Only a source that was never
/// scanned for sidecars (a library from before this existed, until its next sync) is still searched. What the file itself
/// holds (embedded lyrics) is read from the file; for a track on a remote server that is a network round trip or several, so
/// what was found is kept in the database next to the size and modified time of the file it was read for.
async fn local_lyrics_for_track(
    app: Option<&AppHandle>,
    _sources: &[super::settings::SourceEntry],
    database: &Database,
    track_id: TrackId,
    track: &TrackSummary,
) -> Result<LocalLyricsLookup, String> {
    // Order (ADR 0041): the song's own embedded lyrics, then the sidecar beside it (`.lrc`, `.ass`,
    // `.srt`, whichever the sync kept), then nothing here (the caller searches online). Content
    // that looks like a subtitle but does not parse counts as absent.
    let started = std::time::Instant::now();
    let embedded_known = match database
        .track_embedded_lyrics(track_id)
        .map_err(|error| error.to_string())?
    {
        Some(stored) => {
            if let Some(lyrics) = stored.as_deref().and_then(parse_stored_lyrics) {
                return Ok(LocalLyricsLookup::Found(track_lyrics_dto(
                    track.id,
                    track,
                    LyricProvider::Embedded,
                    false,
                    &lyrics,
                )));
            }
            true
        }
        // Tags not read since this existed: look in the file below.
        None => false,
    };
    let mut embedded_only = false;
    let mut sidecar_oversized = false;
    match database
        .track_lyric_sidecar(track_id)
        .map_err(|error| error.to_string())?
    {
        player_db::LyricSidecarLookup::Present { text: Some(text), .. } => {
            if let Some(lyrics) = parse_stored_lyrics(&text) {
                return Ok(LocalLyricsLookup::Found(track_lyrics_dto(
                    track.id,
                    track,
                    LyricProvider::Sidecar,
                    false,
                    &lyrics,
                )));
            }
            embedded_only = true;
        }
        player_db::LyricSidecarLookup::Present { text: None, oversized: true } => {
            embedded_only = true;
            sidecar_oversized = true;
        }
        // Remembered without its text (it could not be read at the sync): look for it the old way.
        player_db::LyricSidecarLookup::Present { text: None, oversized: false } => {}
        player_db::LyricSidecarLookup::Absent => embedded_only = true,
        player_db::LyricSidecarLookup::NotScanned => {}
    }
    if embedded_known && embedded_only {
        return Ok(if sidecar_oversized {
            LocalLyricsLookup::Oversized
        } else {
            LocalLyricsLookup::Missing
        });
    }

    let fingerprint = database
        .remote_file_fingerprint(track_id)
        .map_err(|error| error.to_string())?;
    let cached = match fingerprint {
        Some((size, modified)) => database
            .get_remote_lyric_cache(track_id, size, modified)
            .map_err(|error| error.to_string())?
            .and_then(|json| serde_json::from_str::<RawLocalLyrics>(&json).ok())
            // A result from before the sync knew the sidecars may name a `.lrc` that is gone.
            .filter(|raw| {
                !(embedded_only
                    && matches!(raw, RawLocalLyrics::Found { provider: LyricProvider::Sidecar, .. }))
            }),
        None => None,
    };
    let from_database = cached.is_some();
    let raw = match cached {
        Some(raw) => raw,
        None => {
            let raw = raw_local_lyrics(app, database, track_id, embedded_only).await?;
            if let (Some((size, modified)), Ok(json)) = (fingerprint, serde_json::to_string(&raw)) {
                if remote_lookup_is_definitive(database, track_id, &raw).await {
                    let _ = database.save_remote_lyric_cache(track_id, size, modified, &json);
                }
            }
            raw
        }
    };
    if fingerprint.is_some() {
        eprintln!(
            "[remote] lyrics lookup took {} ms (from the database: {})",
            started.elapsed().as_millis(),
            from_database
        );
    }
    Ok(match raw {
        RawLocalLyrics::Found { provider, lyrics } => {
            LocalLyricsLookup::Found(track_lyrics_dto(track.id, track, provider, false, &lyrics))
        }
        RawLocalLyrics::Missing if sidecar_oversized => LocalLyricsLookup::Oversized,
        RawLocalLyrics::Missing => LocalLyricsLookup::Missing,
        RawLocalLyrics::Oversized => LocalLyricsLookup::Oversized,
    })
}

/// Whether a lookup result may be remembered. A found result always may. "Missing" is the answer of a server that could not
/// be reached just as much as of a file without lyrics, so it is remembered only when the audio file itself can be opened
/// right now (the server is there, and the lyrics really are not).
async fn remote_lookup_is_definitive(database: &Database, track_id: TrackId, raw: &RawLocalLyrics) -> bool {
    if !matches!(raw, RawLocalLyrics::Missing) {
        return true;
    }
    let Some((source_id, path)) = database.track_locators(track_id).ok().and_then(|locators| {
        locators.into_iter().find_map(|locator| match locator {
            player_core::MediaLocator::Remote { source_id, path } => Some((source_id, path)),
            _ => None,
        })
    }) else {
        return false;
    };
    tauri::async_runtime::spawn_blocking(move || {
        player_core::remote_content().is_some_and(|content| content.open(source_id, &path).is_ok())
    })
    .await
    .unwrap_or(false)
}

async fn local_source_candidates(
    database: &Database,
    track_id: TrackId,
    track: &TrackSummary,
) -> Result<Vec<LyricsCandidateDto>, String> {
    #[cfg(target_os = "windows")]
    {
        let locators = database
            .track_locators(track_id)
            .map_err(|error| error.to_string())?;
        let found = tauri::async_runtime::spawn_blocking(move || find_all_lyrics(&locators))
            .await
            .map_err(|error| format!("讀取本機歌詞工作失敗：{error}"))?;
        Ok(found
            .iter()
            .filter_map(|item| local_source_candidate_dto(track, item.provider, &item.lyrics))
            .collect())
    }
    #[cfg(target_os = "android")]
    {
        let locators = database
            .track_locators(track_id)
            .map_err(|error| error.to_string())?;
        let (found, _) = tauri::async_runtime::spawn_blocking(move || {
            android_find_all_lyrics(None, &locators, false, true)
        })
        .await
        .map_err(|error| format!("讀取本機歌詞工作失敗：{error}"))?;
        Ok(found
            .iter()
            .filter_map(|item| local_source_candidate_dto(track, item.provider, &item.lyrics))
            .collect())
    }
    #[cfg(not(any(target_os = "windows", target_os = "android")))]
    {
        let _ = (database, track_id, track);
        Ok(Vec::new())
    }
}

fn local_source_candidate_dto(
    track: &TrackSummary,
    provider: LyricProvider,
    lyrics: &ParsedLyrics,
) -> Option<LyricsCandidateDto> {
    let (id, dto_provider, reason) = match provider {
        LyricProvider::Sidecar => (
            LOCAL_SIDECAR_CANDIDATE_ID,
            LyricsProviderDto::Local,
            "同資料夾同名 LRC",
        ),
        LyricProvider::Embedded => (
            LOCAL_EMBEDDED_CANDIDATE_ID,
            LyricsProviderDto::Embedded,
            "音訊檔內嵌歌詞",
        ),
        LyricProvider::NetEase | LyricProvider::Qq => return None,
    };
    Some(LyricsCandidateDto {
        id: id.to_owned(),
        provider: dto_provider,
        title: track.title.clone().unwrap_or_default(),
        artist: track.artist.clone().unwrap_or_default(),
        album: track.album.clone(),
        duration_ms: track.duration_ms,
        score: 1.0,
        confidence: LyricsConfidenceDto::High,
        reasons: vec![reason.to_owned()],
        preview_lines: lyrics
            .lines
            .iter()
            .filter(|line| !line.text.trim().is_empty())
            .take(3)
            .map(|line| line.text.clone())
            .collect(),
        has_synced_lyrics: lyrics.synced,
        has_word_synced_lyrics: has_word_synced_lyrics(lyrics),
        has_translation: has_translation(lyrics),
        has_romanization: has_romanization(lyrics),
    })
}

/// The manual row for a pick of a local source: provider and candidate id, no text.
fn local_pick_reference(
    track_id: TrackId,
    provider: LyricProvider,
    candidate_id: &str,
    updated_at_utc_ms: i64,
) -> TrackLyrics {
    TrackLyrics {
        track_id,
        provider,
        candidate_id: Some(candidate_id.to_owned()),
        lyrics: empty_parsed_lyrics(),
        manually_selected: true,
        updated_at_utc_ms,
    }
}

/// Saving the song details with changed lyrics links the track to its embedded lyrics (user decision
/// 2026-10-22): the embedded lyrics then win over a same-name `.lrc` and earlier picks.
pub(crate) fn link_embedded_lyrics(
    database: &Database,
    track_id: TrackId,
    updated_at_utc_ms: i64,
) -> Result<(), String> {
    database
        .save_manual_track_lyrics(&local_pick_reference(
            track_id,
            LyricProvider::Embedded,
            LOCAL_EMBEDDED_CANDIDATE_ID,
            updated_at_utc_ms,
        ))
        .map_err(|error| error.to_string())
}

/// What a manual row shows: an online pick is its stored text; a local pick is the source's current
/// lyrics, or `None` when that source is empty now.
fn manual_lyrics_dto(
    database: &Database,
    track: &TrackSummary,
    selected: &TrackLyrics,
) -> Result<Option<TrackLyricsDto>, String> {
    match selected
        .candidate_id
        .as_deref()
        .and_then(local_candidate_provider)
    {
        Some(provider) => Ok(live_local_pick(database, track.id, provider)?
            .map(|lyrics| track_lyrics_dto(track.id, track, provider, true, &lyrics))),
        None => Ok(Some(persisted_lyrics_dto(track, selected.clone()))),
    }
}

/// A local lyric text (embedded or sidecar) read by its content (ADR 0041): LRC, yrc, SRT, ASS or
/// plain text. A broken subtitle is `None`, like no lyrics, so the next source is tried.
pub(crate) fn parse_stored_lyrics(raw: &str) -> Option<ParsedLyrics> {
    parse_any_lyrics(raw).ok().and_then(LyricsContent::usable)
}

/// The current lyrics of a local source: the embedded lyrics and the `.lrc` the last sync stored
/// for the track, or the file itself when the sync has nothing for it.
fn live_local_pick(
    database: &Database,
    track_id: TrackId,
    provider: LyricProvider,
) -> Result<Option<ParsedLyrics>, String> {
    match provider {
        LyricProvider::Embedded => {
            if let Some(stored) = database
                .track_embedded_lyrics(track_id)
                .map_err(|error| error.to_string())?
            {
                return Ok(stored.as_deref().and_then(parse_stored_lyrics));
            }
        }
        LyricProvider::Sidecar => match database
            .track_lyric_sidecar(track_id)
            .map_err(|error| error.to_string())?
        {
            player_db::LyricSidecarLookup::Present { text: Some(text), .. } => {
                return Ok(parse_stored_lyrics(&text));
            }
            player_db::LyricSidecarLookup::Present { text: None, oversized: true }
            | player_db::LyricSidecarLookup::Absent => return Ok(None),
            player_db::LyricSidecarLookup::Present { text: None, oversized: false }
            | player_db::LyricSidecarLookup::NotScanned => {}
        },
        LyricProvider::NetEase | LyricProvider::Qq => return Ok(None),
    }
    Ok(read_local_lyric_source(database, track_id, provider)
        .ok()
        .filter(|lyrics| !lyrics.lines.is_empty()))
}

fn select_local_lyric_candidate(
    database: &Database,
    track_id: TrackId,
    provider: LyricProvider,
    candidate_id: &str,
    selected_at_utc_ms: i64,
) -> Result<TrackLyricsDto, String> {
    let track = database
        .get_track_summary(track_id)
        .map_err(|error| error.to_string())?
        .ok_or_else(|| "找不到這首曲目。".to_owned())?;
    let lyrics = match live_local_pick(database, track_id, provider)? {
        Some(lyrics) => lyrics,
        None => read_local_lyric_source(database, track_id, provider)?,
    };
    // Only the reference (provider + candidate id) is kept; the text is read live on every load.
    database
        .save_manual_track_lyrics(&local_pick_reference(
            track_id,
            provider,
            candidate_id,
            selected_at_utc_ms,
        ))
        .map_err(|error| error.to_string())?;
    Ok(track_lyrics_dto(track.id, &track, provider, true, &lyrics))
}

#[cfg(target_os = "windows")]
fn read_local_lyric_source(
    database: &Database,
    track_id: TrackId,
    provider: LyricProvider,
) -> Result<ParsedLyrics, String> {
    let locators = database
        .track_locators(track_id)
        .map_err(|error| error.to_string())?;
    find_all_lyrics(&locators)
        .into_iter()
        .find(|item| item.provider == provider)
        .map(|item| item.lyrics)
        .ok_or_else(|| "找不到這份本機歌詞。".to_owned())
}

#[cfg(target_os = "android")]
fn read_local_lyric_source(
    database: &Database,
    track_id: TrackId,
    provider: LyricProvider,
) -> Result<ParsedLyrics, String> {
    let locators = database
        .track_locators(track_id)
        .map_err(|error| error.to_string())?;
    android_find_all_lyrics(None, &locators, false, true)
        .0
        .into_iter()
        .find(|item| item.provider == provider)
        .map(|item| item.lyrics)
        .ok_or_else(|| "找不到這份本機歌詞。".to_owned())
}

#[cfg(not(any(target_os = "windows", target_os = "android")))]
fn read_local_lyric_source(
    database: &Database,
    track_id: TrackId,
    provider: LyricProvider,
) -> Result<ParsedLyrics, String> {
    let _ = (database, track_id, provider);
    Err("這個平台還不能從候選清單套用本機歌詞。".to_owned())
}

#[cfg(not(any(target_os = "windows", target_os = "android")))]
async fn raw_local_lyrics(
    _app: Option<&AppHandle>,
    _database: &Database,
    _track_id: TrackId,
    _embedded_only: bool,
) -> Result<RawLocalLyrics, String> {
    Ok(RawLocalLyrics::Missing)
}

#[cfg(target_os = "android")]
async fn raw_local_lyrics(
    app: Option<&AppHandle>,
    database: &Database,
    track_id: TrackId,
    embedded_only: bool,
) -> Result<RawLocalLyrics, String> {
    let Some(app) = app else {
        return Ok(RawLocalLyrics::Missing);
    };
    let locators = database
        .track_locators(track_id)
        .map_err(|error| error.to_string())?;
    if locators.is_empty() {
        return Ok(RawLocalLyrics::Missing);
    }
    let app = app.clone();
    let (found, oversized) = tauri::async_runtime::spawn_blocking(move || {
        android_find_all_lyrics(Some(&app), &locators, true, !embedded_only)
    })
    .await
    .map_err(|error| format!("讀取 Android 本機歌詞工作失敗：{error}"))?;
    // Same precedence as Windows `find_lyrics`: embedded lyrics, then the sidecar.
    Ok(match found.into_iter().next() {
        Some(item) => RawLocalLyrics::Found {
            provider: item.provider,
            lyrics: item.lyrics,
        },
        None if oversized => RawLocalLyrics::Oversized,
        None => RawLocalLyrics::Missing,
    })
}

#[cfg(target_os = "android")]
struct AndroidFoundLyrics {
    provider: LyricProvider,
    lyrics: ParsedLyrics,
}

/// Android counterpart of Windows `find_all_lyrics`, in the same order (ADR 0041): the lyrics
/// embedded in the track's tags first, then the sidecar beside a real-path or remote track
/// (`.lrc`, `.ass`, `.srt`). `first_only` stops after the first hit (playback lookup). A legacy
/// content:// locator is read for embedded tags only when `app` is given.
#[cfg(target_os = "android")]
fn android_find_all_lyrics(
    app: Option<&AppHandle>,
    locators: &[player_core::MediaLocator],
    first_only: bool,
    with_sidecar: bool,
) -> (Vec<AndroidFoundLyrics>, bool) {
    use lofty::{config::ParseOptions, prelude::TaggedFileExt, probe::Probe, tag::ItemKey};
    use player_core::MAX_LYRIC_PAYLOAD_BYTES;
    use tauri_plugin_media_index::MediaIndexExt;

    let audio_path = locators.iter().find_map(|locator| match locator {
        player_core::MediaLocator::FileSystem(path) if path.is_file() => Some(path.clone()),
        _ => None,
    });
    let audio_uri = locators.iter().find_map(|locator| match locator {
        player_core::MediaLocator::ContentUri(uri) => Some(uri.clone()),
        player_core::MediaLocator::FileSystem(_) | player_core::MediaLocator::Remote { .. } => None,
    });
    let mut found = Vec::new();
    let mut oversized = false;
    let first = |texts: &[String]| texts.iter().find_map(|text| parse_stored_lyrics(text));

    // A remote track (ADR 0026): its embedded lyrics, then the sidecar beside it, both read through the remote reader.
    if let Some((source_id, remote_path)) = locators.iter().find_map(|locator| match locator {
        player_core::MediaLocator::Remote { source_id, path } => Some((*source_id, path.clone())),
        _ => None,
    }) {
        let (sidecars, embedded, remote_oversized) =
            crate::remote_media::remote_lyric_texts(source_id, &remote_path, with_sidecar);
        oversized |= remote_oversized;
        if let Some(lyrics) = first(&embedded) {
            found.push(AndroidFoundLyrics { provider: LyricProvider::Embedded, lyrics });
            if first_only {
                return (found, oversized);
            }
        }
        if let Some(lyrics) = first(&sidecars) {
            found.push(AndroidFoundLyrics { provider: LyricProvider::Sidecar, lyrics });
        }
        return (found, oversized);
    }

    let tags_path = match (&audio_path, audio_uri, app) {
        (Some(path), _, _) => Some((path.clone(), None)),
        (None, Some(uri), Some(app)) => app
            .media_index()
            .cache_content_uri(&uri, 512 * 1024 * 1024)
            .ok()
            .map(|lease| (lease.path().to_path_buf(), Some(lease))),
        _ => None,
    };
    if let Some((path, _lease)) = tags_path {
        let tagged = Probe::open(&path)
            .ok()
            .map(|probe| probe.options(ParseOptions::new().read_properties(false)))
            .and_then(|probe| probe.guess_file_type().ok())
            .and_then(|probe| probe.read().ok());
        let mut texts = Vec::new();
        if let Some(tagged) = &tagged {
            for key in [ItemKey::Lyrics, ItemKey::UnsyncLyrics] {
                for raw in tagged.tags().iter().filter_map(|tag| tag.get_string(key)) {
                    if raw.len() > MAX_LYRIC_PAYLOAD_BYTES {
                        oversized = true;
                    } else {
                        texts.push(raw.to_owned());
                    }
                }
            }
        }
        if let Some(lyrics) = first(&texts) {
            found.push(AndroidFoundLyrics { provider: LyricProvider::Embedded, lyrics });
            if first_only {
                return (found, oversized);
            }
        }
    }

    if let Some(path) = audio_path.as_deref().filter(|_| with_sidecar) {
        let mut texts = Vec::new();
        for extension in player_core::LYRIC_SIDECAR_EXTENSIONS {
            let Some(sidecar) =
                tauri_plugin_media_index::fs_scan::sibling_with_extension(path, extension)
            else {
                continue;
            };
            match std::fs::read(sidecar) {
                Ok(bytes) if bytes.len() > MAX_LYRIC_PAYLOAD_BYTES => oversized = true,
                Ok(bytes) => texts.push(
                    String::from_utf8_lossy(bytes.strip_prefix(b"\xEF\xBB\xBF").unwrap_or(&bytes))
                        .into_owned(),
                ),
                Err(_) => {}
            }
        }
        if let Some(lyrics) = first(&texts) {
            found.push(AndroidFoundLyrics { provider: LyricProvider::Sidecar, lyrics });
        }
    }
    (found, oversized)
}

fn persisted_lyrics_dto(track: &TrackSummary, mut lyrics: TrackLyrics) -> TrackLyricsDto {
    // Provider LRC cached before same-timestamp folding may still hold one row per duplicate
    // timestamp; fold it on read so old cache entries follow the primary/translation/romanization
    // rule without rewriting SQLite.
    if lyrics.lyrics.format == LyricFormat::Lrc {
        lyrics.lyrics.lines = merge_same_timestamp_lines(std::mem::take(&mut lyrics.lyrics.lines));
    }
    // Rows stored before word timing kept the yrc only as the raw payload.
    restore_words_from_raw_karaoke(&mut lyrics.lyrics);
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
        provider: match provider {
            LyricProvider::NetEase => LyricsProviderDto::NetEase,
            LyricProvider::Qq => LyricsProviderDto::QqMusic,
            LyricProvider::Sidecar => LyricsProviderDto::Local,
            LyricProvider::Embedded => LyricsProviderDto::Embedded,
        },
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
    let words_traditional = traditional_word_texts(&line.words);
    LyricLineDto {
        start_ms: line.start_ms,
        text: line.text.clone(),
        translation: line.translation.clone(),
        romanization: line.romanization.clone(),
        // With words, the line's Traditional text is its words' joined so both stay aligned.
        text_traditional: match &words_traditional {
            Some(words) => Some(words.concat()),
            None if line.words.is_empty() => traditional_if_changed(&line.text),
            None => None,
        },
        translation_traditional: line.translation.as_deref().and_then(traditional_if_changed),
        end_ms: line.end_ms,
        words: line
            .words
            .iter()
            .map(|word| LyricWordDto {
                s: word.start_ms,
                e: word.end_ms,
                t: word.text.clone(),
            })
            .collect(),
        words_traditional,
    }
}

fn traditional_if_changed(text: &str) -> Option<String> {
    match player_zh::to_traditional(text) {
        std::borrow::Cow::Owned(converted) => Some(converted),
        std::borrow::Cow::Borrowed(_) => None,
    }
}

#[cfg(test)]
mod tests {
    use super::{
        candidate_dto, fetch_provider_candidates, line_dto, raw_to_lyric_candidate, LyricsService,
        LyricsSourceDto, LYRIC_BATCH_SIZE,
    };
    use crate::lyrics_provider::{
        HttpMethod, HttpRequest, HttpResponse, HttpTransport, LyricsProviderClient, ProviderError,
        RawKaraokeData, RawLyricCandidate,
    };
    use player_core::{
        parse_lrc, FileFingerprint, LibraryRepository, LibraryRoot, LyricCandidate, LyricFormat,
        LyricLine, LyricProvider, LyricsTrackMetadata, MediaLocator, MediaSourceKind,
        MediaTrackRecord, SourceId, SourceScanState, TrackId, TrackIdentity, TrackMetadata,
        TrackSummary,
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
                let script_variant = if request.url.contains("%E5%A4%B4")
                    || request
                        .body
                        .as_deref()
                        .is_some_and(|body| body.contains("头发"))
                {
                    "simplified"
                } else if request.url.contains("%E9%A0%AD")
                    || request
                        .body
                        .as_deref()
                        .is_some_and(|body| body.contains("頭髮"))
                {
                    "traditional"
                } else {
                    "default"
                };
                let response = if request.url.contains("/api/search/get/web")
                    || request.url.contains("/api/cloudsearch/pc")
                {
                    netease_search_fixture_for(script_variant)
                } else if request.method == HttpMethod::Post {
                    qq_search_fixture_for(script_variant)
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

    fn netease_search_fixture_for(script_variant: &str) -> serde_json::Value {
        serde_json::json!({
            "result": {
                "songs": (0..25).map(|index| serde_json::json!({
                    "id": if script_variant == "default" {
                        index.to_string()
                    } else {
                        format!("{script_variant}-{index}")
                    },
                    "name": "Song",
                    "artists": [{"name": "Artist"}],
                    "album": {"name": "Album"},
                    "duration": 180_000
                })).collect::<Vec<_>>()
            }
        })
    }

    fn qq_search_fixture_for(script_variant: &str) -> serde_json::Value {
        serde_json::json!({
            "req_1": {
                "data": {
                    "body": {
                        "song": {
                            "list": (0..25).map(|index| serde_json::json!({
                                "songmid": if script_variant == "default" {
                                    format!("qq-{index}")
                                } else {
                                    format!("qq-{script_variant}-{index}")
                                },
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
            release_date: None,
            year: None,
            created_at_utc_ms: None,
            bit_depth: None,
            played_ms: 0,
            file_name: None,
            source_name: None,
            modified_at_utc_ms: None,
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
                created_at_utc_ms: None,
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
            has_more: false,
        };
        let value = serde_json::to_value(result).expect("serialize result DTO");
        assert_eq!(
            keys(&value),
            ["candidates", "error", "hasMore", "lyrics", "status"]
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
                "provider",
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
                "hasWordSyncedLyrics",
                "hasTranslation",
                "hasRomanization",
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
    fn a_matching_yrc_replaces_the_lrc_and_takes_nearby_translations_and_keeps_its_raw_payload() {
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
            yrc_translation: None,
            yrc_romanization: None,
            raw_karaoke: None,
        };
        let candidate = raw_to_lyric_candidate(raw).expect("convert provider response");
        assert_eq!(candidate.lyrics.format, player_core::LyricFormat::Yrc);
        assert!(candidate.lyrics.synced);
        assert_eq!(candidate.lyrics.lines[0].words.len(), 1);
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
            yrc_translation: None,
            yrc_romanization: None,
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
            yrc_translation: None,
            yrc_romanization: None,
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
        // No match-explanation line (title / artist / length / album) is listed any more.
        assert_eq!(qrc_candidate.reasons.len(), 1);
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
        assert!(candidates.len() <= LYRIC_BATCH_SIZE);
        assert!(lyric_fetches <= LYRIC_BATCH_SIZE);
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
            .search_track(
                &database,
                track.id,
                &track,
                CancellationToken::new(),
                true,
                None,
            )
            .await
            .expect("complete fixture search");

        assert!(matches!(
            result.status,
            super::LyricsResultStatusDto::Candidates
        ));
        assert!(result.lyrics.is_none());
        assert!(!result.candidates.is_empty());
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
            .search_track(
                &database,
                track.id,
                &track,
                CancellationToken::new(),
                true,
                None,
            )
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
    async fn service_never_auto_applies_when_duration_cannot_be_verified() {
        let (database, _directory, mut track) = seeded_test_database("Song", "Artisx", "Album");
        // Title/album match and the artist is one character off, but with no local duration
        // the duration gate cannot pass, so nothing is auto-applied (candidates are returned).
        track.duration_ms = None;

        let service = LyricsService::new(LyricsProviderClient::new(FixtureTransport::default()));
        let result = service
            .search_track(
                &database,
                track.id,
                &track,
                CancellationToken::new(),
                true,
                None,
            )
            .await
            .expect("complete fixture search");

        assert!(result.lyrics.is_none());
        assert!(database
            .get_track_lyrics(track.id)
            .expect("read selected lyric cache")
            .is_none());
    }

    #[tokio::test]
    async fn manual_search_skips_auto_apply_and_returns_candidates() {
        let (database, _directory, track) = seeded_test_database("Song", "Artist", "Album");
        let service = LyricsService::new(LyricsProviderClient::new(FixtureTransport::default()));
        let result = service
            .search_track(
                &database,
                track.id,
                &track,
                CancellationToken::new(),
                false,
                None,
            )
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

    fn qq_queries(transport: &FixtureTransport) -> Vec<String> {
        let requests = transport
            .requests
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        requests
            .iter()
            .filter_map(|request| request.body.as_deref())
            .filter_map(|body| serde_json::from_str::<serde_json::Value>(body).ok())
            .filter_map(|body| {
                body.get("req_1")?
                    .get("param")?
                    .get("query")?
                    .as_str()
                    .map(str::to_owned)
            })
            .collect()
    }

    #[tokio::test]
    async fn provider_search_sends_only_the_simplified_spelling() {
        let (database, _directory, track) = seeded_test_database("頭髮", "Artist", "Album");

        let transport = FixtureTransport::default();
        let service = LyricsService::new(LyricsProviderClient::new(transport.clone()));
        service
            .search_track(
                &database,
                track.id,
                &track,
                CancellationToken::new(),
                false,
                Some("頭髮".to_owned()),
            )
            .await
            .expect("manual query search");
        let manual = qq_queries(&transport);
        assert!(!manual.is_empty());
        assert!(manual.iter().all(|query| query == "头发"), "{manual:?}");

        let transport = FixtureTransport::default();
        let service = LyricsService::new(LyricsProviderClient::new(transport.clone()));
        service
            .search_track(
                &database,
                track.id,
                &track,
                CancellationToken::new(),
                false,
                None,
            )
            .await
            .expect("automatic search");
        let automatic = qq_queries(&transport);
        assert!(!automatic.is_empty());
        assert!(
            automatic
                .iter()
                .all(|query| query.contains("头发") && !query.contains("頭髮")),
            "{automatic:?}"
        );
    }

    #[test]
    fn lyric_lines_carry_their_traditional_spelling_only_when_it_differs() {
        let simplified = line_dto(&LyricLine {
            start_ms: Some(0),
            text: "这只有我知道".to_owned(),
            translation: Some("君の名は".to_owned()),
            romanization: None,
            ..LyricLine::default()
        });
        assert_eq!(simplified.text_traditional.as_deref(), Some("這只有我知道"));
        assert_eq!(simplified.translation_traditional, None, "kana lines stay");
        let json = serde_json::to_value(&simplified).expect("serialize");
        assert_eq!(json["textTraditional"], "這只有我知道");
        assert!(json.get("translationTraditional").is_none());

        let traditional = line_dto(&LyricLine {
            start_ms: None,
            text: "頭髮".to_owned(),
            translation: None,
            romanization: None,
            ..LyricLine::default()
        });
        assert_eq!(traditional.text_traditional, None);
    }

    #[tokio::test]
    async fn temporary_catalog_keeps_manual_lyrics_and_candidates_in_memory_only() {
        let (persistent, _persistent_dir, persistent_track) =
            seeded_test_database("Persistent Song", "Artist", "Album");
        let (temporary, _temporary_dir, temporary_track) =
            seeded_test_database("Transient Song", "Artist", "Album");
        let service = LyricsService::new(LyricsProviderClient::new(FixtureTransport::default()));
        let result = service
            .search_track(
                &temporary,
                temporary_track.id,
                &temporary_track,
                CancellationToken::new(),
                false,
                None,
            )
            .await
            .expect("search candidates using the temporary SQLite database");
        assert!(!result.candidates.is_empty());
        let candidate_id = result
            .candidates
            .iter()
            .find(|candidate| candidate.id == "netease:0")
            .expect("fixture candidate exists")
            .id
            .clone();
        super::select_candidate_from_database(
            &temporary,
            temporary_track.id,
            &candidate_id,
            1_800_000_000_100,
        )
        .expect("save the selected document in the temporary catalog");
        assert!(temporary
            .get_track_lyrics(temporary_track.id)
            .unwrap()
            .is_some());
        assert!(persistent
            .get_track_lyrics(persistent_track.id)
            .unwrap()
            .is_none());
        assert!(persistent
            .get_track_summary(temporary_track.id)
            .unwrap()
            .is_none());
    }

    #[tokio::test]
    async fn searched_candidate_is_cached_and_manual_selection_persists_auxiliary_and_raw_yrc() {
        let (database, _directory, track) =
            seeded_test_database("完全不同的本機曲名", "本機演出者", "本機專輯");
        let service = LyricsService::new(LyricsProviderClient::new(FixtureTransport::default()));
        let result = service
            .search_track(
                &database,
                track.id,
                &track,
                CancellationToken::new(),
                true,
                None,
            )
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
            "[offset:250]\n[00:01.00]本機逐行歌詞\n[00:01.00]本機譯文",
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
            created_at_utc_ms: None,
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
        assert_eq!(lyrics.lines.len(), 1);
        assert_eq!(lyrics.lines[0].text, "本機逐行歌詞");
        assert_eq!(lyrics.lines[0].translation.as_deref(), Some("本機譯文"));
        assert_eq!(lyrics.lines[0].start_ms, Some(1_250));

        // An automatic provider cache entry must not hide local lyrics.
        let automatic = player_core::TrackLyrics {
            track_id,
            provider: LyricProvider::NetEase,
            candidate_id: Some("netease:auto".to_owned()),
            lyrics: parse_lrc("[00:02.00]自動快取").expect("automatic LRC"),
            manually_selected: false,
            updated_at_utc_ms: 1_800_000_000_000,
        };
        assert!(database
            .save_automatic_track_lyrics(&automatic)
            .expect("save automatic cache"));
        let result = super::load_track_result(&database, track_id)
            .await
            .expect("load with automatic cache");
        assert_eq!(
            result.lyrics.expect("local lyrics").source,
            LyricsSourceDto::Local
        );

        // A manual pick is linked to the TrackId and wins over the sidecar on the next load.
        let manual = player_core::TrackLyrics {
            track_id,
            provider: LyricProvider::Qq,
            candidate_id: Some("qqmusic:manual".to_owned()),
            lyrics: parse_lrc("[00:03.00]人工指定\n[00:03.00]人工譯文").expect("manual LRC"),
            manually_selected: true,
            updated_at_utc_ms: 1_800_000_000_001,
        };
        database
            .save_manual_track_lyrics(&manual)
            .expect("save manual selection");
        let result = super::load_track_result(&database, track_id)
            .await
            .expect("load with manual selection");
        let lyrics = result.lyrics.expect("manual lyrics returned");
        assert_eq!(lyrics.source, LyricsSourceDto::Manual);
        assert_eq!(lyrics.lines[0].text, "人工指定");
        assert_eq!(lyrics.lines[0].translation.as_deref(), Some("人工譯文"));

        // Clearing without a suppress sentinel still falls back to local lyrics.
        assert!(database
            .clear_track_lyrics(track_id)
            .expect("clear manual selection"));
        let result = super::load_track_result(&database, track_id)
            .await
            .expect("load after clear");
        assert_eq!(
            result.lyrics.expect("local lyrics after clear").source,
            LyricsSourceDto::Local
        );

        // 「移除歌詞」persists an empty manual sentinel so local/auto search stay off.
        database
            .save_manual_track_lyrics(&player_core::TrackLyrics {
                track_id,
                provider: LyricProvider::Sidecar,
                candidate_id: Some(super::CLEARED_LYRICS_CANDIDATE_ID.to_owned()),
                lyrics: super::empty_parsed_lyrics(),
                manually_selected: true,
                updated_at_utc_ms: 1_800_000_000_002,
            })
            .expect("save cleared sentinel");
        let result = super::load_track_result(&database, track_id)
            .await
            .expect("load after cleared sentinel");
        assert!(result.lyrics.is_none());
        assert!(matches!(
            result.status,
            super::LyricsResultStatusDto::Cleared
        ));

        let track = database
            .get_track_summary(track_id)
            .expect("read track")
            .expect("track summary");
        let candidates = super::local_source_candidates(&database, track_id, &track)
            .await
            .expect("list local lyric sources");
        assert_eq!(candidates.len(), 1);
        assert_eq!(candidates[0].id, super::LOCAL_SIDECAR_CANDIDATE_ID);
        assert_eq!(candidates[0].provider, super::LyricsProviderDto::Local);
        assert!(candidates[0]
            .preview_lines
            .iter()
            .any(|line| line.contains("本機逐行歌詞")));
        let selected = super::select_candidate_from_database(
            &database,
            track_id,
            super::LOCAL_SIDECAR_CANDIDATE_ID,
            1_800_000_000_500,
        )
        .expect("select the sidecar candidate");
        assert_eq!(selected.source, LyricsSourceDto::Manual);
        assert_eq!(selected.lines[0].text, "本機逐行歌詞");
        let result = super::load_track_result(&database, track_id)
            .await
            .expect("load after selecting the local sidecar");
        assert_eq!(
            result.lyrics.expect("manual sidecar link").source,
            LyricsSourceDto::Manual
        );
    }

    /// A track whose embedded lyrics (from the sync) can be rewritten by applying the scan again.
    struct EmbeddedFixture {
        database: Database,
        directory: TestDirectory,
        root: LibraryRoot,
        record: MediaTrackRecord,
        track_id: TrackId,
    }

    impl EmbeddedFixture {
        fn new(embedded: Option<&str>) -> Self {
            let directory = TestDirectory::new();
            let audio_path = directory.0.join("track.mp3");
            fs::write(&audio_path, b"lyrics reference test media").expect("write media placeholder");
            let source_id = SourceId::new();
            let root = LibraryRoot {
                id: source_id,
                kind: MediaSourceKind::WindowsFilesystem,
                display_name: "lyrics reference source".to_owned(),
                locator: MediaLocator::FileSystem(directory.0.clone()),
                enabled: true,
            };
            let database = Database::open_in_memory().expect("open isolated test database");
            database.save_library_root(&root).expect("save test source");
            let record = MediaTrackRecord {
                identity: TrackIdentity {
                    source_id,
                    source_item_id: "lyrics-reference-track".to_owned(),
                    locator_key: None,
                },
                locator: MediaLocator::FileSystem(audio_path.clone()),
                fingerprint: FileFingerprint {
                    size_bytes: fs::metadata(&audio_path).expect("media metadata").len(),
                    modified_at_utc_ms: None,
                    created_at_utc_ms: None,
                },
                metadata: Some(TrackMetadata {
                    title: Some("參照曲目".to_owned()),
                    artist: Some("參照演出者".to_owned()),
                    duration_ms: Some(180_000),
                    ..TrackMetadata::default()
                }),
            };
            let mut fixture = Self { database, directory, root, record, track_id: TrackId::new() };
            fixture.rescan(embedded);
            fixture.track_id = fixture
                .database
                .resolve_track_id_for_locator(&MediaLocator::FileSystem(audio_path))
                .expect("resolve indexed track")
                .expect("track mapping exists");
            fixture
        }

        /// What the next sync would store after the file's embedded lyrics changed.
        fn rescan(&mut self, embedded: Option<&str>) {
            if let Some(metadata) = self.record.metadata.as_mut() {
                metadata.embedded_lyrics = embedded.map(str::to_owned);
            }
            self.database
                .apply_source_scan(
                    &self.root,
                    &SourceScanState::Complete,
                    std::slice::from_ref(&self.record),
                    std::slice::from_ref(&self.record),
                    &[],
                    1_800_000_000_000,
                )
                .expect("apply scan");
        }

        async fn shown(&self) -> Option<(LyricsSourceDto, String)> {
            super::load_track_result(&self.database, self.track_id)
                .await
                .expect("load lyrics")
                .lyrics
                .map(|lyrics| (lyrics.source, lyrics.lines[0].text.clone()))
        }
    }

    #[tokio::test]
    async fn picked_embedded_lyrics_are_a_reference_that_follows_the_file() {
        let mut fixture = EmbeddedFixture::new(Some("[00:01.00]舊的內嵌"));
        let selected = super::select_candidate_from_database(
            &fixture.database,
            fixture.track_id,
            super::LOCAL_EMBEDDED_CANDIDATE_ID,
            1_800_000_000_100,
        )
        .expect("pick the embedded lyrics");
        assert_eq!(selected.lines[0].text, "舊的內嵌");
        let row = fixture
            .database
            .get_track_lyrics(fixture.track_id)
            .expect("read manual row")
            .expect("manual row saved");
        assert!(row.manually_selected);
        assert_eq!(row.provider, LyricProvider::Embedded);
        assert_eq!(row.candidate_id.as_deref(), Some(super::LOCAL_EMBEDDED_CANDIDATE_ID));
        assert!(row.lyrics.lines.is_empty() && row.lyrics.raw_text.is_empty(), "no text snapshot");
        assert_eq!(fixture.shown().await, Some((LyricsSourceDto::Manual, "舊的內嵌".to_owned())));

        // The tags were edited: the next load shows the new embedded lyrics, no re-pick needed.
        fixture.rescan(Some("[00:01.00]新的內嵌"));
        assert_eq!(fixture.shown().await, Some((LyricsSourceDto::Manual, "新的內嵌".to_owned())));

        // The embedded lyrics were removed: the normal order applies (no .lrc, no cache here).
        fixture.rescan(None);
        assert_eq!(fixture.shown().await, None);
    }

    #[tokio::test]
    async fn legacy_local_pick_rows_ignore_their_stored_snapshot() {
        let fixture = EmbeddedFixture::new(Some("[00:01.00]檔案裡的內嵌"));
        fixture
            .database
            .save_manual_track_lyrics(&player_core::TrackLyrics {
                track_id: fixture.track_id,
                provider: LyricProvider::Embedded,
                candidate_id: Some(super::LOCAL_EMBEDDED_CANDIDATE_ID.to_owned()),
                lyrics: parse_lrc("[00:01.00]舊快照").expect("legacy snapshot"),
                manually_selected: true,
                updated_at_utc_ms: 1_800_000_000_000,
            })
            .expect("save legacy row");
        assert_eq!(
            fixture.shown().await,
            Some((LyricsSourceDto::Manual, "檔案裡的內嵌".to_owned()))
        );
        // An online pick still keeps its stored text.
        fixture
            .database
            .save_manual_track_lyrics(&player_core::TrackLyrics {
                track_id: fixture.track_id,
                provider: LyricProvider::NetEase,
                candidate_id: Some("netease:42".to_owned()),
                lyrics: parse_lrc("[00:01.00]線上選擇").expect("online LRC"),
                manually_selected: true,
                updated_at_utc_ms: 1_800_000_000_001,
            })
            .expect("save online pick");
        assert_eq!(
            fixture.shown().await,
            Some((LyricsSourceDto::Manual, "線上選擇".to_owned()))
        );
    }

    #[tokio::test]
    async fn embedded_subtitles_are_parsed_and_broken_ones_fall_through_to_the_sidecar() {
        let fixture = EmbeddedFixture::new(Some("1\n00:00:01,000 --> 00:00:02,000\n內嵌字幕\n"));
        assert_eq!(
            fixture.shown().await,
            Some((LyricsSourceDto::Embedded, "內嵌字幕".to_owned()))
        );
        let broken = EmbeddedFixture::new(Some("[Events]\nDialogue: broken"));
        fs::write(broken.directory.0.join("track.lrc"), "[00:01.00]旁邊的 LRC").expect("write sidecar");
        assert_eq!(
            broken.shown().await,
            Some((LyricsSourceDto::Local, "旁邊的 LRC".to_owned())),
            "an unusable embedded subtitle counts as no embedded lyrics"
        );
    }

    #[tokio::test]
    async fn saving_details_links_embedded_lyrics_over_a_same_name_lrc() {
        let fixture = EmbeddedFixture::new(Some("[00:01.00]寫入的內嵌"));
        fs::write(fixture.directory.0.join("track.lrc"), "[00:01.00]旁邊的 LRC").expect("write sidecar");
        assert_eq!(
            fixture.shown().await,
            Some((LyricsSourceDto::Embedded, "寫入的內嵌".to_owned())),
            "normal order (ADR 0041): embedded lyrics win over the .lrc"
        );
        super::link_embedded_lyrics(&fixture.database, fixture.track_id, 1_800_000_000_200)
            .expect("link embedded lyrics");
        assert_eq!(
            fixture.shown().await,
            Some((LyricsSourceDto::Manual, "寫入的內嵌".to_owned()))
        );
    }

    #[tokio::test]
    async fn refreshing_one_track_stores_its_new_tags_and_embedded_lyrics() {
        let fixture = EmbeddedFixture::new(Some("[00:01.00]舊的內嵌"));
        let mut metadata = fixture.record.metadata.clone().expect("fixture metadata");
        metadata.title = Some("改過的曲名".to_owned());
        metadata.embedded_lyrics = Some("[00:01.00]剛寫入的內嵌".to_owned());
        fixture
            .database
            .refresh_track_metadata(fixture.track_id, &metadata, &fixture.record.locator)
            .expect("refresh one track");
        let summary = fixture
            .database
            .get_track_summary(fixture.track_id)
            .expect("read track")
            .expect("track exists");
        assert_eq!(summary.title.as_deref(), Some("改過的曲名"));
        assert_eq!(
            fixture.database.track_embedded_lyrics(fixture.track_id).expect("embedded lyrics"),
            Some(Some("[00:01.00]剛寫入的內嵌".to_owned()))
        );
    }

    #[test]
    fn album_index_key_follows_an_edited_album_tag() {
        let fixture = EmbeddedFixture::new(None);
        assert_eq!(
            fixture.database.album_index_key_for_track(fixture.track_id).expect("album key"),
            Some(("album:unmarked".to_owned(), 1))
        );
        let mut metadata = fixture.record.metadata.clone().expect("fixture metadata");
        metadata.album = Some("新專輯".to_owned());
        fixture
            .database
            .refresh_track_metadata(fixture.track_id, &metadata, &fixture.record.locator)
            .expect("refresh one track");
        let (key, count) = fixture
            .database
            .album_index_key_for_track(fixture.track_id)
            .expect("album key")
            .expect("track in the library");
        assert!(key.starts_with("album:") && key != "album:unmarked", "{key}");
        assert_eq!(count, 1);
    }

    #[test]
    fn legacy_cached_lrc_rows_with_shared_timestamps_are_folded_on_read() {
        use player_core::{LyricLine, TrackLyrics};

        let track = track();
        let mut lyrics = parse_lrc("[00:13.64]placeholder").expect("parse LRC");
        let row = |text: &str| LyricLine {
            start_ms: Some(13_640),
            text: text.to_owned(),
            translation: None,
            romanization: None,
            ..LyricLine::default()
        };
        lyrics.lines = vec![row("二番なんて望んでない"), row("才不稀罕當第二呢")];
        let persisted = TrackLyrics {
            track_id: track.id,
            provider: LyricProvider::NetEase,
            candidate_id: Some("netease:legacy".to_owned()),
            lyrics,
            manually_selected: false,
            updated_at_utc_ms: 1_800_000_000_000,
        };
        let dto = super::persisted_lyrics_dto(&track, persisted);
        assert_eq!(dto.lines.len(), 1);
        assert_eq!(dto.lines[0].text, "二番なんて望んでない");
        assert_eq!(
            dto.lines[0].translation.as_deref(),
            Some("才不稀罕當第二呢")
        );
    }

    #[test]
    fn candidate_dto_distinguishes_word_synced_line_synced_and_plain_lyrics() {
        let metadata = LyricsTrackMetadata {
            title: Some("Title".to_owned()),
            artist: Some("Artist".to_owned()),
            album: None,
            duration_ms: None,
        };
        let make_candidate = |format, synced, raw_karaoke: Option<&str>| {
            let mut lyrics = parse_lrc("[00:01.00]line").expect("fixture LRC");
            lyrics.format = format;
            lyrics.synced = synced;
            lyrics.raw_karaoke = raw_karaoke.map(str::to_owned);
            LyricCandidate {
                candidate_id: "candidate-1".to_owned(),
                provider: LyricProvider::NetEase,
                provider_track_id: Some("provider-1".to_owned()),
                title: Some("Title".to_owned()),
                artist: Some("Artist".to_owned()),
                album: None,
                duration_ms: None,
                lyrics,
                score: 1.0,
            }
        };

        // Word timing is what the lines carry (ADR 0041), not the format name or a raw payload.
        let mut worded = make_candidate(LyricFormat::Yrc, true, None);
        worded.lyrics.lines[0].words = vec![player_core::LyricWord {
            start_ms: 1_000,
            end_ms: 1_500,
            text: "line".to_owned(),
        }];
        let yrc = candidate_dto(&metadata, &worded);
        assert!(yrc.has_synced_lyrics);
        assert!(yrc.has_word_synced_lyrics);
        assert!(!candidate_dto(&metadata, &make_candidate(LyricFormat::Yrc, true, None)).has_word_synced_lyrics);

        let qrc = candidate_dto(&metadata, &make_candidate(LyricFormat::Qrc, false, None));
        assert!(!qrc.has_synced_lyrics);
        assert!(!qrc.has_word_synced_lyrics, "encrypted QRC is never decoded");

        let yrc_preserved = candidate_dto(
            &metadata,
            &make_candidate(
                LyricFormat::Lrc,
                true,
                Some(r#"{"yrc":"[1000,500](1000,500,0)word"}"#),
            ),
        );
        assert!(!yrc_preserved.has_word_synced_lyrics);

        let qrc_preserved = candidate_dto(
            &metadata,
            &make_candidate(
                LyricFormat::Lrc,
                true,
                Some(r#"{"qrc_base64":"cXJj","translation_base64":"dHJhbnNsYXRpb24"}"#),
            ),
        );
        assert!(!qrc_preserved.has_word_synced_lyrics);
        assert!(qrc_preserved.has_translation);
        assert!(!qrc_preserved.has_romanization);

        let qrc_camel_case = candidate_dto(
            &metadata,
            &make_candidate(LyricFormat::Lrc, true, Some(r#"{"qrcBase64":"cXJj"}"#)),
        );
        assert!(!qrc_camel_case.has_word_synced_lyrics);

        let mut parsed_auxiliary = make_candidate(LyricFormat::Lrc, true, None);
        parsed_auxiliary.lyrics.lines[0].translation = Some("translated line".to_owned());
        parsed_auxiliary.lyrics.lines[0].romanization = Some("romanized line".to_owned());
        let parsed_auxiliary = candidate_dto(&metadata, &parsed_auxiliary);
        assert!(parsed_auxiliary.has_translation);
        assert!(parsed_auxiliary.has_romanization);

        let line_only = candidate_dto(&metadata, &make_candidate(LyricFormat::Lrc, true, None));
        assert!(line_only.has_synced_lyrics);
        assert!(!line_only.has_word_synced_lyrics);

        let auxiliary_only = candidate_dto(
            &metadata,
            &make_candidate(
                LyricFormat::Plain,
                false,
                Some(
                    r#"{"translation_base64":"dHJhbnNsYXRpb24","romanization_base64":"cm9tYQ=="}"#,
                ),
            ),
        );
        assert!(!auxiliary_only.has_synced_lyrics);
        assert!(!auxiliary_only.has_word_synced_lyrics);
        assert!(auxiliary_only.has_translation);
        assert!(auxiliary_only.has_romanization);
        let serialized = serde_json::to_value(auxiliary_only).expect("serialize candidate DTO");
        assert_eq!(serialized["hasWordSyncedLyrics"], false);
        assert_eq!(serialized["hasTranslation"], true);
        assert_eq!(serialized["hasRomanization"], true);
    }
    #[tokio::test]
    async fn windows_asking_about_the_same_track_share_one_automatic_search() {
        use std::sync::atomic::{AtomicUsize, Ordering as AtomicOrdering};
        let service = LyricsService::new(LyricsProviderClient::new(FixtureTransport::default()));
        let track_id = TrackId::new();
        let runs = Arc::new(AtomicUsize::new(0));
        let release = Arc::new(Notify::new());
        let search = |runs: Arc<AtomicUsize>, release: Arc<Notify>| {
            move || {
                let runs = runs.clone();
                let release = release.clone();
                async move {
                    runs.fetch_add(1, AtomicOrdering::SeqCst);
                    release.notified().await;
                    (Ok(super::LyricsTrackResultDto::empty(None)), false)
                }
            }
        };
        let first = service.shared_auto_search(track_id, search(runs.clone(), release.clone()));
        let second = service.shared_auto_search(track_id, search(runs.clone(), release.clone()));
        let released = async {
            tokio::task::yield_now().await;
            tokio::task::yield_now().await;
            release.notify_waiters();
        };
        let (first, second, _) = tokio::join!(first, second, released);
        assert!(first.is_ok() && second.is_ok());
        assert_eq!(runs.load(AtomicOrdering::SeqCst), 1, "the second window waited for the first search");
        assert!(service.auto_flights.lock().unwrap().is_empty(), "finished searches are forgotten");
    }

    #[tokio::test]
    async fn a_waiting_window_searches_again_when_the_search_it_waited_for_was_cancelled() {
        use std::sync::atomic::{AtomicUsize, Ordering as AtomicOrdering};
        let service = LyricsService::new(LyricsProviderClient::new(FixtureTransport::default()));
        let track_id = TrackId::new();
        let runs = Arc::new(AtomicUsize::new(0));
        let release = Arc::new(Notify::new());
        let make = |runs: Arc<AtomicUsize>, release: Arc<Notify>| {
            move || {
                let runs = runs.clone();
                let release = release.clone();
                async move {
                    let run = runs.fetch_add(1, AtomicOrdering::SeqCst);
                    if run == 0 {
                        release.notified().await;
                        // The first search is cancelled: it found nothing out.
                        (Ok(super::LyricsTrackResultDto::empty(None)), true)
                    } else {
                        (Ok(super::LyricsTrackResultDto::empty(Some("second".into()))), false)
                    }
                }
            }
        };
        let first = service.shared_auto_search(track_id, make(runs.clone(), release.clone()));
        let second = service.shared_auto_search(track_id, make(runs.clone(), release.clone()));
        let released = async {
            tokio::task::yield_now().await;
            tokio::task::yield_now().await;
            release.notify_waiters();
        };
        let (_first, second, _) = tokio::join!(first, second, released);
        assert_eq!(runs.load(AtomicOrdering::SeqCst), 2, "the waiting window ran its own search");
        assert_eq!(second.unwrap().error.as_deref(), Some("second"));
    }
}
