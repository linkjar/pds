//! The S3-compatible blob store (`docs/SPEC.md` §8.1).
//!
//! The object names are the Reference's (`packages/aws/src/s3.ts` at the
//! pin): a blob is `blocks/<did>/<cid>`, an upload waits at
//! `tmp/<did>/<key>`, and a taken-down blob is `quarantine/<did>/<cid>`.
//!
//! The store signs each request with AWS Signature Version 4 and sends it
//! with the HTTP client of the server. Its calls block: they run on the
//! blocking pool, as the calls of the disk store do.

use std::collections::BTreeSet;
use std::fmt;
use std::future::Future;
use std::io;
use std::sync::Arc;
use std::time::{Duration, Instant};

use bytes::Bytes;
use data_encoding::HEXLOWER;
use hmac::{Hmac, KeyInit as _, Mac as _};
use reqwest::{Method, Response, StatusCode};
use sha2::{Digest as _, Sha256};
use tokio::runtime::Handle;

use crate::BlobError;

/// The size of one part of an upload: 8 MiB. An upload that is smaller is
/// one `PutObject`. S3 takes no part below 5 MiB except the last.
pub const PART_LEN: usize = 8 * 1024 * 1024;

/// How often a request is sent before its failure is the answer.
const ATTEMPTS: u32 = 3;

/// The SHA-256 of no bytes.
const EMPTY_HASH: &str = "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855";

/// The time in milliseconds since the epoch. Requests are signed with it.
pub type Clock = Arc<dyn Fn() -> i64 + Send + Sync>;

/// The configuration of the store: the `PDS_BLOBSTORE_S3_*` variables.
#[derive(Clone, PartialEq, Eq)]
pub struct S3Config {
    /// `PDS_BLOBSTORE_S3_BUCKET`.
    pub bucket: String,
    /// `PDS_BLOBSTORE_S3_REGION`. Cloudflare R2 takes `auto`.
    pub region: String,
    /// `PDS_BLOBSTORE_S3_ENDPOINT`. Without it the store is AWS S3 in the
    /// region.
    pub endpoint: Option<String>,
    /// `PDS_BLOBSTORE_S3_FORCE_PATH_STYLE`: the bucket is the first segment
    /// of the path and not a label of the host.
    pub force_path_style: bool,
    /// `PDS_BLOBSTORE_S3_ACCESS_KEY_ID`.
    pub access_key_id: String,
    /// `PDS_BLOBSTORE_S3_SECRET_ACCESS_KEY`.
    pub secret_access_key: String,
    /// `AWS_SESSION_TOKEN`, for temporary credentials.
    pub session_token: Option<String>,
    /// `PDS_BLOBSTORE_S3_UPLOAD_TIMEOUT_MS`: the limit for one upload from
    /// its first byte to its last.
    pub upload_timeout: Duration,
    /// `PDS_BLOBSTORE_S3_REQUEST_TIMEOUT_MS`: how long a request may stall.
    pub request_timeout: Option<Duration>,
}

impl fmt::Debug for S3Config {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("S3Config")
            .field("bucket", &self.bucket)
            .field("region", &self.region)
            .field("endpoint", &self.endpoint)
            .field("force_path_style", &self.force_path_style)
            .field("access_key_id", &self.access_key_id)
            .field("upload_timeout", &self.upload_timeout)
            .field("request_timeout", &self.request_timeout)
            .finish_non_exhaustive()
    }
}

impl S3Config {
    /// How long a request may stall. The default is the Reference's: the
    /// upload timeout, at most 15 seconds and at least 6.
    pub fn stall_timeout(&self) -> Duration {
        self.request_timeout.unwrap_or_else(|| {
            self.upload_timeout
                .min(Duration::from_secs(15))
                .max(Duration::from_secs(6))
        })
    }
}

/// Percent-encodes as Signature Version 4 requires: every byte except the
/// unreserved characters, and `/` when `keep_slash` is set.
fn uri_encode(text: &str, keep_slash: bool) -> String {
    let mut out = String::with_capacity(text.len());
    for byte in text.bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'.' | b'_' | b'~' => {
                out.push(char::from(byte));
            }
            b'/' if keep_slash => out.push('/'),
            _ => {
                out.push('%');
                out.push_str(&data_encoding::HEXUPPER.encode(&[byte]));
            }
        }
    }
    out
}

