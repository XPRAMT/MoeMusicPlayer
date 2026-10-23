//! The WebDAV client (ADR 0026): `PROPFIND` to list and stat, `GET` with `Range` to read, Basic and Digest
//! authentication, TLS verified by default or pinned.
//!
//! No credential is sent before the server asks for one (a plain `http://` server would see it in the clear anyway, but
//! a Digest server must not be handed the password with Basic first). The first request therefore costs one extra round
//! trip; the scheme the server chose is remembered and used from then on, with a rising nonce counter for Digest.

use std::io::Read;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use md5::Md5;
use percent_encoding::{percent_decode_str, utf8_percent_encode, AsciiSet, NON_ALPHANUMERIC};
use reqwest::blocking::{Client, RequestBuilder, Response};
use reqwest::header::{HeaderValue, AUTHORIZATION, CONTENT_RANGE, RANGE, WWW_AUTHENTICATE};
use reqwest::{Method, StatusCode};
use sha2::{Digest, Sha256};
use zeroize::Zeroizing;

use crate::account::{RemoteAccount, TlsMode};
use crate::fs::{replacement_temp_path, FileStamp, RemoteEntry, RemoteError, RemoteFile, RemoteFs};

/// Everything but unreserved characters and `/` is percent-encoded in a path.
const PATH_SET: &AsciiSet = &NON_ALPHANUMERIC
    .remove(b'-')
    .remove(b'.')
    .remove(b'_')
    .remove(b'~')
    .remove(b'/');

/// Connecting (TCP and TLS) may take this long.
const CONNECT_TIMEOUT: Duration = Duration::from_secs(10);
/// A request without a body to speak of (PROPFIND, MOVE, DELETE, a small ranged GET) must be answered in full in
/// this time; a link that went dead mid-request is thereby found in seconds, not minutes. The read is then retried
/// on a new connection (`WebDavFile::read_at`), and the prefetcher keeps retrying with back-off before playback
/// stops with `REMOTE_LINK_LOST` (ADR 0026, ADR 0035).
const REQUEST_TIMEOUT: Duration = Duration::from_secs(10);
/// The slowest transfer still accepted before a request is called stalled: 64 KiB/s on top of `REQUEST_TIMEOUT`.
const SLOWEST_BYTES_PER_SECOND: u64 = 64 * 1024;
/// Nothing waits longer than this.
const LONGEST_REQUEST: Duration = Duration::from_secs(30 * 60);

/// The bound of one request that carries or fetches `bytes`.
fn request_timeout(bytes: u64) -> Duration {
    (REQUEST_TIMEOUT + Duration::from_secs(bytes / SLOWEST_BYTES_PER_SECOND)).min(LONGEST_REQUEST)
}

/// Percent-encode a path, segment by segment (`/` kept).
fn encode_path(path: &str) -> String {
    utf8_percent_encode(path, PATH_SET).to_string()
}

const PROPFIND_BODY: &str = r#"<?xml version="1.0" encoding="utf-8"?><d:propfind xmlns:d="DAV:"><d:prop><d:resourcetype/><d:getcontentlength/><d:getlastmodified/></d:prop></d:propfind>"#;

#[derive(Clone, Debug)]
struct DigestChallenge {
    realm: String,
    nonce: String,
    opaque: Option<String>,
    sha256: bool,
    qop_auth: bool,
    nonce_count: u32,
}

#[derive(Clone, Debug)]
enum AuthState {
    /// No challenge seen yet: send nothing.
    Unknown,
    Basic,
    Digest(DigestChallenge),
}

