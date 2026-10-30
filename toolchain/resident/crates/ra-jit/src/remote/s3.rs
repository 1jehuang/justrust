//! A bucket of an S3 service and a prefix in it, through the AWS SDK.
//!
//! The SDK finds the credentials (environment, shared config and profile, web identity,
//! container and instance roles), signs, and tries a request again with a backoff when the
//! connection or the service fails it. What is here is what a store of packs needs on top of
//! that: a body read from any offset, an upload in parts, and a write that only lands if
//! nobody else wrote first.

use std::{
    path::Path,
    sync::{Arc, atomic::Ordering},
    time::{Duration, SystemTime},
};

use anyhow::{Context, bail};
use aws_config::{BehaviorVersion, Region, timeout::TimeoutConfig};
use aws_sdk_s3::{
    Client,
    config::{
        Builder, ConfigBag, Intercept, RuntimeComponents,
        interceptors::BeforeTransmitInterceptorContextRef,
    },
    error::{BoxError, ProvideErrorMetadata, SdkError},
    primitives::{ByteStream, Length},
    types::{CompletedMultipartUpload, CompletedPart},
};
use aws_smithy_http_client::{Builder as HttpClients, tls};
use futures_util::{StreamExt, TryStreamExt, stream};
use url::Url;

use super::transfer::{Object, Refused, Traffic, Version};

/// A file of at least this many bytes is sent in parts.
const MULTIPART_AT: u64 = 16 << 20;
/// The size of a part: S3 takes none below 5 MiB but the last.
const PART: u64 = 8 << 20;
/// An upload is given up when it is slower than this for the whole of an attempt.
const SLOWEST_UPLOAD: u64 = 64 << 10;

pub(crate) struct S3 {
    client: Client,
    bucket: String,
    /// With a `/` at its end, or empty.
    prefix: String,
    /// How long one attempt at a request may take before its body.
    patience: Duration,
    traffic: Arc<Traffic>,
}

/// What the answer to a conditional write was.
pub(crate) enum Written {
    Done,
    /// Somebody else wrote since the version was read.
    Lost,
}

impl S3 {
    /// `s3://bucket/prefix?endpoint=URL&region=NAME&profile=NAME&path-style=true|false`.
    /// What the address leaves out is what the AWS tools read: `AWS_ENDPOINT_URL`,
    /// `AWS_REGION`, `AWS_PROFILE` and the shared config files. A service at an endpoint of
    /// its own is addressed by path unless told otherwise, as most that are not AWS need.
    pub(crate) async fn open(
        spec: &Url,
        patience: Duration,
        traffic: Arc<Traffic>,
    ) -> anyhow::Result<S3> {
        let bucket = spec.host_str().context("the address names no bucket")?.to_owned();
        let prefix = match spec.path().trim_matches('/') {
            "" => String::new(),
            prefix => format!("{prefix}/"),
        };
        let (mut endpoint, mut path_style, mut region) = (None, None, None);
        let timeouts = TimeoutConfig::builder()
            .connect_timeout(patience.min(Duration::from_secs(5)))
            .read_timeout(patience)
            .operation_attempt_timeout(patience)
            .build();
        let tls = tls::Provider::Rustls(tls::rustls_provider::CryptoMode::Ring);
        let mut loader = aws_config::defaults(BehaviorVersion::latest())
            .timeout_config(timeouts)
            .http_client(HttpClients::new().tls_provider(tls).build_https());
        for (name, value) in spec.query_pairs() {
            match &*name {
                "endpoint" => endpoint = Some(value.into_owned()),
                "region" => region = Some(Region::new(value.into_owned())),
                "profile" => loader = loader.profile_name(value.into_owned()),
                "path-style" => {
                    path_style = Some(value.parse().context("path-style is true or false")?)
                }
                _ => bail!("`{name}` is not an option of an S3 address"),
            }
        }
        let shared = loader.load().await;
        let own_endpoint = endpoint.is_some() || shared.endpoint_url().is_some();
        let mut config = Builder::from(&shared)
            .force_path_style(path_style.unwrap_or(own_endpoint))
            .interceptor(CountAttempts(traffic.clone()));
        if let Some(endpoint) = endpoint {
            config = config.endpoint_url(endpoint);
        }
        // Signing needs a region, and a service that is not AWS does not care which.
        let anywhere = || Region::new("us-east-1");
        config =
            config.region(region.or_else(|| shared.region().cloned()).unwrap_or_else(anywhere));
        Ok(S3 { client: Client::from_conf(config.build()), bucket, prefix, patience, traffic })
    }