fn sha256_hex(bytes: &[u8]) -> String {
    let digest: [u8; 32] = Sha256::digest(bytes).into();
    HEXLOWER.encode(&digest)
}

fn hmac(key: &[u8], message: &str) -> [u8; 32] {
    // HMAC takes a key of any length, so there is no other arm.
    let Ok(mut mac) = Hmac::<Sha256>::new_from_slice(key) else {
        return [0; 32];
    };
    mac.update(message.as_bytes());
    mac.finalize().into_bytes().into()
}

/// The query string in canonical form: sorted, each name and value encoded.
fn canonical_query(query: &[(&str, &str)]) -> String {
    let mut pairs: Vec<(String, String)> = query
        .iter()
        .map(|(name, value)| (uri_encode(name, false), uri_encode(value, false)))
        .collect();
    pairs.sort();
    let pairs: Vec<String> = pairs
        .into_iter()
        .map(|(name, value)| format!("{name}={value}"))
        .collect();
    pairs.join("&")
}

/// What signs a request.
struct Signer<'a> {
    access_key_id: &'a str,
    secret_access_key: &'a str,
    region: &'a str,
}

impl Signer<'_> {
    /// The `Authorization` header of a request. `path` is encoded,
    /// `headers` are every header that is signed, with lower-case names,
    /// and `amz_date` is `YYYYMMDDTHHMMSSZ`.
    fn authorization(
        &self,
        method: &str,
        path: &str,
        query: &str,
        headers: &[(&str, &str)],
        payload_hash: &str,
        amz_date: &str,
    ) -> String {
        let mut headers = headers.to_vec();
        headers.sort_unstable();
        let mut canonical_headers = String::new();
        for (name, value) in &headers {
            canonical_headers.push_str(name);
            canonical_headers.push(':');
            canonical_headers.push_str(value.trim());
            canonical_headers.push('\n');
        }
        let names: Vec<&str> = headers.iter().map(|(name, _)| *name).collect();
        let signed = names.join(";");
        let request =
            format!("{method}\n{path}\n{query}\n{canonical_headers}\n{signed}\n{payload_hash}");
        let date = amz_date.get(..8).unwrap_or_default();
        let scope = format!("{date}/{}/s3/aws4_request", self.region);
        let to_sign = format!(
            "AWS4-HMAC-SHA256\n{amz_date}\n{scope}\n{}",
            sha256_hex(request.as_bytes())
        );
        let mut key = hmac(format!("AWS4{}", self.secret_access_key).as_bytes(), date);
        for part in [self.region, "s3", "aws4_request"] {
            key = hmac(&key, part);
        }
        let signature = HEXLOWER.encode(&hmac(&key, &to_sign));
        format!(
            "AWS4-HMAC-SHA256 Credential={}/{scope},SignedHeaders={signed},Signature={signature}",
            self.access_key_id
        )
    }
}

/// The text of the first `<name>` element.
fn element<'a>(xml: &'a str, name: &str) -> Option<&'a str> {
    elements(xml, name).next()
}

/// The text of every `<name>` element, in order.
fn elements<'a>(xml: &'a str, name: &str) -> impl Iterator<Item = &'a str> {
    let open = format!("<{name}>");
    let close = format!("</{name}>");
    let mut rest = xml;
    std::iter::from_fn(move || {
        let start = rest.find(&open)? + open.len();
        let after = rest.get(start..)?;
        let end = after.find(&close)?;
        rest = after.get(end + close.len()..)?;
        after.get(..end)
    })
}

fn unescape(text: &str) -> String {
    text.replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&apos;", "'")
        .replace("&amp;", "&")
}

struct Inner {
    config: S3Config,
    client: reqwest::Client,
    handle: Handle,
    clock: Clock,
    /// `https://host[:port]`.
    origin: String,
    /// The `Host` header.
    host: String,
    /// `/<bucket>` in path style, and nothing otherwise.
    base_path: String,
}

