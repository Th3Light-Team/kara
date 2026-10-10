//! A small, hermetic S3 server on `127.0.0.1`, so the **real** AWS client of
//! `object_store` (request signing, URL encoding of keys, XML, paging,
//! multipart, conditional requests, retries) can be tested end to end without
//! a network or MinIO.
//!
//! It speaks the subset `object_store` uses, path-style, one bucket:
//! `ListObjectsV2` (prefix, delimiter, continuation token, max-keys, capped
//! by [`MockState::page_size`]), `GET` (with `Range: bytes=N-`), `HEAD`,
//! `PUT` (with `If-None-Match: *`), `PUT` + `x-amz-copy-source`, `DELETE`,
//! `POST ?delete` (bulk), and multipart (`?uploads`, `?partNumber&uploadId`,
//! complete, abort).
//!
//! Every request must carry a valid SigV4 signature for the configured key:
//! the mock recomputes it the way AWS documents it — the path is decoded and
//! each segment re-encoded with the RFC 3986 unreserved set, the query sorted
//! and encoded the same way, the signed headers trimmed — and compares. A
//! wrong secret is `403 SignatureDoesNotMatch`, an unknown key id
//! `403 InvalidAccessKeyId`. A body whose SHA-256 does not match
//! `x-amz-content-sha256` is refused.
//!
//! Faults: answer the next N requests with `503 SlowDown`, or stall them.
//! Not modelled: versioning, storage classes, checksums, SSE, presigned URLs.

#![allow(dead_code, clippy::result_large_err)]

use std::collections::{BTreeMap, HashMap};
use std::convert::Infallible;
use std::io;
use std::net::SocketAddr;
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use bytes::Bytes;
use hmac::{Hmac, KeyInit, Mac};
use http_body_util::combinators::BoxBody;
use http_body_util::{BodyExt, Full, StreamBody};
use hyper::body::Frame;
use hyper::body::Incoming;
use hyper::server::conn::http1;
use hyper::service::service_fn;
use hyper::{Method, Request, Response, StatusCode};
use hyper_util::rt::TokioIo;
use percent_encoding::{AsciiSet, NON_ALPHANUMERIC, percent_decode_str, utf8_percent_encode};
use sha2::{Digest, Sha256};
use tokio::runtime::Runtime;

pub const BUCKET: &str = "kara-test";
pub const ACCESS_KEY: &str = "AKIAKARAMOCK";
pub const SECRET_KEY: &str = "mock/Secret+Key=With/Odd+Chars";
pub const REGION: &str = "us-east-1";

/// RFC 3986 unreserved characters stay; everything else is `%XX`.
const AWS_ENCODE: &AsciiSet = &NON_ALPHANUMERIC.remove(b'-').remove(b'_').remove(b'.').remove(b'~');

#[derive(Debug, Clone)]
struct Object {
    data: Bytes,
    etag: String,
    modified: SystemTime,
}

#[derive(Debug, Default)]
struct Upload {
    key: String,
    parts: BTreeMap<u32, (String, Bytes)>,
}

/// What the server holds and how it misbehaves.
#[derive(Default)]
pub struct MockState {
    objects: Mutex<BTreeMap<String, Object>>,
    uploads: Mutex<HashMap<String, Upload>>,
    serial: AtomicU64,
    /// Most keys per list page, whatever the client asks.
    pub page_size: AtomicUsize,
    /// Answer this many next requests with 503.
    pub fail_next: AtomicUsize,
    /// Delay every request by this many milliseconds.
    pub delay_ms: AtomicUsize,
    /// Send `GET` bodies in 16 pieces with this many milliseconds between
    /// them (a slow download that never goes silent for long).
    pub trickle_ms: AtomicUsize,
    /// `METHOD kind` of every request served, in order.
    pub log: Mutex<Vec<String>>,
    pub signature_failures: AtomicUsize,
}

impl MockState {
    fn next(&self) -> u64 {
        self.serial.fetch_add(1, Ordering::SeqCst)
    }

    /// Keys stored, sorted.
    pub fn keys(&self) -> Vec<String> {
        self.objects.lock().map(|o| o.keys().cloned().collect()).unwrap_or_default()
    }

    pub fn bytes(&self, key: &str) -> Option<Vec<u8>> {
        self.objects.lock().ok()?.get(key).map(|o| o.data.to_vec())
    }