    /// The bucket and the prefix, which is what the lists of this remote are remembered
    /// under: the same bucket reached another way is the same remote.
    pub(crate) fn place(&self) -> String {
        format!("s3://{}/{}", self.bucket, self.prefix)
    }

    fn key(&self, name: &str) -> String {
        format!("{}{name}", self.prefix)
    }

    /// The object under `name` from `offset` on, or `None` when there is no such object.
    /// `offset` at or past its end is an object with nothing left.
    pub(crate) async fn read_from(
        &self,
        name: &str,
        offset: u64,
    ) -> anyhow::Result<Option<ByteStream>> {
        self.traffic.operations.fetch_add(1, Ordering::Relaxed);
        let mut request = self.client.get_object().bucket(&self.bucket).key(self.key(name));
        if offset != 0 {
            request = request.range(format!("bytes={offset}-"));
        }
        match request.send().await {
            Ok(object) => Ok(Some(object.body)),
            Err(e) if e.as_service_error().is_some_and(|it| it.is_no_such_key()) => Ok(None),
            Err(e) if status(&e) == Some(416) => Ok(Some(ByteStream::from_static(b""))),
            Err(e) => Err(failed(e)).with_context(|| format!("fetching {name}")),
        }
    }

    /// The whole of a small object and the version it was read at, when it is at most
    /// `most` bytes. How long the service says it is is not believed: the bytes are counted.
    pub(crate) async fn read_small(
        &self,
        name: &str,
        most: u64,
    ) -> anyhow::Result<Option<(Vec<u8>, Version)>> {
        self.traffic.operations.fetch_add(1, Ordering::Relaxed);
        let request = self.client.get_object().bucket(&self.bucket).key(self.key(name));
        let object = match request.send().await {
            Ok(object) => object,
            Err(e) if e.as_service_error().is_some_and(|it| it.is_no_such_key()) => {
                return Ok(None);
            }
            Err(e) => return Err(failed(e)).with_context(|| format!("fetching {name}")),
        };
        let version = Version(object.e_tag);
        let body = super::read_at_most(super::Body::S3(object.body), most, name);
        let bytes = match tokio::time::timeout(self.patience, body).await {
            Ok(bytes) => bytes?,
            Err(_) => bail!("fetching {name}: no bytes for {:?}", self.patience),
        };
        self.traffic.received.fetch_add(bytes.len() as u64, Ordering::Relaxed);
        Ok(Some((bytes, version)))
    }

    /// The object under `name` as a listing would have it, if there is one.
    pub(crate) async fn found(&self, name: &str) -> anyhow::Result<Option<Object>> {
        self.traffic.operations.fetch_add(1, Ordering::Relaxed);
        match self.client.head_object().bucket(&self.bucket).key(self.key(name)).send().await {
            Ok(found) => Ok(Some(Object {
                name: name.to_owned(),
                len: found.content_length.and_then(|it| u64::try_from(it).ok()).unwrap_or(0),
                written: written_at(found.last_modified),
            })),
            Err(e) if e.as_service_error().is_some_and(|it| it.is_not_found()) => Ok(None),
            Err(e) => Err(failed(e)).with_context(|| format!("looking for {name}")),
        }
    }

    /// The objects whose names start with `prefix`.
    pub(crate) async fn list(&self, prefix: &str) -> anyhow::Result<Vec<Object>> {
        let (mut objects, mut next) = (Vec::new(), None);
        loop {
            self.traffic.operations.fetch_add(1, Ordering::Relaxed);
            let listing = self.client.list_objects_v2().bucket(&self.bucket);
            let page = listing.prefix(self.key(prefix)).set_continuation_token(next).send().await;
            let page = page.map_err(failed).with_context(|| format!("listing {prefix}"))?;
            objects.extend(page.contents().iter().filter_map(|it| {
                Some(Object {
                    name: it.key()?.strip_prefix(&self.prefix)?.to_owned(),
                    len: it.size().and_then(|it| u64::try_from(it).ok()).unwrap_or(0),
                    written: written_at(it.last_modified),
                })
            }));
            next = page.next_continuation_token().map(str::to_owned);
            if next.is_none() {
                return Ok(objects);
            }
        }
    }

    /// Removes the object under `name`. One that is not there is removed.
    pub(crate) async fn delete(&self, name: &str) -> anyhow::Result<()> {
        self.traffic.operations.fetch_add(1, Ordering::Relaxed);
        let sent =
            self.client.delete_object().bucket(&self.bucket).key(self.key(name)).send().await;
        sent.map(drop).map_err(failed).with_context(|| format!("removing {name}"))
    }