/// The S3-compatible blob store.
#[derive(Clone)]
pub struct S3BlobStore {
    inner: Arc<Inner>,
}

impl fmt::Debug for S3BlobStore {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("S3BlobStore")
            .field("config", &self.inner.config)
            .field("origin", &self.inner.origin)
            .finish_non_exhaustive()
    }
}

/// One request to the store.
struct Call<'a> {
    method: Method,
    /// The object, or the bucket when there is none.
    object: Option<&'a str>,
    query: &'a [(&'a str, &'a str)],
    /// `x-amz-copy-source`, encoded.
    copy_source: Option<&'a str>,
    body: Bytes,
    /// The limit for the whole request. A read has none: the client paces
    /// it.
    limit: Option<Duration>,
}

impl<'a> Call<'a> {
    fn new(method: Method, object: &'a str) -> Self {
        Self {
            method,
            object: Some(object),
            query: &[],
            copy_source: None,
            body: Bytes::new(),
            limit: None,
        }
    }
}

fn remote(object: &str, message: impl fmt::Display) -> BlobError {
    BlobError::Remote {
        object: object.to_owned(),
        message: message.to_string(),
    }
}

/// The failure that a response states.
async fn refusal(object: &str, response: Response) -> BlobError {
    let status = response.status();
    let body = response.text().await.unwrap_or_default();
    let code = element(&body, "Code").unwrap_or_default();
    let message = element(&body, "Message").unwrap_or_default();
    remote(object, format!("HTTP {} {code} {message}", status.as_u16()))
}

impl S3BlobStore {
    /// The store that `config` names. Its calls run on `handle`.
    pub fn new(config: S3Config, handle: Handle, clock: Clock) -> Result<Self, BlobError> {
        let endpoint = config
            .endpoint
            .clone()
            .unwrap_or_else(|| format!("https://s3.{}.amazonaws.com", config.region));
        let bad = |message: &str| remote(&endpoint, message);
        let url = reqwest::Url::parse(&endpoint).map_err(|error| remote(&endpoint, error))?;
        let name = url.host_str().ok_or_else(|| bad("no host"))?;
        // A bucket cannot be a label before an address.
        let literal = name == "localhost" || name.parse::<std::net::IpAddr>().is_ok();
        let path_style =
            config.force_path_style || literal || matches!(url.host(), Some(url::Host::Ipv6(_)));
        let name = if path_style {
            name.to_owned()
        } else {
            format!("{}.{name}", config.bucket)
        };
        let host = url
            .port()
            .map_or_else(|| name.clone(), |port| format!("{name}:{port}"));
        let base_path = if path_style {
            format!("/{}", uri_encode(&config.bucket, false))
        } else {
            String::new()
        };
        let client = reqwest::Client::builder()
            .connect_timeout(Duration::from_secs(5))
            .read_timeout(config.stall_timeout())
            .redirect(reqwest::redirect::Policy::none())
            .build()
            .map_err(|error| remote(&endpoint, error))?;
        Ok(Self {
            inner: Arc::new(Inner {
                origin: format!("{}://{host}", url.scheme()),
                host,
                base_path,
                config,
                client,
                handle,
                clock,
            }),
        })
    }

    fn wait<T>(&self, call: impl Future<Output = T>) -> T {
        self.inner.handle.block_on(call)
    }

    fn amz_date(&self) -> String {
        chrono::DateTime::from_timestamp_millis((self.inner.clock)())
            .unwrap_or_default()
            .format("%Y%m%dT%H%M%SZ")
            .to_string()
    }

    fn path(&self, object: Option<&str>) -> String {
        let base = &self.inner.base_path;
        match object {
            Some(object) => format!("{base}/{}", uri_encode(object, true)),
            None if base.is_empty() => "/".to_owned(),
            None => base.clone(),
        }
    }