pub struct WebDavClient {
    http: Client,
    /// `scheme://host[:port]`
    origin: String,
    /// The account's base path without a trailing slash (`""` or `/remote.php/dav/files/me`).
    base_path: String,
    /// `base_path` percent-encoded, as it goes into a URL.
    base_url_path: String,
    user: String,
    password: Option<Zeroizing<String>>,
    auth: Mutex<AuthState>,
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

fn network(error: reqwest::Error) -> RemoteError {
    let text = error.to_string();
    let chain = format!("{error:?}");
    // A refused certificate is a `rustls::Error` somewhere down the source chain (inside an `io::Error`); its
    // `Display` is all the top-level error shows of it ("error sending request"), so look for it there.
    let mut tls_error = None;
    let mut source: Option<&(dyn std::error::Error + 'static)> = std::error::Error::source(&error);
    while let Some(current) = source {
        if let Some(rustls_error) = current.downcast_ref::<rustls::Error>() {
            tls_error = Some(rustls_error.to_string());
            break;
        }
        if let Some(io) = current.downcast_ref::<std::io::Error>() {
            if let Some(rustls_error) = io.get_ref().and_then(|inner| inner.downcast_ref::<rustls::Error>()) {
                tls_error = Some(rustls_error.to_string());
                break;
            }
        }
        source = current.source();
    }
    if let Some(reason) = tls_error {
        return RemoteError::Tls(reason);
    }
    if chain.contains("certificate")
        || chain.contains("InvalidCertificate")
        || chain.contains("UnknownIssuer")
    {
        RemoteError::Tls(text)
    } else {
        RemoteError::Network(text)
    }
}

/// Parse `Digest realm="…", nonce="…", …` out of the challenge headers (Digest preferred over Basic).
fn read_challenge(response: &Response) -> Option<AuthState> {
    let mut basic = false;
    for value in response.headers().get_all(WWW_AUTHENTICATE) {
        let Ok(text) = value.to_str() else { continue };
        if let Some(rest) = text.strip_prefix("Digest ") {
            let mut realm = String::new();
            let mut nonce = String::new();
            let mut opaque = None;
            let mut sha256 = false;
            let mut qop_auth = false;
            for part in split_challenge(rest) {
                let Some((key, value)) = part.split_once('=') else {
                    continue;
                };
                let value = value.trim().trim_matches('"').to_owned();
                match key.trim().to_ascii_lowercase().as_str() {
                    "realm" => realm = value,
                    "nonce" => nonce = value,
                    "opaque" => opaque = Some(value),
                    "algorithm" => sha256 = value.to_ascii_uppercase().starts_with("SHA-256"),
                    "qop" => qop_auth = value.split(',').any(|item| item.trim() == "auth"),
                    _ => {}
                }
            }
            if !nonce.is_empty() {
                return Some(AuthState::Digest(DigestChallenge {
                    realm,
                    nonce,
                    opaque,
                    sha256,
                    qop_auth,
                    nonce_count: 0,
                }));
            }
        } else if text.starts_with("Basic") {
            basic = true;
        }
    }
    basic.then_some(AuthState::Basic)
}

/// Split on commas that are not inside quotes.
fn split_challenge(text: &str) -> Vec<&str> {
    let mut parts = Vec::new();
    let (mut start, mut quoted) = (0, false);
    for (index, ch) in text.char_indices() {
        match ch {
            '"' => quoted = !quoted,
            ',' if !quoted => {
                parts.push(text[start..index].trim());
                start = index + 1;
            }
            _ => {}
        }
    }
    parts.push(text[start..].trim());
    parts
}

fn digest_header(
    challenge: &mut DigestChallenge,
    user: &str,
    password: &str,
    method: &str,
    uri: &str,
) -> String {
    challenge.nonce_count += 1;
    let nc = format!("{:08x}", challenge.nonce_count);
    let cnonce = crate::secret::random_hex(8);
    let h = |text: &str| -> String {
        if challenge.sha256 {
            hex(&Sha256::digest(text.as_bytes()))
        } else {
            hex(&Md5::digest(text.as_bytes()))
        }
    };
    let ha1 = h(&format!("{user}:{}:{password}", challenge.realm));
    let ha2 = h(&format!("{method}:{uri}"));
    let response = if challenge.qop_auth {
        h(&format!(
            "{ha1}:{}:{nc}:{cnonce}:auth:{ha2}",
            challenge.nonce
        ))
    } else {
        h(&format!("{ha1}:{}:{ha2}", challenge.nonce))
    };
    let mut header = format!(
        "Digest username=\"{user}\", realm=\"{}\", nonce=\"{}\", uri=\"{uri}\", algorithm={}, response=\"{response}\"",
        challenge.realm,
        challenge.nonce,
        if challenge.sha256 { "SHA-256" } else { "MD5" }
    );
    if challenge.qop_auth {
        header.push_str(&format!(", qop=auth, nc={nc}, cnonce=\"{cnonce}\""));
    }
    if let Some(opaque) = &challenge.opaque {
        header.push_str(&format!(", opaque=\"{opaque}\""));
    }
    header
}

impl WebDavClient {
    pub fn new(
        account: &RemoteAccount,
        password: Option<Zeroizing<String>>,
    ) -> Result<Self, RemoteError> {
        let scheme = if account.tls == TlsMode::Off {
            "http"
        } else {
            "https"
        };
        let port = account
            .port
            .map(|port| format!(":{port}"))
            .unwrap_or_default();
        // No client-wide timeout: every request gets its own bound (`request_timeout`), so a stalled link is found
        // in seconds while an upload of a whole song may take minutes. Idle keep-alive connections are reused by
        // the 16 tag-reading threads and the prefetcher; they are dropped before a typical server closes them
        // (Apache's default `KeepAliveTimeout` is 5 s, many NAS use 15–30 s), and a reused one the server already
        // closed fails fast and is retried on a fresh connection.
        let mut builder = Client::builder()
            .connect_timeout(CONNECT_TIMEOUT)
            .tcp_nodelay(true)
            .tcp_keepalive(Duration::from_secs(15))
            .pool_idle_timeout(Duration::from_secs(4))
            .pool_max_idle_per_host(32)
            .user_agent("MoeMusicPlayer")
            .redirect(reqwest::redirect::Policy::limited(5));
        if let TlsMode::Pinned { sha256 } = &account.tls {
            builder = builder.use_preconfigured_tls(crate::tls::pinned_client_config(sha256));
        }
        let http = builder
            .build()
            .map_err(|error| RemoteError::Protocol(error.to_string()))?;
        Ok(Self {
            http,
            origin: format!("{scheme}://{}{port}", account.host),
            base_path: account.path.trim_end_matches('/').to_owned(),
            base_url_path: encode_path(account.path.trim_end_matches('/')),
            user: account.username.clone(),
            password,
            auth: Mutex::new(AuthState::Unknown),
        })
    }

    fn url(&self, path: &str) -> String {
        let path = path.trim_matches('/');
        let encoded = encode_path(path);
        if encoded.is_empty() {
            format!("{}{}/", self.origin, self.base_url_path)
        } else {
            format!("{}{}/{encoded}", self.origin, self.base_url_path)
        }
    }

    fn authorize(
        &self,
        builder: RequestBuilder,
        method: &str,
        request_target: &str,
    ) -> RequestBuilder {
        let Some(password) = self.password.as_deref() else {
            return builder;
        };
        let mut state = self
            .auth
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        match &mut *state {
            AuthState::Unknown => builder,
            AuthState::Basic => {
                use base64::Engine as _;
                let token = base64::engine::general_purpose::STANDARD
                    .encode(format!("{}:{password}", self.user));
                builder.header(
                    AUTHORIZATION,
                    HeaderValue::from_str(&format!("Basic {token}")).expect("base64 is ASCII"),
                )
            }
            AuthState::Digest(challenge) => {
                let header = digest_header(challenge, &self.user, password, method, request_target);
                match HeaderValue::from_str(&header) {
                    Ok(value) => builder.header(AUTHORIZATION, value),
                    Err(_) => builder,
                }
            }
        }
    }

    /// Send a request with a body of bytes, answering one authentication challenge (the body is sent again after it).
    fn send_bytes(
        &self,
        method: Method,
        path: &str,
        headers: &[(&str, String)],
        body: &[u8],
        timeout: Duration,
    ) -> Result<Response, RemoteError> {
        let url = self.url(path);
        let request_target = url.strip_prefix(&self.origin).unwrap_or(&url).to_owned();
        for attempt in 0..2 {
            let mut builder = self.http.request(method.clone(), &url);
            for (name, value) in headers {
                builder = builder.header(*name, value);
            }
            builder = builder.body(body.to_vec()).timeout(timeout);
            let response = self
                .authorize(builder, method.as_str(), &request_target)
                .send()
                .map_err(network)?;
            if response.status() != StatusCode::UNAUTHORIZED {
                return Ok(response);
            }
            if attempt == 1 || self.password.is_none() && self.user.is_empty() {
                return Err(RemoteError::Auth);
            }
            match read_challenge(&response) {
                Some(state) => {
                    *self
                        .auth
                        .lock()
                        .unwrap_or_else(std::sync::PoisonError::into_inner) = state
                }
                None => return Err(RemoteError::Auth),
            }
        }
        Err(RemoteError::Auth)
    }

    /// The address of a path as a `Destination` header names it.
    fn destination(&self, path: &str) -> String {
        self.url(path)
    }

    /// Send a request, answering one authentication challenge.
    fn send(
        &self,
        method: Method,
        path: &str,
        headers: &[(&str, String)],
        body: Option<&'static str>,
        timeout: Duration,
    ) -> Result<Response, RemoteError> {
        let url = self.url(path);
        let request_target = url.strip_prefix(&self.origin).unwrap_or(&url).to_owned();
        for attempt in 0..2 {
            let mut builder = self.http.request(method.clone(), &url).timeout(timeout);
            for (name, value) in headers {
                builder = builder.header(*name, value);
            }
            if let Some(body) = body {
                builder = builder
                    .header("Content-Type", "application/xml; charset=utf-8")
                    .body(body);
            }
            let response = self
                .authorize(builder, method.as_str(), &request_target)
                .send()
                .map_err(network)?;
            if response.status() != StatusCode::UNAUTHORIZED {
                return Ok(response);
            }
            if attempt == 1 || self.password.is_none() && self.user.is_empty() {
                return Err(RemoteError::Auth);
            }
            match read_challenge(&response) {
                Some(state) => {
                    *self
                        .auth
                        .lock()
                        .unwrap_or_else(std::sync::PoisonError::into_inner) = state
                }
                None => return Err(RemoteError::Auth),
            }
        }
        Err(RemoteError::Auth)
    }

    fn propfind(&self, path: &str, depth: &str) -> Result<Vec<RawEntry>, RemoteError> {
        let method = Method::from_bytes(b"PROPFIND").expect("a valid method");
        let response = self.send(
            method,
            path,
            &[("Depth", depth.to_owned())],
            Some(PROPFIND_BODY),
            // A big folder's listing is a big answer.
            request_timeout(1024 * 1024),
        )?;
        match response.status().as_u16() {
            207 => {}
            404 => return Err(RemoteError::NotFound),
            401 | 403 => return Err(RemoteError::Auth),
            // A server restarting or a proxy in front of one that is down: worth retrying.
            status @ 500..=599 => return Err(RemoteError::Network(format!("PROPFIND 回應 {status}"))),
            status => return Err(RemoteError::Protocol(format!("PROPFIND 回應 {status}"))),
        }
        let text = response.text().map_err(network)?;
        parse_multistatus(&text, &self.base_path)
    }
}

#[derive(Debug, PartialEq, Eq)]
struct RawEntry {
    /// Path relative to the account's base, without slashes at the ends.
    path: String,
    entry: RemoteEntry,
}

fn parse_multistatus(xml: &str, base_path: &str) -> Result<Vec<RawEntry>, RemoteError> {
    let document = roxmltree::Document::parse(xml)
        .map_err(|error| RemoteError::Protocol(error.to_string()))?;
    let mut entries = Vec::new();
    for response in document
        .descendants()
        .filter(|node| node.is_element() && node.tag_name().name() == "response")
    {
        let child_text = |node: roxmltree::Node, name: &str| -> Option<String> {
            node.descendants()
                .find(|n| n.is_element() && n.tag_name().name() == name)
                .and_then(|n| n.text())
                .map(|text| text.trim().to_owned())
        };
        let Some(href) = child_text(response, "href") else {
            continue;
        };
        // Only successful property sets count.
        let ok = response
            .descendants()
            .filter(|n| n.is_element() && n.tag_name().name() == "status")
            .all(|n| n.text().is_some_and(|text| text.contains(" 2")));
        if !ok {
            continue;
        }
        let href_path = href.find("://").map_or(href.as_str(), |index| {
            let rest = &href[index + 3..];
            rest.find('/').map_or("/", |slash| &rest[slash..])
        });
        let decoded = percent_decode_str(href_path)
            .decode_utf8_lossy()
            .into_owned();
        let relative = decoded
            .strip_prefix(base_path)
            .unwrap_or(&decoded)
            .trim_matches('/')
            .to_owned();
        let is_dir = response
            .descendants()
            .any(|n| n.is_element() && n.tag_name().name() == "collection");
        let size = child_text(response, "getcontentlength")
            .and_then(|text| text.parse().ok())
            .unwrap_or(0);
        let modified_ms = child_text(response, "getlastmodified")
            .and_then(|text| httpdate::parse_http_date(&text).ok())
            .and_then(|time| time.duration_since(std::time::UNIX_EPOCH).ok())
            .map(|duration| duration.as_millis() as i64);
        let name = relative.rsplit('/').next().unwrap_or_default().to_owned();
        entries.push(RawEntry {
            path: relative,
            entry: RemoteEntry {
                name,
                is_dir,
                size,
                modified_ms,
                created_utc_100ns: None,
            },
        });
    }
    Ok(entries)
}

/// The WebDAV tree of an account.
pub struct WebDavFs {
    client: Arc<WebDavClient>,
}

impl WebDavFs {
    pub fn new(
        account: &RemoteAccount,
        password: Option<Zeroizing<String>>,
    ) -> Result<Self, RemoteError> {
        Ok(Self {
            client: Arc::new(WebDavClient::new(account, password)?),
        })
    }
}

impl RemoteFs for WebDavFs {
    fn list(&self, dir: &str) -> Result<Vec<RemoteEntry>, RemoteError> {
        let dir = dir.trim_matches('/');
        let entries = self.client.propfind(dir, "1")?;
        Ok(entries
            .into_iter()
            // The listing includes the folder itself.
            .filter(|raw| raw.path != dir && !raw.entry.name.is_empty())
            .map(|raw| raw.entry)
            .collect())
    }

    fn stat(&self, path: &str) -> Result<RemoteEntry, RemoteError> {
        self.client
            .propfind(path.trim_matches('/'), "0")?
            .into_iter()
            .next()
            .map(|raw| raw.entry)
            .ok_or(RemoteError::NotFound)
    }

    fn replace_file(
        &self,
        path: &str,
        source: &mut dyn Read,
        len: u64,
        expected: Option<FileStamp>,
    ) -> Result<RemoteEntry, RemoteError> {
        // The most a song may be to be sent in one request from memory.
        const LARGEST: u64 = 512 * 1024 * 1024;
        // This also signs in, so the request that carries the file does not have to be answered with a challenge.
        // A file that does not exist yet can be created (when nothing was expected of it).
        let current = match self.stat(path) {
            Ok(current) => Some(current),
            Err(RemoteError::NotFound) if expected.is_none() => None,
            Err(error) => return Err(error),
        };
        if let (Some(expected), Some(current)) = (expected, current) {
            if FileStamp::of(&current) != expected {
                return Err(RemoteError::Changed);
            }
        }
        if len > LARGEST {
            return Err(RemoteError::Unsupported("檔案太大，WebDAV 目前一次最多傳 512 MiB".to_owned()));
        }
        let mut data = Vec::with_capacity(len as usize);
        source
            .take(len)
            .read_to_end(&mut data)
            .map_err(|error| RemoteError::Protocol(format!("could not read the file to send: {error}")))?;
        if data.len() as u64 != len {
            return Err(RemoteError::Protocol(format!("only {} of {len} bytes were sent", data.len())));
        }
        let temp = replacement_temp_path(path);
        let result = (|| {
            let put = self.client.send_bytes(
                Method::PUT,
                &temp,
                &[("Content-Type", "application/octet-stream".to_owned())],
                &data,
                request_timeout(len),
            )?;
            match put.status().as_u16() {
                200 | 201 | 204 => {}
                401 | 403 => return Err(RemoteError::Auth),
                507 => return Err(RemoteError::Protocol("伺服器空間不足".to_owned())),
                status => return Err(RemoteError::Protocol(format!("PUT 回應 {status}"))),
            }
            // All of it must have arrived before it takes the place of the original.
            let held = self.stat(&temp)?;
            if held.size != len {
                return Err(RemoteError::Protocol(format!("the server holds {} of {len} bytes", held.size)));
            }
            let method = Method::from_bytes(b"MOVE").expect("a valid method");
            let moved = self.client.send(
                method,
                &temp,
                &[
                    ("Destination", self.client.destination(path)),
                    ("Overwrite", "T".to_owned()),
                ],
                None,
                REQUEST_TIMEOUT,
            )?;
            match moved.status().as_u16() {
                200 | 201 | 204 => Ok(()),
                401 | 403 => Err(RemoteError::Auth),
                404 => Err(RemoteError::NotFound),
                status => Err(RemoteError::Protocol(format!("MOVE 回應 {status}"))),
            }
        })();
        if result.is_err() {
            let _ = self.remove_file(&temp);
        }
        result?;
        self.stat(path)
    }

    fn remove_file(&self, path: &str) -> Result<(), RemoteError> {
        let response = self.client.send(Method::DELETE, path.trim_matches('/'), &[], None, REQUEST_TIMEOUT)?;
        match response.status().as_u16() {
            200 | 204 => Ok(()),
            404 => Err(RemoteError::NotFound),
            401 | 403 => Err(RemoteError::Auth),
            status => Err(RemoteError::Protocol(format!("DELETE 回應 {status}"))),
        }
    }

    fn rename_file(&self, from: &str, to: &str) -> Result<(), RemoteError> {
        let method = Method::from_bytes(b"MOVE").expect("a valid method");
        let moved = self.client.send(
            method,
            from.trim_matches('/'),
            &[
                ("Destination", self.client.destination(to)),
                ("Overwrite", "F".to_owned()),
            ],
            None,
            REQUEST_TIMEOUT,
        )?;
        match moved.status().as_u16() {
            200 | 201 | 204 => Ok(()),
            401 | 403 => Err(RemoteError::Auth),
            404 => Err(RemoteError::NotFound),
            status => Err(RemoteError::Protocol(format!("MOVE 回應 {status}"))),
        }
    }

    fn open(&self, path: &str) -> Result<Box<dyn RemoteFile>, RemoteError> {
        let entry = self.stat(path)?;
        if entry.is_dir {
            return Err(RemoteError::NotFound);
        }
        Ok(Box::new(WebDavFile {
            client: Arc::clone(&self.client),
            path: path.trim_matches('/').to_owned(),
            len: entry.size,
        }))
    }
}

pub struct WebDavFile {
    client: Arc<WebDavClient>,
    path: String,
    len: u64,
}

impl RemoteFile for WebDavFile {
    fn len(&self) -> u64 {
        self.len
    }

    fn read_at(&mut self, offset: u64, buf: &mut [u8]) -> Result<usize, RemoteError> {
        if buf.is_empty() || offset >= self.len {
            return Ok(0);
        }
        let last = (offset + buf.len() as u64 - 1).min(self.len - 1);
        let mut attempts = 0;
        loop {
            attempts += 1;
            let started = std::time::Instant::now();
            let result = self.fetch(offset, last, buf);
            match result {
                // A quick failure (a pooled connection the server had closed, a restart) is retried at once on a
                // new connection. A request that ran into its timeout is a stalled link: report it now and let the
                // prefetcher's back-off (ADR 0026) take over instead of waiting out two more timeouts here.
                Err(RemoteError::Network(_))
                    if attempts < 3 && started.elapsed() < Duration::from_secs(2) =>
                {
                    std::thread::sleep(Duration::from_millis(200 * attempts))
                }
                other => return other,
            }
        }
    }
}

impl WebDavFile {
    fn fetch(&self, offset: u64, last: u64, buf: &mut [u8]) -> Result<usize, RemoteError> {
        let response = self.client.send(
            Method::GET,
            &self.path,
            &[(RANGE.as_str(), format!("bytes={offset}-{last}"))],
            None,
            request_timeout(last - offset + 1),
        )?;
        match response.status().as_u16() {
            206 => {
                // Check the server answered the range we asked for.
                if let Some(range) = response
                    .headers()
                    .get(CONTENT_RANGE)
                    .and_then(|value| value.to_str().ok())
                {
                    let start = range
                        .strip_prefix("bytes ")
                        .and_then(|rest| rest.split('-').next())
                        .and_then(|text| text.parse::<u64>().ok());
                    if start.is_some_and(|start| start != offset) {
                        return Err(RemoteError::Protocol("伺服器回傳了不同的範圍".to_owned()));
                    }
                }
            }
            // A server that ignores Range sends the whole file: usable only from the start.
            200 if offset == 0 => {}
            200 => {
                return Err(RemoteError::Protocol(
                    "伺服器不支援範圍讀取（Range）".to_owned(),
                ))
            }
            416 => return Ok(0),
            404 => return Err(RemoteError::NotFound),
            401 | 403 => return Err(RemoteError::Auth),
            status @ 500..=599 => return Err(RemoteError::Network(format!("GET 回應 {status}"))),
            status => return Err(RemoteError::Protocol(format!("GET 回應 {status}"))),
        }
        let wanted = ((last - offset + 1) as usize).min(buf.len());
        let mut body = response;
        let mut filled = 0;
        while filled < wanted {
            use std::io::Read as _;
            match body.read(&mut buf[filled..wanted]) {
                Ok(0) => break,
                Ok(count) => filled += count,
                Err(error) => return Err(RemoteError::Network(error.to_string())),
            }
        }
        Ok(filled)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::account::RemoteProtocol;
    use crate::test_server::{Auth, TestWebDav};

    fn account(server: &TestWebDav) -> RemoteAccount {
        RemoteAccount {
            id: "t".to_owned(),
            name: "test".to_owned(),
            protocol: RemoteProtocol::Webdav,
            host: "127.0.0.1".to_owned(),
            port: Some(server.address.port()),
            path: String::new(),
            username: "me".to_owned(),
            password: None,
            tls: TlsMode::Off,
            only_on_wifi: false,
        }
    }

    fn music_folder(name: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!("moe-webdav-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("專輯 A").join("CD 1")).unwrap();
        std::fs::create_dir_all(dir.join("Single")).unwrap();
        std::fs::write(
            dir.join("專輯 A").join("01 歌 (live) & more.flac"),
            crate::fs::testing::pattern(700_000),
        )
        .unwrap();
        std::fs::write(
            dir.join("專輯 A").join("CD 1").join("02.mp3"),
            crate::fs::testing::pattern(1_234),
        )
        .unwrap();
        std::fs::write(dir.join("Single").join("cover.jpg"), b"jpg").unwrap();
        std::fs::write(dir.join("top.wav"), crate::fs::testing::pattern(10)).unwrap();
        dir
    }

    fn fs_for(server: &TestWebDav, password: Option<&str>) -> WebDavFs {
        WebDavFs::new(
            &account(server),
            password.map(|p| Zeroizing::new(p.to_owned())),
        )
        .unwrap()
    }

    #[test]
    fn lists_folders_and_files_with_unicode_names_sizes_and_times() {
        let dir = music_folder("list");
        let server = TestWebDav::serve(&dir, Auth::None);
        let fs = fs_for(&server, None);
        let mut root: Vec<_> = fs.list("").unwrap();
        root.sort_by(|a, b| a.name.cmp(&b.name));
        let names: Vec<(&str, bool)> = root.iter().map(|e| (e.name.as_str(), e.is_dir)).collect();
        assert_eq!(
            names,
            [("Single", true), ("top.wav", false), ("專輯 A", true)]
        );
        assert_eq!(root[1].size, 10);
        assert!(
            root[1].modified_ms.is_some_and(|ms| ms > 1_600_000_000_000),
            "{:?}",
            root[1].modified_ms
        );
        let album = fs.list("專輯 A").unwrap();
        let mut names: Vec<_> = album.iter().map(|e| e.name.clone()).collect();
        names.sort();
        assert_eq!(names, ["01 歌 (live) & more.flac", "CD 1"]);
        let file = album.iter().find(|e| !e.is_dir).unwrap();
        assert_eq!(file.size, 700_000);
        assert_eq!(fs.stat("專輯 A/CD 1/02.mp3").unwrap().size, 1_234);
        assert!(fs.stat("專輯 A").unwrap().is_dir);
        assert_eq!(fs.stat("nope.flac"), Err(RemoteError::NotFound));
        assert_eq!(fs.list("nope"), Err(RemoteError::NotFound));
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn a_file_reads_through_the_cached_range_reader_byte_for_byte() {
        use std::io::{Read, Seek, SeekFrom};
        let dir = music_folder("read");
        let server = TestWebDav::serve(&dir, Auth::None);
        let fs = fs_for(&server, None);
        let expected = std::fs::read(dir.join("專輯 A").join("01 歌 (live) & more.flac")).unwrap();
        let mut reader =
            crate::fs::RangeReader::new(fs.open("專輯 A/01 歌 (live) & more.flac").unwrap());
        assert_eq!(reader.len(), 700_000);
        let mut all = Vec::new();
        reader.read_to_end(&mut all).unwrap();
        assert_eq!(all, expected);
        let mut tail = [0u8; 100];
        reader.seek(SeekFrom::End(-100)).unwrap();
        reader.read_exact(&mut tail).unwrap();
        assert_eq!(tail[..], expected[expected.len() - 100..]);
        assert!(
            server.count("GET") <= 6,
            "{} GETs for 700 kB",
            server.count("GET")
        );
        assert!(fs.open("專輯 A").is_err(), "a folder is not a file");
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn a_file_is_replaced_through_a_temporary_file_with_digest_authentication() {
        let dir = music_folder("replace");
        let server = TestWebDav::serve(&dir, Auth::Digest { user: "me".into(), password: "s3cret".into() });
        let fs = fs_for(&server, Some("s3cret"));
        let path = "專輯 A/01 歌 (live) & more.flac";
        let before = FileStamp::of(&fs.stat(path).unwrap());
        let new = vec![9_u8; 1_234_567];
        let entry = fs
            .replace_file(path, &mut new.as_slice(), new.len() as u64, Some(before))
            .unwrap();
        assert_eq!(entry.size, new.len() as u64);
        assert_eq!(std::fs::read(dir.join("專輯 A").join("01 歌 (live) & more.flac")).unwrap(), new);
        let leftovers = std::fs::read_dir(dir.join("專輯 A"))
            .unwrap()
            .filter(|e| e.as_ref().unwrap().file_name().to_string_lossy().starts_with(".moetmp-"))
            .count();
        assert_eq!(leftovers, 0);
        // The file changed since it was read: it is not overwritten.
        let stale = fs.replace_file(path, &mut &b"x"[..], 1, Some(before));
        assert_eq!(stale, Err(RemoteError::Changed));
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn a_replaced_file_is_kept_as_the_single_bak_backup_on_webdav() {
        let dir = music_folder("backup");
        let server = TestWebDav::serve(&dir, Auth::Digest { user: "me".into(), password: "s3cret".into() });
        let fs = fs_for(&server, Some("s3cret"));
        let path = "專輯 A/list.m3u8";
        let folder = dir.join("專輯 A");
        std::fs::write(folder.join("list.m3u8"), b"one\n").unwrap();
        std::fs::write(folder.join("list.m3u8.bak"), b"stale").unwrap();
        for (new, kept) in [(&b"two\n"[..], &b"one\n"[..]), (&b"three\n"[..], &b"two\n"[..])] {
            let before = FileStamp::of(&fs.stat(path).unwrap());
            fs.replace_file_keeping_backup(path, &mut &new[..], new.len() as u64, Some(before), "專輯 A/list.m3u8.bak")
                .unwrap();
            assert_eq!(std::fs::read(folder.join("list.m3u8")).unwrap(), new);
            assert_eq!(std::fs::read(folder.join("list.m3u8.bak")).unwrap(), kept);
        }
        let strays = std::fs::read_dir(&folder)
            .unwrap()
            .filter(|e| e.as_ref().unwrap().file_name().to_string_lossy().starts_with(".moetmp-"))
            .count();
        assert_eq!(strays, 0);
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn basic_authentication_waits_for_the_challenge_and_is_then_remembered() {
        let dir = music_folder("basic");
        let server = TestWebDav::serve(
            &dir,
            Auth::Basic {
                user: "me".into(),
                password: "pä ss:wörd".into(),
            },
        );
        let fs = fs_for(&server, Some("pä ss:wörd"));
        assert!(fs.list("").is_ok());
        assert_eq!(
            server.challenges.load(std::sync::atomic::Ordering::SeqCst),
            1,
            "one 401, then the scheme is remembered"
        );
        assert!(fs.list("Single").is_ok());
        assert_eq!(
            server.challenges.load(std::sync::atomic::Ordering::SeqCst),
            1
        );
        let wrong = fs_for(&server, Some("wrong"));
        assert_eq!(wrong.list(""), Err(RemoteError::Auth));
        let none = fs_for(&server, None);
        assert_eq!(none.list(""), Err(RemoteError::Auth), "no password at all");
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn digest_authentication_works_over_many_requests_without_sending_the_password() {
        let dir = music_folder("digest");
        let server = TestWebDav::serve(
            &dir,
            Auth::Digest {
                user: "me".into(),
                password: "s3cret".into(),
            },
        );
        let fs = fs_for(&server, Some("s3cret"));
        assert!(fs.list("").is_ok());
        let mut reader =
            crate::fs::RangeReader::new(fs.open("專輯 A/01 歌 (live) & more.flac").unwrap());
        let mut all = Vec::new();
        std::io::Read::read_to_end(&mut reader, &mut all).unwrap();
        assert_eq!(all.len(), 700_000);
        assert_eq!(
            fs_for(&server, Some("wrong")).list(""),
            Err(RemoteError::Auth)
        );
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn an_unreachable_server_is_a_network_error_and_a_missing_account_path_is_not_found() {
        let dir = music_folder("down");
        let server = TestWebDav::serve(&dir, Auth::None);
        let mut gone = account(&server);
        drop(server);
        gone.port = Some(9); // nothing listens on the discard port
        let fs = WebDavFs::new(&gone, None).unwrap();
        assert!(matches!(fs.list(""), Err(RemoteError::Network(_))));
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn a_base_path_is_part_of_every_request_and_stripped_from_the_listing() {
        let dir = music_folder("base");
        let server = TestWebDav::serve(&dir, Auth::None);
        let mut with_base = account(&server);
        with_base.path = "/專輯 A".to_owned();
        let fs = WebDavFs::new(&with_base, None).unwrap();
        let mut names: Vec<_> = fs.list("").unwrap().into_iter().map(|e| e.name).collect();
        names.sort();
        assert_eq!(names, ["01 歌 (live) & more.flac", "CD 1"]);
        assert_eq!(fs.stat("CD 1/02.mp3").unwrap().size, 1_234);
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn a_base_path_with_url_special_characters_is_encoded() {
        let dir = music_folder("special");
        std::fs::create_dir_all(dir.join("a #1 %20 b;")).unwrap();
        std::fs::write(dir.join("a #1 %20 b;").join("歌 #2.flac"), b"12345").unwrap();
        let server = TestWebDav::serve(&dir, Auth::None);
        let mut with_base = account(&server);
        with_base.path = "/a #1 %20 b;".to_owned();
        let fs = WebDavFs::new(&with_base, None).unwrap();
        let names: Vec<_> = fs.list("").unwrap().into_iter().map(|e| e.name).collect();
        assert_eq!(names, ["歌 #2.flac"]);
        assert_eq!(fs.stat("歌 #2.flac").unwrap().size, 5);
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn requests_are_bounded_by_their_size() {
        assert_eq!(request_timeout(0), REQUEST_TIMEOUT);
        assert_eq!(request_timeout(1024 * 1024), REQUEST_TIMEOUT + Duration::from_secs(16));
        assert_eq!(request_timeout(u64::MAX), LONGEST_REQUEST);
    }

    #[test]
    fn multistatus_parsing_handles_absolute_hrefs_encoded_names_and_failed_properties() {
        let xml = r#"<?xml version="1.0"?><D:multistatus xmlns:D="DAV:">
          <D:response><D:href>http://host/dav/music/</D:href><D:propstat><D:prop><D:resourcetype><D:collection/></D:resourcetype></D:prop><D:status>HTTP/1.1 200 OK</D:status></D:propstat></D:response>
          <D:response><D:href>/dav/music/%E6%AD%8C%20one.flac</D:href><D:propstat><D:prop><D:resourcetype/><D:getcontentlength>42</D:getcontentlength><D:getlastmodified>Tue, 15 Nov 1994 08:12:31 GMT</D:getlastmodified></D:prop><D:status>HTTP/1.1 200 OK</D:status></D:propstat></D:response>
          <D:response><D:href>/dav/music/locked.flac</D:href><D:propstat><D:prop/><D:status>HTTP/1.1 403 Forbidden</D:status></D:propstat></D:response>
        </D:multistatus>"#;
        let entries = parse_multistatus(xml, "/dav").unwrap();
        assert_eq!(
            entries.len(),
            2,
            "the entry whose properties failed is skipped"
        );
        assert_eq!(entries[0].path, "music");
        assert!(entries[0].entry.is_dir);
        assert_eq!(entries[1].entry.name, "歌 one.flac");
        assert_eq!(entries[1].entry.size, 42);
        assert_eq!(entries[1].entry.modified_ms, Some(784_887_151_000));
        assert!(matches!(
            parse_multistatus("not xml", ""),
            Err(RemoteError::Protocol(_))
        ));
    }

    #[test]
    fn a_pinned_certificate_config_builds_and_the_pin_is_compared_case_insensitively() {
        let config = crate::tls::pinned_client_config(&"AB".repeat(32));
        assert!(!config.alpn_protocols.is_empty() || config.alpn_protocols.is_empty());
        assert_eq!(
            crate::tls::sha256_hex(b"abc"),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
    }
}