    /// Writes a small object if the one there is still the version that was read: `None`
    /// when none was there.
    pub(crate) async fn write_small(
        &self,
        name: &str,
        bytes: &[u8],
        read_at: Option<&Version>,
    ) -> anyhow::Result<Written> {
        self.traffic.operations.fetch_add(1, Ordering::Relaxed);
        let len = bytes.len() as u64;
        let request = self.client.put_object().bucket(&self.bucket).key(self.key(name));
        let request = match read_at.and_then(|it| it.0.as_deref()) {
            Some(tag) => request.if_match(tag),
            None => request.if_none_match("*"),
        };
        match request.body(ByteStream::from(bytes.to_vec())).send().await {
            Ok(_) => {
                self.traffic.sent.fetch_add(len, Ordering::Relaxed);
                Ok(Written::Done)
            }
            // 412 is the condition failing; 409 is another write in flight on the same key.
            Err(e) if matches!(status(&e), Some(412 | 409)) => Ok(Written::Lost),
            Err(e) => Err(failed(e)).with_context(|| format!("sending {name}")),
        }
    }

    /// Puts the file under `name`, all of it or nothing: an object appears when its last
    /// byte is there.
    pub(crate) async fn send_file(&self, name: &str, file: &Path, len: u64) -> anyhow::Result<()> {
        if len < MULTIPART_AT {
            self.send_whole(name, file, len).await
        } else {
            self.send_in_parts(name, file, len).await
        }
        .with_context(|| format!("sending {name}"))
    }

    async fn send_whole(&self, name: &str, file: &Path, len: u64) -> anyhow::Result<()> {
        let _permit = self.traffic.permit().await?;
        self.traffic.operations.fetch_add(1, Ordering::Relaxed);
        let body = ByteStream::from_path(file).await?;
        let request = self.client.put_object().bucket(&self.bucket).key(self.key(name)).body(body);
        request.customize().config_override(self.patient_for(len)).send().await.map_err(failed)?;
        self.traffic.sent.fetch_add(len, Ordering::Relaxed);
        Ok(())
    }

    /// 1. Uploads left behind under this name by a process that was stopped are aborted.
    /// 2. The parts go up a few at a time.
    /// 3. The upload is completed, or aborted when a part could not be sent.
    async fn send_in_parts(&self, name: &str, file: &Path, len: u64) -> anyhow::Result<()> {
        let key = self.key(name);
        self.abort_left_behind(&key).await?;
        self.traffic.operations.fetch_add(1, Ordering::Relaxed);
        let created = self.client.create_multipart_upload().bucket(&self.bucket).key(&key);
        let created = created.send().await.map_err(failed)?;
        let upload = created.upload_id.context("the service named no upload")?;
        let offsets = (0..len.div_ceil(PART)).map(|part| part * PART);
        let parts = stream::iter(offsets.map(|at| self.send_part(&key, &upload, file, at, len)))
            .buffered(self.traffic.jobs())
            .try_collect::<Vec<_>>()
            .await;
        let parts = match parts {
            Ok(parts) => parts,
            Err(e) => {
                self.abort(&key, &upload).await;
                return Err(e);
            }
        };
        self.traffic.operations.fetch_add(1, Ordering::Relaxed);
        let parts = CompletedMultipartUpload::builder().set_parts(Some(parts)).build();
        let complete = self.client.complete_multipart_upload().bucket(&self.bucket).key(&key);
        let done = complete.upload_id(&upload).multipart_upload(parts).send().await;
        if let Err(e) = done {
            self.abort(&key, &upload).await;
            return Err(failed(e));
        }
        Ok(())
    }

    async fn send_part(
        &self,
        key: &str,
        upload: &str,
        file: &Path,
        at: u64,
        len: u64,
    ) -> anyhow::Result<CompletedPart> {
        let _permit = self.traffic.permit().await?;
        self.traffic.operations.fetch_add(1, Ordering::Relaxed);
        let size = PART.min(len - at);
        let number = i32::try_from(at / PART + 1).context("too many parts")?;
        let body = ByteStream::read_from().path(file).offset(at).length(Length::Exact(size));
        let request = self.client.upload_part().bucket(&self.bucket).key(key).upload_id(upload);
        let request = request.part_number(number).body(body.build().await?);
        let sent = request.customize().config_override(self.patient_for(size)).send().await;
        let sent = sent.map_err(failed)?;
        self.traffic.sent.fetch_add(size, Ordering::Relaxed);
        Ok(CompletedPart::builder().part_number(number).set_e_tag(sent.e_tag).build())
    }