    /// Sends one attempt of a call.
    async fn attempt(&self, call: &Call<'_>) -> reqwest::Result<Response> {
        let inner = &*self.inner;
        let path = self.path(call.object);
        let query = canonical_query(call.query);
        let payload_hash = if call.body.is_empty() {
            EMPTY_HASH.to_owned()
        } else {
            sha256_hex(&call.body)
        };
        let amz_date = self.amz_date();
        let mut headers = vec![
            ("host", inner.host.as_str()),
            ("x-amz-content-sha256", payload_hash.as_str()),
            ("x-amz-date", amz_date.as_str()),
        ];
        if let Some(source) = call.copy_source {
            headers.push(("x-amz-copy-source", source));
        }
        if let Some(token) = &inner.config.session_token {
            headers.push(("x-amz-security-token", token));
        }
        let authorization = Signer {
            access_key_id: &inner.config.access_key_id,
            secret_access_key: &inner.config.secret_access_key,
            region: &inner.config.region,
        }
        .authorization(
            call.method.as_str(),
            &path,
            &query,
            &headers,
            &payload_hash,
            &amz_date,
        );
        let mut url = format!("{}{path}", inner.origin);
        if !query.is_empty() {
            url.push('?');
            url.push_str(&query);
        }
        let mut request = inner.client.request(call.method.clone(), url);
        // The client sets `host` from the URL.
        for (name, value) in headers.into_iter().skip(1) {
            request = request.header(name, value);
        }
        request = request.header("authorization", authorization);
        if call.method == Method::PUT || call.method == Method::POST {
            request = request.body(call.body.clone());
        }
        if let Some(limit) = call.limit {
            request = request.timeout(limit);
        }
        request.send().await
    }

    /// Sends a call. A failure to connect and an answer that says the
    /// store is busy are tried again.
    async fn send(&self, call: &Call<'_>) -> Result<Response, BlobError> {
        let object = call.object.unwrap_or(&self.inner.config.bucket);
        let mut attempt = 1;
        loop {
            let failure = match self.attempt(call).await {
                Ok(response) => {
                    let busy = matches!(response.status().as_u16(), 429 | 500 | 502 | 503 | 504);
                    if !busy || attempt == ATTEMPTS {
                        return Ok(response);
                    }
                    format!("HTTP {}", response.status().as_u16())
                }
                Err(error) if error.is_timeout() && call.limit.is_some() => {
                    return Err(remote(object, "Blob upload timed out"));
                }
                Err(error) if attempt == ATTEMPTS => return Err(remote(object, error)),
                Err(error) => error.to_string(),
            };
            tracing::warn!(%object, attempt, %failure, "a request to the blob store is tried again");
            tokio::time::sleep(Duration::from_millis(100 * u64::from(attempt * attempt))).await;
            attempt += 1;
        }
    }

    /// Sends a call that answers 200 or 204, and returns the body.
    async fn done(&self, call: &Call<'_>) -> Result<String, BlobError> {
        let object = call.object.unwrap_or(&self.inner.config.bucket);
        let response = self.send(call).await?;
        if !response.status().is_success() {
            return Err(refusal(object, response).await);
        }
        response.text().await.map_err(|error| remote(object, error))
    }

    /// Copies an object and removes the source.
    async fn move_object(&self, from: &str, to: &str) -> Result<(), BlobError> {
        let source = uri_encode(&format!("{}/{from}", self.inner.config.bucket), true);
        let mut copy = Call::new(Method::PUT, to);
        copy.copy_source = Some(&source);
        let response = self.send(&copy).await?;
        if response.status() == StatusCode::NOT_FOUND {
            return Err(BlobError::NotFound);
        }
        if !response.status().is_success() {
            return Err(refusal(to, response).await);
        }
        // A copy can answer 200 and state an error in the body.
        let body = response.text().await.map_err(|error| remote(to, error))?;
        if body.contains("<Error>") {
            let code = element(&body, "Code").unwrap_or_default();
            return Err(remote(to, format!("copy failed: {code}")));
        }
        self.done(&Call::new(Method::DELETE, from)).await?;
        Ok(())
    }

    async fn head(&self, object: &str) -> Result<bool, BlobError> {
        let response = self.send(&Call::new(Method::HEAD, object)).await?;
        match response.status() {
            StatusCode::OK => Ok(true),
            StatusCode::NOT_FOUND => Ok(false),
            status => Err(remote(object, format!("HTTP {}", status.as_u16()))),
        }
    }