    pub fn put_raw(&self, key: &str, data: &[u8]) {
        let etag = format!("\"raw-{}\"", self.next());
        if let Ok(mut objects) = self.objects.lock() {
            objects.insert(
                key.to_owned(),
                Object {
                    data: Bytes::copy_from_slice(data),
                    etag,
                    modified: SystemTime::now(),
                },
            );
        }
    }

    /// Multipart uploads started and neither completed nor aborted.
    pub fn open_uploads(&self) -> usize {
        self.uploads.lock().map(|u| u.len()).unwrap_or(0)
    }

    /// How many served requests had `kind` in their log line.
    pub fn count(&self, kind: &str) -> usize {
        self.log
            .lock()
            .map(|log| log.iter().filter(|line| line.contains(kind)).count())
            .unwrap_or(0)
    }

    fn record(&self, line: String) {
        if let Ok(mut log) = self.log.lock() {
            log.push(line);
        }
    }
}

/// The running server. Dropping it stops it.
pub struct S3Mock {
    pub addr: SocketAddr,
    pub state: Arc<MockState>,
    runtime: Option<Runtime>,
}

impl Drop for S3Mock {
    fn drop(&mut self) {
        if let Some(runtime) = self.runtime.take() {
            runtime.shutdown_background();
        }
    }
}

impl S3Mock {
    pub fn start() -> io::Result<S3Mock> {
        let runtime = tokio::runtime::Builder::new_multi_thread()
            .worker_threads(2)
            .enable_all()
            .build()?;
        let state = Arc::new(MockState {
            page_size: AtomicUsize::new(1000),
            ..MockState::default()
        });
        let listener = runtime.block_on(tokio::net::TcpListener::bind("127.0.0.1:0"))?;
        let addr = listener.local_addr()?;
        let served = Arc::clone(&state);
        runtime.spawn(async move {
            loop {
                let Ok((stream, _)) = listener.accept().await else {
                    continue;
                };
                let state = Arc::clone(&served);
                tokio::spawn(async move {
                    let service = service_fn(move |request| handle(Arc::clone(&state), request));
                    let _ = http1::Builder::new()
                        .serve_connection(TokioIo::new(stream), service)
                        .await;
                });
            }
        });
        Ok(S3Mock {
            addr,
            state,
            runtime: Some(runtime),
        })
    }

    pub fn endpoint(&self) -> String {
        format!("http://{}", self.addr)
    }
}

// ---------------------------------------------------------------------------
// Small helpers.

fn xml_escape(text: &str) -> String {
    text.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&apos;")
}

fn xml_unescape(text: &str) -> String {
    text.replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&apos;", "'")
        .replace("&amp;", "&")
}

/// The text of every `<tag>…</tag>` in `xml`, in order.
fn elements(xml: &str, tag: &str) -> Vec<String> {
    let open = format!("<{tag}>");
    let close = format!("</{tag}>");
    let mut out = Vec::new();
    let mut rest = xml;
    while let Some(start) = rest.find(&open) {
        let after = &rest[start + open.len()..];
        let Some(end) = after.find(&close) else {
            break;
        };
        out.push(xml_unescape(&after[..end]));
        rest = &after[end + close.len()..];
    }
    out
}

fn decode(text: &str) -> String {
    percent_decode_str(text).decode_utf8_lossy().into_owned()
}

/// `application/x-www-form-urlencoded` pairs: `+` is a space, as AWS reads
/// query strings (and as `object_store` signs them).
fn query_pairs(query: &str) -> Vec<(String, String)> {
    let form = |text: &str| decode(&text.replace('+', " "));
    query
        .split('&')
        .filter(|pair| !pair.is_empty())
        .map(|pair| match pair.split_once('=') {
            Some((k, v)) => (form(k), form(v)),
            None => (form(pair), String::new()),
        })
        .collect()
}

/// Days since 1970-01-01 to (year, month, day), proleptic Gregorian.
fn civil(days: i64) -> (i64, u32, u32) {
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    (if m <= 2 { y + 1 } else { y }, m, d)
}

fn parts_of(time: SystemTime) -> (i64, u32, u32, u64, u64, u64, u64, u32) {
    let since = time.duration_since(UNIX_EPOCH).unwrap_or_default();
    let secs = since.as_secs();
    let days = i64::try_from(secs / 86_400).unwrap_or(0);
    let (y, m, d) = civil(days);
    let weekday = (days + 4).rem_euclid(7) as u64; // 1970-01-01 was a Thursday
    let rem = secs % 86_400;
    (y, m, d, rem / 3600, (rem / 60) % 60, rem % 60, weekday, since.subsec_millis())
}

