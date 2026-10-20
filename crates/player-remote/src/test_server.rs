//! A small WebDAV server over a real folder, for tests (enabled for other crates by the `testing` feature): it answers
//! `PROPFIND` (depth 0 and 1), `HEAD` and `GET` with `Range`, and can demand Basic or Digest authentication. It is only
//! meant to be talked to by this crate's client, one connection per request.

use std::collections::HashMap;
use std::io::{BufRead, BufReader, Read, Write};
use std::net::{SocketAddr, TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;

use md5::{Digest as _, Md5};

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Auth {
    None,
    Basic { user: String, password: String },
    Digest { user: String, password: String },
}

pub struct TestWebDav {
    pub address: SocketAddr,
    /// Requests answered (authenticated ones only), by method.
    pub requests: Arc<Mutex<HashMap<String, u64>>>,
    /// Requests that were refused with 401.
    pub challenges: Arc<AtomicU64>,
    stop: Arc<AtomicBool>,
    thread: Option<JoinHandle<()>>,
}

fn percent_decode(text: &str) -> String {
    percent_encoding::percent_decode_str(text)
        .decode_utf8_lossy()
        .into_owned()
}

fn xml_escape(text: &str) -> String {
    text.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

fn md5_hex(text: &str) -> String {
    Md5::digest(text.as_bytes())
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

/// `a="b", c=d` pairs of an `Authorization: Digest` header.
fn parse_pairs(text: &str) -> HashMap<String, String> {
    let mut pairs = HashMap::new();
    for part in text.split(',') {
        if let Some((key, value)) = part.split_once('=') {
            pairs.insert(
                key.trim().to_ascii_lowercase(),
                value.trim().trim_matches('"').to_owned(),
            );
        }
    }
    pairs
}

const REALM: &str = "moe-test";
const NONCE: &str = "dcd98b7102dd2f0e8b11d0f600bfb0c093";

fn authorized(auth: &Auth, method: &str, headers: &HashMap<String, String>) -> bool {
    let header = headers
        .get("authorization")
        .map(String::as_str)
        .unwrap_or_default();
    match auth {
        Auth::None => true,
        Auth::Basic { user, password } => {
            use base64::Engine as _;
            header
                .strip_prefix("Basic ")
                .and_then(|encoded| {
                    base64::engine::general_purpose::STANDARD
                        .decode(encoded.trim())
                        .ok()
                })
                .is_some_and(|decoded| decoded == format!("{user}:{password}").as_bytes())
        }
        Auth::Digest { user, password } => {
            let Some(rest) = header.strip_prefix("Digest ") else {
                return false;
            };
            let pairs = parse_pairs(rest);
            let get = |key: &str| pairs.get(key).map(String::as_str).unwrap_or_default();
            if get("username") != user || get("realm") != REALM || get("nonce") != NONCE {
                return false;
            }
            let ha1 = md5_hex(&format!("{user}:{REALM}:{password}"));
            let ha2 = md5_hex(&format!("{method}:{}", get("uri")));
            let expected = if get("qop") == "auth" {
                md5_hex(&format!(
                    "{ha1}:{NONCE}:{}:{}:auth:{ha2}",
                    get("nc"),
                    get("cnonce")
                ))
            } else {
                md5_hex(&format!("{ha1}:{NONCE}:{ha2}"))
            };
            get("response") == expected
        }
    }
}

fn http_date(time: std::time::SystemTime) -> String {
    let seconds = time
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let days = seconds / 86_400;
    let weekday = ["Thu", "Fri", "Sat", "Sun", "Mon", "Tue", "Wed"][(days % 7) as usize];
    // civil-from-days
    let z = days as i64 + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let year = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = year + i64::from(month <= 2);
    let month_name = [
        "Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec",
    ][(month - 1) as usize];
    format!(
        "{weekday}, {day:02} {month_name} {year} {:02}:{:02}:{:02} GMT",
        seconds / 3600 % 24,
        seconds / 60 % 60,
        seconds % 60
    )
}

fn respond(
    stream: &mut TcpStream,
    status: &str,
    headers: &[(String, String)],
    body: &[u8],
    head_only: bool,
) {
    let mut text = format!(
        "HTTP/1.1 {status}\r\nConnection: close\r\nContent-Length: {}\r\n",
        body.len()
    );
    for (key, value) in headers {
        text.push_str(&format!("{key}: {value}\r\n"));
    }
    text.push_str("\r\n");
    let _ = stream.write_all(text.as_bytes());
    if !head_only {
        let _ = stream.write_all(body);
    }
}

fn multistatus_entry(base: &str, relative: &str, path: &Path) -> Option<String> {
    let metadata = std::fs::metadata(path).ok()?;
    let mut href = format!("{base}/{}", relative.trim_matches('/'));
    if metadata.is_dir() && !href.ends_with('/') {
        href.push('/');
    }
    let href = href.replace("//", "/");
    let encoded = href
        .split('/')
        .map(|segment| {
            percent_encoding::utf8_percent_encode(segment, percent_encoding::NON_ALPHANUMERIC)
                .to_string()
        })
        .collect::<Vec<_>>()
        .join("/");
    let modified = metadata.modified().map(http_date).unwrap_or_default();
    Some(if metadata.is_dir() {
        format!(
            "<d:response><d:href>{}</d:href><d:propstat><d:prop><d:resourcetype><d:collection/></d:resourcetype><d:getlastmodified>{modified}</d:getlastmodified></d:prop><d:status>HTTP/1.1 200 OK</d:status></d:propstat></d:response>",
            xml_escape(&encoded)
        )
    } else {
        format!(
            "<d:response><d:href>{}</d:href><d:propstat><d:prop><d:resourcetype/><d:getcontentlength>{}</d:getcontentlength><d:getlastmodified>{modified}</d:getlastmodified></d:prop><d:status>HTTP/1.1 200 OK</d:status></d:propstat></d:response>",
            xml_escape(&encoded),
            metadata.len()
        )
    })
}

fn handle(
    mut stream: TcpStream,
    root: &Path,
    auth: &Auth,
    requests: &Mutex<HashMap<String, u64>>,
    challenges: &AtomicU64,
) {
    let _ = stream.set_read_timeout(Some(std::time::Duration::from_secs(10)));
    let mut reader = BufReader::new(stream.try_clone().expect("clone the stream"));
    let mut line = String::new();
    if reader.read_line(&mut line).unwrap_or(0) == 0 {
        return;
    }
    let mut parts = line.split_whitespace();
    let (method, target) = (
        parts.next().unwrap_or_default().to_owned(),
        parts.next().unwrap_or_default().to_owned(),
    );
    let mut headers = HashMap::new();
    loop {
        let mut header = String::new();
        if reader.read_line(&mut header).unwrap_or(0) == 0 || header.trim().is_empty() {
            break;
        }
        if let Some((key, value)) = header.split_once(':') {
            headers.insert(key.trim().to_ascii_lowercase(), value.trim().to_owned());
        }
    }
    let length: usize = headers
        .get("content-length")
        .and_then(|value| value.parse().ok())
        .unwrap_or(0);
    let mut body = vec![0u8; length];
    let _ = reader.read_exact(&mut body);

    if !authorized(auth, &method, &headers) {
        challenges.fetch_add(1, Ordering::SeqCst);
        let challenge = match auth {
            Auth::Basic { .. } => format!("Basic realm=\"{REALM}\""),
            _ => format!("Digest realm=\"{REALM}\", qop=\"auth\", algorithm=MD5, nonce=\"{NONCE}\", opaque=\"5ccc069c403ebaf9f0171e9517f40e41\""),
        };
        respond(
            &mut stream,
            "401 Unauthorized",
            &[("WWW-Authenticate".to_owned(), challenge)],
            b"",
            false,
        );
        return;
    }
    *requests.lock().unwrap().entry(method.clone()).or_insert(0) += 1;

    let path_part = target.split('?').next().unwrap_or_default();
    let relative = percent_decode(path_part);
    let relative = relative.trim_matches('/');
    if relative.split('/').any(|segment| segment == "..") {
        respond(&mut stream, "403 Forbidden", &[], b"", false);
        return;
    }
    let path: PathBuf = if relative.is_empty() {
        root.to_path_buf()
    } else {
        root.join(relative)
    };
    match method.as_str() {
        "PROPFIND" => {
            if !path.exists() {
                respond(&mut stream, "404 Not Found", &[], b"", false);
                return;
            }
            let mut xml = String::from(
                "<?xml version=\"1.0\" encoding=\"utf-8\"?><d:multistatus xmlns:d=\"DAV:\">",
            );
            xml.push_str(&multistatus_entry("", relative, &path).unwrap_or_default());
            if headers.get("depth").map(String::as_str) != Some("0") && path.is_dir() {
                let mut children: Vec<_> = std::fs::read_dir(&path)
                    .map(|entries| entries.flatten().collect())
                    .unwrap_or_default();
                children.sort_by_key(|entry| entry.file_name());
                for child in children {
                    let name = child.file_name().to_string_lossy().into_owned();
                    let child_relative = if relative.is_empty() {
                        name
                    } else {
                        format!("{relative}/{name}")
                    };
                    xml.push_str(
                        &multistatus_entry("", &child_relative, &child.path()).unwrap_or_default(),
                    );
                }
            }
            xml.push_str("</d:multistatus>");
            respond(
                &mut stream,
                "207 Multi-Status",
                &[(
                    "Content-Type".to_owned(),
                    "application/xml; charset=utf-8".to_owned(),
                )],
                xml.as_bytes(),
                false,
            );
        }
        "PUT" => {
            let existed = path.exists();
            match std::fs::write(&path, &body) {
                Ok(()) => respond(
                    &mut stream,
                    if existed { "204 No Content" } else { "201 Created" },
                    &[],
                    b"",
                    false,
                ),
                Err(_) => respond(&mut stream, "409 Conflict", &[], b"", false),
            }
        }
        "DELETE" => {
            let status = match std::fs::remove_file(&path) {
                Ok(()) => "204 No Content",
                Err(_) => "404 Not Found",
            };
            respond(&mut stream, status, &[], b"", false);
        }
        "MOVE" => {
            // `Destination` is the full address; what follows the host is the path under the same root.
            let destination = headers
                .get("destination")
                .map(|value| {
                    let without_scheme = value.split_once("://").map_or(value.as_str(), |(_, rest)| rest);
                    let path_only = without_scheme.split_once('/').map_or("", |(_, rest)| rest);
                    percent_decode(path_only.split('?').next().unwrap_or_default())
                })
                .unwrap_or_default();
            let destination = root.join(destination.trim_matches('/'));
            let overwrite = headers.get("overwrite").map_or(true, |value| value != "F");
            if !path.exists() {
                respond(&mut stream, "404 Not Found", &[], b"", false);
            } else if destination.exists() && !overwrite {
                respond(&mut stream, "412 Precondition Failed", &[], b"", false);
            } else {
                let existed = destination.exists();
                match std::fs::rename(&path, &destination) {
                    Ok(()) => respond(
                        &mut stream,
                        if existed { "204 No Content" } else { "201 Created" },
                        &[],
                        b"",
                        false,
                    ),
                    Err(_) => respond(&mut stream, "409 Conflict", &[], b"", false),
                }
            }
        }
        "GET" | "HEAD" => {
            let Ok(mut file) = std::fs::File::open(&path)
                .and_then(|file| file.metadata().map(|meta| (file, meta)))
                .and_then(|(file, meta)| {
                    if meta.is_file() {
                        Ok(file)
                    } else {
                        Err(std::io::Error::other("not a file"))
                    }
                })
            else {
                respond(&mut stream, "404 Not Found", &[], b"", false);
                return;
            };
            let total = file.metadata().map(|meta| meta.len()).unwrap_or(0);
            let head_only = method == "HEAD";
            let range = headers
                .get("range")
                .and_then(|value| value.strip_prefix("bytes="))
                .and_then(|value| {
                    let (start, end) = value.split_once('-')?;
                    let start: u64 = start.parse().ok()?;
                    let end: u64 = if end.is_empty() {
                        total.saturating_sub(1)
                    } else {
                        end.parse().ok()?
                    };
                    Some((start, end.min(total.saturating_sub(1))))
                });
            match range {
                Some((start, end)) if start < total && start <= end => {
                    use std::io::Seek as _;
                    let mut data = vec![0u8; (end - start + 1) as usize];
                    let _ = file.seek(std::io::SeekFrom::Start(start));
                    let _ = file.read_exact(&mut data);
                    respond(
                        &mut stream,
                        "206 Partial Content",
                        &[
                            (
                                "Content-Range".to_owned(),
                                format!("bytes {start}-{end}/{total}"),
                            ),
                            ("Accept-Ranges".to_owned(), "bytes".to_owned()),
                        ],
                        &data,
                        head_only,
                    );
                }
                Some(_) => respond(
                    &mut stream,
                    "416 Range Not Satisfiable",
                    &[("Content-Range".to_owned(), format!("bytes */{total}"))],
                    b"",
                    false,
                ),
                None => {
                    let mut data = Vec::new();
                    let _ = file.read_to_end(&mut data);
                    respond(
                        &mut stream,
                        "200 OK",
                        &[("Accept-Ranges".to_owned(), "bytes".to_owned())],
                        &data,
                        head_only,
                    );
                }
            }
        }
        _ => respond(&mut stream, "405 Method Not Allowed", &[], b"", false),
    }
}

impl TestWebDav {
    pub fn serve(root: &Path, auth: Auth) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind a local port");
        let address = listener.local_addr().expect("local address");
        listener
            .set_nonblocking(true)
            .expect("non-blocking listener");
        let stop = Arc::new(AtomicBool::new(false));
        let requests = Arc::new(Mutex::new(HashMap::new()));
        let challenges = Arc::new(AtomicU64::new(0));
        let (root, auth) = (root.to_path_buf(), auth);
        let thread = {
            let (stop, requests, challenges) = (
                Arc::clone(&stop),
                Arc::clone(&requests),
                Arc::clone(&challenges),
            );
            std::thread::spawn(move || {
                while !stop.load(Ordering::SeqCst) {
                    match listener.accept() {
                        Ok((stream, _)) => {
                            let _ = stream.set_nonblocking(false);
                            let (root, auth, requests, challenges) = (
                                root.clone(),
                                auth.clone(),
                                Arc::clone(&requests),
                                Arc::clone(&challenges),
                            );
                            std::thread::spawn(move || {
                                handle(stream, &root, &auth, &requests, &challenges)
                            });
                        }
                        Err(_) => std::thread::sleep(std::time::Duration::from_millis(2)),
                    }
                }
            })
        };
        Self {
            address,
            requests,
            challenges,
            stop,
            thread: Some(thread),
        }
    }

    pub fn url(&self) -> String {
        format!("http://{}", self.address)
    }

    pub fn count(&self, method: &str) -> u64 {
        self.requests
            .lock()
            .unwrap()
            .get(method)
            .copied()
            .unwrap_or(0)
    }
}

impl Drop for TestWebDav {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}