    /// Starts an upload to `tmp/<did>/<key>`.
    pub(crate) fn stage(&self, did: &str, key: &str) -> Upload {
        Upload {
            store: self.clone(),
            object: format!("tmp/{did}/{key}"),
            buffer: Vec::new(),
            multipart: None,
            parts: Vec::new(),
            deadline: Instant::now() + self.inner.config.upload_timeout,
            done: false,
        }
    }

    /// Moves a staged upload to its place as the blob `cid`. A staged
    /// object that is gone is no error when the blob is in place.
    pub(crate) fn make_permanent(&self, did: &str, key: &str, cid: &str) -> Result<(), BlobError> {
        let from = format!("tmp/{did}/{key}");
        let to = format!("blocks/{did}/{cid}");
        self.wait(async {
            match self.move_object(&from, &to).await {
                Err(BlobError::NotFound) if self.head(&to).await? => Ok(()),
                Err(BlobError::NotFound) => Err(remote(&from, "the staged upload is gone")),
                other => other,
            }
        })
    }

    pub(crate) fn delete(&self, did: &str, cid: &str) -> Result<(), BlobError> {
        let object = format!("blocks/{did}/{cid}");
        self.wait(self.done(&Call::new(Method::DELETE, &object)))
            .map(drop)
    }

    pub(crate) fn delete_staged(&self, did: &str, key: &str) -> Result<(), BlobError> {
        let object = format!("tmp/{did}/{key}");
        self.wait(self.done(&Call::new(Method::DELETE, &object)))
            .map(drop)
    }

    pub(crate) fn has(&self, did: &str, cid: &str) -> Result<bool, BlobError> {
        self.wait(self.head(&format!("blocks/{did}/{cid}")))
    }

    pub(crate) fn open(&self, did: &str, cid: &str) -> Result<Reader, BlobError> {
        let object = format!("blocks/{did}/{cid}");
        let response = self.wait(self.send(&Call::new(Method::GET, &object)))?;
        match response.status() {
            StatusCode::OK => Ok(Reader {
                handle: self.inner.handle.clone(),
                response,
                chunk: Bytes::new(),
            }),
            StatusCode::NOT_FOUND => Err(BlobError::NotFound),
            _ => Err(self.wait(refusal(&object, response))),
        }
    }

    /// The CID of every blob the store holds for `did`.
    pub(crate) fn stored(&self, did: &str) -> Result<BTreeSet<String>, BlobError> {
        let prefix = format!("blocks/{did}/");
        let mut found = BTreeSet::new();
        let mut token: Option<String> = None;
        loop {
            let mut query = vec![("list-type", "2"), ("prefix", prefix.as_str())];
            if let Some(token) = &token {
                query.push(("continuation-token", token));
            }
            let mut call = Call::new(Method::GET, "");
            call.object = None;
            call.query = &query;
            let body = self.wait(self.done(&call))?;
            for key in elements(&body, "Key") {
                if let Some(cid) = unescape(key).strip_prefix(&prefix) {
                    found.insert(cid.to_owned());
                }
            }
            token = element(&body, "NextContinuationToken").map(unescape);
            if token.is_none() {
                return Ok(found);
            }
        }
    }
}

/// An upload in progress. Its bytes go to the store in parts of
/// [`PART_LEN`]; the store keeps none of them when the value is dropped
/// before [`Upload::finish`].
pub(crate) struct Upload {
    store: S3BlobStore,
    object: String,
    buffer: Vec<u8>,
    multipart: Option<String>,
    /// The `ETag` of each part sent.
    parts: Vec<String>,
    deadline: Instant,
    done: bool,
}

impl fmt::Debug for Upload {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Upload")
            .field("object", &self.object)
            .field("parts", &self.parts.len())
            .finish_non_exhaustive()
    }
}

impl Upload {
    fn remaining(&self) -> Result<Duration, BlobError> {
        match self.deadline.checked_duration_since(Instant::now()) {
            Some(left) if !left.is_zero() => Ok(left),
            _ => Err(remote(&self.object, "Blob upload timed out")),
        }
    }