/// `Sat, 10 Oct 2026 01:33:21 GMT`
fn http_date(time: SystemTime) -> String {
    const DAYS: [&str; 7] = ["Sun", "Mon", "Tue", "Wed", "Thu", "Fri", "Sat"];
    const MONTHS: [&str; 12] = ["Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec"];
    let (y, m, d, h, min, s, wd, _) = parts_of(time);
    format!(
        "{}, {d:02} {} {y} {h:02}:{min:02}:{s:02} GMT",
        DAYS[wd as usize],
        MONTHS[(m - 1) as usize]
    )
}

/// `2026-10-10T01:33:21.123Z`
fn iso_date(time: SystemTime) -> String {
    let (y, m, d, h, min, s, _, ms) = parts_of(time);
    format!("{y}-{m:02}-{d:02}T{h:02}:{min:02}:{s:02}.{ms:03}Z")
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

fn hmac(key: &[u8], data: &[u8]) -> Vec<u8> {
    match <Hmac<Sha256> as KeyInit>::new_from_slice(key) {
        Ok(mut mac) => {
            mac.update(data);
            mac.finalize().into_bytes().to_vec()
        }
        Err(_) => Vec::new(),
    }
}

type Reply = Response<BoxBody<Bytes, Infallible>>;

fn reply(status: StatusCode, body: impl Into<Bytes>) -> Reply {
    let mut response = Response::new(Full::new(body.into()).boxed());
    *response.status_mut() = status;
    response
}

/// A body sent in 16 pieces, `pause` apart.
fn trickled(status: StatusCode, body: Bytes, pause: Duration) -> Reply {
    let piece = body.len().div_ceil(16).max(1);
    let stream = futures::stream::unfold((body, true), move |(mut rest, first)| async move {
        if rest.is_empty() {
            return None;
        }
        if !first {
            tokio::time::sleep(pause).await;
        }
        let chunk = rest.split_to(piece.min(rest.len()));
        Some((Ok::<_, Infallible>(Frame::data(chunk)), (rest, false)))
    });
    let mut response = Response::new(BodyExt::boxed(StreamBody::new(stream)));
    *response.status_mut() = status;
    response
}

fn error(status: StatusCode, code: &str, message: &str) -> Reply {
    let mut response = reply(
        status,
        format!(
            "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<Error><Code>{code}</Code><Message>{}</Message></Error>",
            xml_escape(message)
        ),
    );
    response
        .headers_mut()
        .insert("content-type", hyper::header::HeaderValue::from_static("application/xml"));
    response
}

fn with_header(mut response: Reply, name: &'static str, value: &str) -> Reply {
    if let Ok(value) = hyper::header::HeaderValue::from_str(value) {
        response.headers_mut().insert(name, value);
    }
    response
}

fn object_headers(response: Reply, object: &Object, length: usize) -> Reply {
    let response = with_header(response, "etag", &object.etag);
    let response = with_header(response, "last-modified", &http_date(object.modified));
    with_header(response, "content-length", &length.to_string())
}

// ---------------------------------------------------------------------------
// Signature.

/// Checks the SigV4 signature the AWS way. `Err` is the reply to send.
fn check_signature(parts: &hyper::http::request::Parts, body: &[u8], state: &MockState) -> Result<(), Reply> {
    let refuse = |code: &str, message: &str| {
        state.signature_failures.fetch_add(1, Ordering::SeqCst);
        Err(error(StatusCode::FORBIDDEN, code, message))
    };
    let header = |name: &str| {
        parts
            .headers
            .get(name)
            .and_then(|v| v.to_str().ok())
            .map(str::to_owned)
    };
    let Some(auth) = header("authorization") else {
        return refuse("AccessDenied", "anonymous access is not allowed");
    };
    let Some(rest) = auth.strip_prefix("AWS4-HMAC-SHA256 ") else {
        return refuse("AccessDenied", "unsupported authorization");
    };
    let mut credential = "";
    let mut signed = "";
    let mut signature = "";
    for field in rest.split(',').map(str::trim) {
        if let Some(v) = field.strip_prefix("Credential=") {
            credential = v;
        } else if let Some(v) = field.strip_prefix("SignedHeaders=") {
            signed = v;
        } else if let Some(v) = field.strip_prefix("Signature=") {
            signature = v;
        }
    }
    let scope: Vec<&str> = credential.split('/').collect();
    let [key_id, date, region, service, terminal] = scope.as_slice() else {
        return refuse("AuthorizationHeaderMalformed", "bad credential scope");
    };
    if *key_id != ACCESS_KEY {
        return refuse("InvalidAccessKeyId", "The AWS Access Key Id you provided does not exist in our records.");
    }
    if *region != REGION || *service != "s3" || *terminal != "aws4_request" {
        return refuse("AuthorizationHeaderMalformed", "bad scope");
    }
    let Some(amz_date) = header("x-amz-date") else {
        return refuse("AccessDenied", "no x-amz-date");
    };
    let Some(payload) = header("x-amz-content-sha256") else {
        return refuse("AccessDenied", "no x-amz-content-sha256");
    };
    if payload != "UNSIGNED-PAYLOAD" && payload != hex(&Sha256::digest(body)) {
        return Err(error(
            StatusCode::BAD_REQUEST,
            "XAmzContentSHA256Mismatch",
            "The provided 'x-amz-content-sha256' header does not match what was computed.",
        ));
    }
    // Canonical URI: decoded, then each segment encoded once (S3 rule).
    let path = decode(parts.uri.path());
    let canonical_uri = path
        .split('/')
        .map(|segment| utf8_percent_encode(segment, AWS_ENCODE).to_string())
        .collect::<Vec<_>>()
        .join("/");
    let mut query: Vec<(String, String)> = query_pairs(parts.uri.query().unwrap_or(""))
        .into_iter()
        .map(|(k, v)| {
            (
                utf8_percent_encode(&k, AWS_ENCODE).to_string(),
                utf8_percent_encode(&v, AWS_ENCODE).to_string(),
            )
        })
        .collect();
    query.sort();
    let canonical_query = query.iter().map(|(k, v)| format!("{k}={v}")).collect::<Vec<_>>().join("&");
    let mut canonical_headers = String::new();
    for name in signed.split(';') {
        let values: Vec<String> = parts
            .headers
            .get_all(name)
            .iter()
            .filter_map(|v| v.to_str().ok())
            .map(|v| v.split_whitespace().collect::<Vec<_>>().join(" "))
            .collect();
        canonical_headers.push_str(&format!("{name}:{}\n", values.join(",")));
    }
    let canonical = format!(
        "{}\n{canonical_uri}\n{canonical_query}\n{canonical_headers}\n{signed}\n{payload}",
        parts.method.as_str()
    );
    let to_sign = format!(
        "AWS4-HMAC-SHA256\n{amz_date}\n{date}/{region}/{service}/aws4_request\n{}",
        hex(&Sha256::digest(canonical.as_bytes()))
    );
    let key = hmac(format!("AWS4{SECRET_KEY}").as_bytes(), date.as_bytes());
    let key = hmac(&key, region.as_bytes());
    let key = hmac(&key, service.as_bytes());
    let key = hmac(&key, b"aws4_request");
    if hex(&hmac(&key, to_sign.as_bytes())) != signature {
        return refuse(
            "SignatureDoesNotMatch",
            "The request signature we calculated does not match the signature you provided.",
        );
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// Requests.

async fn handle(state: Arc<MockState>, request: Request<Incoming>) -> Result<Reply, Infallible> {
    let (parts, body) = request.into_parts();
    let body = match body.collect().await {
        Ok(collected) => collected.to_bytes(),
        Err(_) => return Ok(error(StatusCode::BAD_REQUEST, "IncompleteBody", "body")),
    };
    let delay = state.delay_ms.load(Ordering::SeqCst);
    if delay > 0 {
        tokio::time::sleep(Duration::from_millis(delay as u64)).await;
    }
    if let Err(refused) = check_signature(&parts, &body, &state) {
        return Ok(refused);
    }
    if state
        .fail_next
        .fetch_update(Ordering::SeqCst, Ordering::SeqCst, |n| n.checked_sub(1))
        .is_ok()
    {
        state.record(format!("{} 503", parts.method));
        return Ok(error(StatusCode::SERVICE_UNAVAILABLE, "SlowDown", "Please reduce your request rate."));
    }
    let path = decode(parts.uri.path());
    let Some(rest) = path.strip_prefix(&format!("/{BUCKET}")) else {
        return Ok(error(StatusCode::NOT_FOUND, "NoSuchBucket", "The specified bucket does not exist"));
    };
    let key = rest.strip_prefix('/').unwrap_or(rest).to_owned();
    let query: HashMap<String, String> = query_pairs(parts.uri.query().unwrap_or("")).into_iter().collect();
    let header = |name: &str| parts.headers.get(name).and_then(|v| v.to_str().ok()).map(str::to_owned);
    let method = parts.method.clone();

    let response = if key.is_empty() {
        match (&method, query.contains_key("delete"), query.get("list-type")) {
            (&Method::GET, _, Some(_)) => {
                state.record(String::from("GET list"));
                list(&state, &query)
            }
            (&Method::POST, true, _) => {
                state.record(String::from("POST bulk-delete"));
                bulk_delete(&state, &body)
            }
            _ => error(StatusCode::NOT_IMPLEMENTED, "NotImplemented", "bucket operation"),
        }
    } else if let Some(upload_id) = query.get("uploadId") {
        match method {
            Method::PUT => {
                state.record(String::from("PUT part"));
                put_part(&state, upload_id, query.get("partNumber"), body)
            }
            Method::POST => {
                state.record(String::from("POST complete"));
                complete(&state, &key, upload_id, &body, header("if-none-match").as_deref())
            }
            Method::DELETE => {
                state.record(String::from("DELETE abort"));
                abort(&state, upload_id)
            }
            _ => error(StatusCode::NOT_IMPLEMENTED, "NotImplemented", "upload operation"),
        }
    } else if method == Method::POST && query.contains_key("uploads") {
        state.record(String::from("POST start-upload"));
        let id = format!("upload-{}", state.next());
        if let Ok(mut uploads) = state.uploads.lock() {
            uploads.insert(id.clone(), Upload { key: key.clone(), parts: BTreeMap::new() });
        }
        reply(
            StatusCode::OK,
            format!(
                "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<InitiateMultipartUploadResult><Bucket>{BUCKET}</Bucket><Key>{}</Key><UploadId>{id}</UploadId></InitiateMultipartUploadResult>",
                xml_escape(&key)
            ),
        )
    } else {
        match method {
            Method::HEAD => {
                state.record(String::from("HEAD object"));
                head(&state, &key)
            }
            Method::GET => {
                state.record(String::from("GET object"));
                get(&state, &key, header("range").as_deref())
            }
            Method::PUT => match header("x-amz-copy-source") {
                Some(source) => {
                    state.record(String::from("PUT copy"));
                    copy(&state, &source, &key)
                }
                None => {
                    state.record(String::from("PUT object"));
                    put(&state, &key, body, header("if-none-match").as_deref())
                }
            },
            Method::DELETE => {
                state.record(String::from("DELETE object"));
                if let Ok(mut objects) = state.objects.lock() {
                    objects.remove(&key);
                }
                reply(StatusCode::NO_CONTENT, Bytes::new())
            }
            _ => error(StatusCode::NOT_IMPLEMENTED, "NotImplemented", "object operation"),
        }
    };
    Ok(response)
}

fn found(state: &MockState, key: &str) -> Option<Object> {
    state.objects.lock().ok()?.get(key).cloned()
}

fn head(state: &MockState, key: &str) -> Reply {
    match found(state, key) {
        Some(object) => {
            let length = object.data.len();
            object_headers(reply(StatusCode::OK, Bytes::new()), &object, length)
        }
        None => reply(StatusCode::NOT_FOUND, Bytes::new()),
    }
}

fn get(state: &MockState, key: &str, range: Option<&str>) -> Reply {
    let Some(object) = found(state, key) else {
        return error(StatusCode::NOT_FOUND, "NoSuchKey", "The specified key does not exist.");
    };
    let len = object.data.len();
    let start = match range.and_then(|r| r.strip_prefix("bytes=")) {
        None => None,
        Some(spec) => match spec.split_once('-') {
            Some((from, "")) => from.parse::<usize>().ok(),
            _ => return error(StatusCode::NOT_IMPLEMENTED, "NotImplemented", "range form"),
        },
    };
    let pause = state.trickle_ms.load(Ordering::SeqCst);
    let body = |status: StatusCode, data: Bytes| {
        if pause > 0 {
            trickled(status, data, Duration::from_millis(pause as u64))
        } else {
            reply(status, data)
        }
    };
    match start {
        None => object_headers(body(StatusCode::OK, object.data.clone()), &object, len),
        Some(from) if from >= len => error(StatusCode::RANGE_NOT_SATISFIABLE, "InvalidRange", "range"),
        Some(from) => {
            let rest = object.data.slice(from..);
            let response = object_headers(body(StatusCode::PARTIAL_CONTENT, rest), &object, len - from);
            with_header(response, "content-range", &format!("bytes {from}-{}/{len}", len - 1))
        }
    }
}

fn store(state: &MockState, key: &str, data: Bytes) -> String {
    let etag = format!("\"{}\"", hex(&Sha256::digest(&data))[..32].to_owned());
    if let Ok(mut objects) = state.objects.lock() {
        objects.insert(
            key.to_owned(),
            Object {
                data,
                etag: etag.clone(),
                modified: SystemTime::now(),
            },
        );
    }
    etag
}

fn put(state: &MockState, key: &str, body: Bytes, if_none_match: Option<&str>) -> Reply {
    if if_none_match == Some("*") && found(state, key).is_some() {
        return error(StatusCode::PRECONDITION_FAILED, "PreconditionFailed", "At least one of the pre-conditions you specified did not hold");
    }
    let etag = store(state, key, body);
    with_header(reply(StatusCode::OK, Bytes::new()), "etag", &etag)
}

fn copy(state: &MockState, source: &str, key: &str) -> Reply {
    let source = decode(source);
    let source = source.trim_start_matches('/');
    let Some(source_key) = source.strip_prefix(&format!("{BUCKET}/")) else {
        return error(StatusCode::BAD_REQUEST, "InvalidArgument", "copy source bucket");
    };
    let Some(object) = found(state, source_key) else {
        return error(StatusCode::NOT_FOUND, "NoSuchKey", "The specified key does not exist.");
    };
    let etag = store(state, key, object.data);
    reply(
        StatusCode::OK,
        format!(
            "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<CopyObjectResult><LastModified>{}</LastModified><ETag>{}</ETag></CopyObjectResult>",
            iso_date(SystemTime::now()),
            xml_escape(&etag)
        ),
    )
}

fn put_part(state: &MockState, upload_id: &str, number: Option<&String>, body: Bytes) -> Reply {
    let Some(number) = number.and_then(|n| n.parse::<u32>().ok()) else {
        return error(StatusCode::BAD_REQUEST, "InvalidArgument", "partNumber");
    };
    let etag = format!("\"part-{}\"", state.next());
    let Ok(mut uploads) = state.uploads.lock() else {
        return error(StatusCode::INTERNAL_SERVER_ERROR, "InternalError", "lock");
    };
    let Some(upload) = uploads.get_mut(upload_id) else {
        return error(StatusCode::NOT_FOUND, "NoSuchUpload", "The specified upload does not exist.");
    };
    upload.parts.insert(number, (etag.clone(), body));
    with_header(reply(StatusCode::OK, Bytes::new()), "etag", &etag)
}

fn complete(state: &MockState, key: &str, upload_id: &str, body: &[u8], if_none_match: Option<&str>) -> Reply {
    let text = String::from_utf8_lossy(body);
    let numbers = elements(&text, "PartNumber");
    let etags = elements(&text, "ETag");
    let upload = match state.uploads.lock() {
        Ok(mut uploads) => uploads.remove(upload_id),
        Err(_) => None,
    };
    let Some(upload) = upload else {
        return error(StatusCode::NOT_FOUND, "NoSuchUpload", "The specified upload does not exist.");
    };
    if upload.key != key {
        return error(StatusCode::BAD_REQUEST, "InvalidArgument", "key");
    }
    let mut data = Vec::new();
    for (number, etag) in numbers.iter().zip(&etags) {
        let Some((stored, bytes)) = number.parse::<u32>().ok().and_then(|n| upload.parts.get(&n)) else {
            return error(StatusCode::BAD_REQUEST, "InvalidPart", "One or more of the specified parts could not be found.");
        };
        if stored != etag {
            return error(StatusCode::BAD_REQUEST, "InvalidPart", "etag");
        }
        data.extend_from_slice(bytes);
    }
    if numbers.len() != upload.parts.len() {
        return error(StatusCode::BAD_REQUEST, "InvalidPartOrder", "parts");
    }
    if if_none_match == Some("*") && found(state, key).is_some() {
        return error(StatusCode::PRECONDITION_FAILED, "PreconditionFailed", "exists");
    }
    let etag = store(state, key, Bytes::from(data));
    reply(
        StatusCode::OK,
        format!(
            "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<CompleteMultipartUploadResult><Bucket>{BUCKET}</Bucket><Key>{}</Key><ETag>{}</ETag></CompleteMultipartUploadResult>",
            xml_escape(key),
            xml_escape(&etag)
        ),
    )
}

fn abort(state: &MockState, upload_id: &str) -> Reply {
    let removed = state.uploads.lock().ok().and_then(|mut u| u.remove(upload_id));
    match removed {
        Some(_) => reply(StatusCode::NO_CONTENT, Bytes::new()),
        None => error(StatusCode::NOT_FOUND, "NoSuchUpload", "The specified upload does not exist."),
    }
}

fn bulk_delete(state: &MockState, body: &[u8]) -> Reply {
    let text = String::from_utf8_lossy(body);
    let keys = elements(&text, "Key");
    let mut out = String::from("<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<DeleteResult>");
    if let Ok(mut objects) = state.objects.lock() {
        for key in keys {
            objects.remove(&key);
            out.push_str(&format!("<Deleted><Key>{}</Key></Deleted>", xml_escape(&key)));
        }
    }
    out.push_str("</DeleteResult>");
    reply(StatusCode::OK, out)
}

fn list(state: &MockState, query: &HashMap<String, String>) -> Reply {
    let prefix = query.get("prefix").cloned().unwrap_or_default();
    let delimiter = query.get("delimiter").cloned().filter(|d| !d.is_empty());
    let after = query
        .get("continuation-token")
        .or_else(|| query.get("start-after"))
        .cloned()
        .unwrap_or_default();
    let max = query
        .get("max-keys")
        .and_then(|m| m.parse::<usize>().ok())
        .unwrap_or(1000)
        .min(state.page_size.load(Ordering::SeqCst))
        .max(1);
    let objects = match state.objects.lock() {
        Ok(objects) => objects.clone(),
        Err(_) => return error(StatusCode::INTERNAL_SERVER_ERROR, "InternalError", "lock"),
    };
    let mut contents = String::new();
    let mut prefixes = String::new();
    let mut count = 0usize;
    let mut last = String::new();
    let mut truncated = false;
    let mut last_prefix: Option<String> = None;
    for (key, object) in objects.range(prefix.clone()..) {
        if !key.starts_with(&prefix) {
            break;
        }
        if !after.is_empty() && (key.as_str() <= after.as_str() || (after.ends_with('/') && key.starts_with(&after))) {
            continue;
        }
        let rest = &key[prefix.len()..];
        let item = match delimiter.as_deref().and_then(|d| rest.find(d).map(|i| (d, i))) {
            Some((d, i)) => {
                let common = format!("{prefix}{}{d}", &rest[..i]);
                if last_prefix.as_deref() == Some(common.as_str()) {
                    continue;
                }
                Some(common)
            }
            None => None,
        };
        if count == max {
            truncated = true;
            break;
        }
        count += 1;
        match item {
            Some(common) => {
                prefixes.push_str(&format!("<CommonPrefixes><Prefix>{}</Prefix></CommonPrefixes>", xml_escape(&common)));
                last = common.clone();
                last_prefix = Some(common);
            }
            None => {
                contents.push_str(&format!(
                    "<Contents><Key>{}</Key><LastModified>{}</LastModified><ETag>{}</ETag><Size>{}</Size><StorageClass>STANDARD</StorageClass></Contents>",
                    xml_escape(key),
                    iso_date(object.modified),
                    xml_escape(&object.etag),
                    object.data.len()
                ));
                last = key.clone();
            }
        }
    }
    let token = if truncated {
        format!("<NextContinuationToken>{}</NextContinuationToken>", xml_escape(&last))
    } else {
        String::new()
    };
    reply(
        StatusCode::OK,
        format!(
            "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<ListBucketResult xmlns=\"http://s3.amazonaws.com/doc/2006-03-01/\"><Name>{BUCKET}</Name><Prefix>{}</Prefix><KeyCount>{count}</KeyCount><MaxKeys>{max}</MaxKeys><IsTruncated>{truncated}</IsTruncated>{contents}{prefixes}{token}</ListBucketResult>",
            xml_escape(&prefix)
        ),
    )
}