    /// The timeouts of a request that carries `len` bytes up.
    fn patient_for(&self, len: u64) -> Builder {
        let attempt = self.patience + Duration::from_secs(len / SLOWEST_UPLOAD);
        let timeouts = TimeoutConfig::builder()
            .connect_timeout(self.patience.min(Duration::from_secs(5)))
            .read_timeout(attempt)
            .operation_attempt_timeout(attempt)
            .build();
        Builder::new().timeout_config(timeouts)
    }

    /// The service keeps the parts of an upload that was never completed until it is
    /// aborted, and bills for them.
    async fn abort_left_behind(&self, key: &str) -> anyhow::Result<()> {
        self.traffic.operations.fetch_add(1, Ordering::Relaxed);
        let listed = self.client.list_multipart_uploads().bucket(&self.bucket).prefix(key);
        let listed = listed.send().await.map_err(failed)?;
        for upload in listed.uploads() {
            if let (Some(of), Some(id)) = (upload.key(), upload.upload_id())
                && of == key
            {
                self.abort(key, id).await;
            }
        }
        Ok(())
    }

    async fn abort(&self, key: &str, upload: &str) {
        self.traffic.operations.fetch_add(1, Ordering::Relaxed);
        let abort = self.client.abort_multipart_upload().bucket(&self.bucket).key(key);
        if let Err(e) = abort.upload_id(upload).send().await {
            eprintln!("remote: an upload of {key} is left unfinished: {:#}", failed(e));
        }
    }
}

/// Counts every attempt at a request, the ones the SDK makes again among them.
struct CountAttempts(Arc<Traffic>);

impl std::fmt::Debug for CountAttempts {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("CountAttempts")
    }
}

impl Intercept for CountAttempts {
    fn name(&self) -> &'static str {
        "ra-jit-count-attempts"
    }

    fn read_before_attempt(
        &self,
        _: &BeforeTransmitInterceptorContextRef<'_>,
        _: &RuntimeComponents,
        _: &mut ConfigBag,
    ) -> Result<(), BoxError> {
        self.0.requests.fetch_add(1, Ordering::Relaxed);
        Ok(())
    }
}

/// When the service says an object was written. One it gives no time for is taken to have
/// been written now, which is the reading that removes nothing.
fn written_at(at: Option<aws_sdk_s3::primitives::DateTime>) -> SystemTime {
    at.and_then(|it| SystemTime::try_from(it).ok()).unwrap_or_else(SystemTime::now)
}

fn status<E, R>(error: &SdkError<E, R>) -> Option<u16>
where
    R: std::borrow::Borrow<aws_sdk_s3::config::http::HttpResponse>,
{
    error.raw_response().map(|it| it.borrow().status().as_u16())
}

/// What went wrong, as the service or the connection said it. Nothing of a request is in
/// it: the SDK keeps credentials and signatures out of what it prints.
fn failed<E>(error: SdkError<E>) -> anyhow::Error
where
    E: ProvideErrorMetadata + std::error::Error + Send + Sync + 'static,
{
    let said = match (error.code(), status(&error)) {
        (Some(code), _) => format!("{code}: {}", error.message().unwrap_or("no message")),
        (None, Some(status)) => format!("the service answered {status}"),
        (None, None) => causes(&error),
    };
    // The service understood and said no. It says 408 and 429 to be asked again. A
    // certificate that is not trusted now is not trusted in a quarter of a second either.
    match status(&error) {
        Some(400..=499) if !matches!(status(&error), Some(408 | 429)) => Refused(said).into(),
        None if is_distrust(&error) => Refused(said).into(),
        _ => anyhow::Error::msg(said),
    }
}

/// Whether the error is the other end's certificate being turned down.
fn is_distrust(error: &(dyn std::error::Error + 'static)) -> bool {
    if matches!(error.downcast_ref(), Some(rustls::Error::InvalidCertificate(_))) {
        return true;
    }
    // An error of the transport carries what it is about inside it, not as its source.
    let inside = error.downcast_ref::<std::io::Error>().and_then(|it| it.get_ref());
    inside.is_some_and(|it| is_distrust(it)) || error.source().is_some_and(is_distrust)
}

/// An error and what it was caused by, each as it describes itself.
fn causes(error: &dyn std::error::Error) -> String {
    let mut said = error.to_string();
    let mut cause = error.source();
    while let Some(next) = cause {
        said = format!("{said}: {next}");
        cause = next.source();
    }
    said
}