    fn call(&self, method: Method) -> Result<Call<'_>, BlobError> {
        let mut call = Call::new(method, &self.object);
        call.limit = Some(self.remaining()?);
        Ok(call)
    }

    fn send_part(&mut self) -> Result<(), BlobError> {
        let store = self.store.clone();
        if self.multipart.is_none() {
            let mut call = self.call(Method::POST)?;
            call.query = &[("uploads", "")];
            let body = store.wait(store.done(&call))?;
            let id = element(&body, "UploadId")
                .ok_or_else(|| remote(&self.object, "no UploadId in the answer"))?;
            self.multipart = Some(unescape(id));
        }
        let id = self.multipart.clone().unwrap_or_default();
        let number = (self.parts.len() + 1).to_string();
        let query = [("partNumber", number.as_str()), ("uploadId", id.as_str())];
        let body = Bytes::from(std::mem::take(&mut self.buffer));
        let mut call = self.call(Method::PUT)?;
        call.query = &query;
        call.body = body;
        let etag = store.wait(async {
            let response = store.send(&call).await?;
            if !response.status().is_success() {
                return Err(refusal(&self.object, response).await);
            }
            response
                .headers()
                .get("etag")
                .and_then(|value| value.to_str().ok())
                .map(str::to_owned)
                .ok_or_else(|| remote(&self.object, "a part was stored without an ETag"))
        })?;
        self.parts.push(etag);
        Ok(())
    }

    pub(crate) fn write(&mut self, mut bytes: &[u8]) -> Result<(), BlobError> {
        while !bytes.is_empty() {
            let room = PART_LEN - self.buffer.len();
            let (now, later) = bytes.split_at(room.min(bytes.len()));
            self.buffer.extend_from_slice(now);
            bytes = later;
            if self.buffer.len() == PART_LEN {
                self.send_part()?;
            }
        }
        Ok(())
    }

    pub(crate) fn finish(&mut self) -> Result<(), BlobError> {
        let store = self.store.clone();
        if self.multipart.is_none() {
            let body = Bytes::from(std::mem::take(&mut self.buffer));
            let mut call = self.call(Method::PUT)?;
            call.body = body;
            store.wait(store.done(&call))?;
            self.done = true;
            return Ok(());
        }
        if !self.buffer.is_empty() {
            self.send_part()?;
        }
        let parts: Vec<String> = self
            .parts
            .iter()
            .enumerate()
            .map(|(index, etag)| {
                let etag = etag.replace('&', "&amp;").replace('"', "&quot;");
                let number = index + 1;
                format!("<Part><PartNumber>{number}</PartNumber><ETag>{etag}</ETag></Part>")
            })
            .collect();
        let list = format!(
            "<CompleteMultipartUpload>{}</CompleteMultipartUpload>",
            parts.concat()
        );
        let id = self.multipart.clone().unwrap_or_default();
        let query = [("uploadId", id.as_str())];
        let mut call = self.call(Method::POST)?;
        call.query = &query;
        call.body = Bytes::from(list);
        let body = store.wait(store.done(&call))?;
        // The answer can be 200 and state an error in the body.
        if body.contains("<Error>") {
            let code = element(&body, "Code").unwrap_or_default();
            return Err(remote(&self.object, format!("upload failed: {code}")));
        }
        self.done = true;
        Ok(())
    }
}

impl Drop for Upload {
    fn drop(&mut self) {
        let Some(id) = self.multipart.take().filter(|_| !self.done) else {
            return;
        };
        // The value may be dropped where blocking is not allowed, so the
        // store is told in a task of its own.
        let store = self.store.clone();
        let object = std::mem::take(&mut self.object);
        self.store.inner.handle.spawn(async move {
            let query = [("uploadId", id.as_str())];
            let mut call = Call::new(Method::DELETE, &object);
            call.query = &query;
            if let Err(error) = store.done(&call).await {
                tracing::warn!(%object, %error, "an unfinished upload was not removed from the blob store");
            }
        });
    }
}

/// A stored blob that is read from the store as it is consumed.
pub(crate) struct Reader {
    handle: Handle,
    response: Response,
    chunk: Bytes,
}

impl io::Read for Reader {
    fn read(&mut self, out: &mut [u8]) -> io::Result<usize> {
        if self.chunk.is_empty() {
            match self.handle.block_on(self.response.chunk()) {
                Ok(Some(chunk)) => self.chunk = chunk,
                Ok(None) => return Ok(0),
                Err(error) => return Err(io::Error::other(error)),
            }
        }
        let take = out.len().min(self.chunk.len());
        let head = self.chunk.split_to(take);
        out.get_mut(..take)
            .unwrap_or_default()
            .copy_from_slice(&head);
        Ok(take)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SIGNER: Signer<'static> = Signer {
        access_key_id: "AKIAIOSFODNN7EXAMPLE",
        secret_access_key: "wJalrXUtnFEMI/K7MDENG/bPxRfiCYEXAMPLEKEY",
        region: "us-east-1",
    };
    const DATE: &str = "20130524T000000Z";

    fn signature(authorization: &str) -> &str {
        authorization.rsplit("Signature=").next().unwrap()
    }

    /// The examples of "Authenticating Requests: Using the Authorization
    /// Header (AWS Signature Version 4)" in the Amazon S3 API reference.
    #[test]
    fn the_signatures_are_those_of_the_s3_reference() {
        let get = SIGNER.authorization(
            "GET",
            "/test.txt",
            "",
            &[
                ("host", "examplebucket.s3.amazonaws.com"),
                ("range", "bytes=0-9"),
                ("x-amz-content-sha256", EMPTY_HASH),
                ("x-amz-date", DATE),
            ],
            EMPTY_HASH,
            DATE,
        );
        assert_eq!(
            get,
            "AWS4-HMAC-SHA256 Credential=AKIAIOSFODNN7EXAMPLE/20130524/us-east-1/s3/aws4_request,\
             SignedHeaders=host;range;x-amz-content-sha256;x-amz-date,\
             Signature=f0e8bdb87c964420e857bd35b5d6ed310bd44f0170aba48dd91039c6036bdb41"
        );

        let headers = [
            ("host", "examplebucket.s3.amazonaws.com"),
            ("x-amz-content-sha256", EMPTY_HASH),
            ("x-amz-date", DATE),
        ];
        let lifecycle = SIGNER.authorization(
            "GET",
            "/",
            &canonical_query(&[("lifecycle", "")]),
            &headers,
            EMPTY_HASH,
            DATE,
        );
        assert_eq!(
            signature(&lifecycle),
            "fea454ca298b7da1c68078a5d1bdbfbbe0d65c699e0f91ac7a200a0136783543"
        );
        let list = SIGNER.authorization(
            "GET",
            "/",
            &canonical_query(&[("prefix", "J"), ("max-keys", "2")]),
            &headers,
            EMPTY_HASH,
            DATE,
        );
        assert_eq!(
            signature(&list),
            "34b48302e7b5fa45bde8084f4b7868a86f0a534bc59db6670ed5711ef69dc6f7"
        );
    }

    #[test]
    fn a_name_is_encoded_once() {
        assert_eq!(
            uri_encode("blocks/did:plc:abc/bafkrei", true),
            "blocks/did%3Aplc%3Aabc/bafkrei"
        );
        assert_eq!(uri_encode("a/b c~", false), "a%2Fb%20c~");
        assert_eq!(sha256_hex(b""), EMPTY_HASH);
    }

    #[test]
    fn a_listing_is_read() {
        let xml = "<ListBucketResult><IsTruncated>true</IsTruncated>\
                   <Contents><Key>blocks/did:plc:a/one</Key></Contents>\
                   <Contents><Key>blocks/did:plc:a/t&amp;wo</Key></Contents>\
                   <NextContinuationToken>abc</NextContinuationToken></ListBucketResult>";
        let keys: Vec<String> = elements(xml, "Key").map(unescape).collect();
        assert_eq!(keys, ["blocks/did:plc:a/one", "blocks/did:plc:a/t&wo"]);
        assert_eq!(element(xml, "NextContinuationToken"), Some("abc"));
        assert_eq!(element(xml, "UploadId"), None);
    }
}
