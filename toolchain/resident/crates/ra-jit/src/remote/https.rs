//! A remote that is files any HTTPS server hands out, such as the assets of a GitHub release:
//! read-only, and flat. The object `packs/<hash>.pack` is the file `packs.<hash>.pack` under
//! the address, as a release has no directories. A redirect is followed, as GitHub's
//! downloads are.

use std::{
    sync::{Arc, atomic::Ordering},
    time::{Duration, SystemTime},
};

use anyhow::{Context, bail};
use http_body_util::{BodyExt, Empty};
use hyper::{
    Method, Request, StatusCode,
    body::{Bytes, Incoming},
    header,
};
use hyper_rustls::{HttpsConnector, HttpsConnectorBuilder};
use hyper_util::{
    client::legacy::{Client, connect::HttpConnector},
    rt::TokioExecutor,
};
use url::Url;

use super::transfer::{Object, Refused, TooLong, Traffic, Version};

/// How many redirects one request follows.
const REDIRECTS: usize = 5;

/// How long a connection may take to be made. A network that is down or drops what is sent
/// is told from a slow answer this soon, not after the patience of a request.
pub(crate) const CONNECT_WITHIN: Duration = Duration::from_secs(2);

/// The connector of a client to an HTTP(S) remote: [`CONNECT_WITHIN`], or `patience` when
/// that is shorter.
pub(crate) fn connector(patience: Duration) -> HttpConnector {
    let mut http = HttpConnector::new();
    http.enforce_http(false);
    http.set_connect_timeout(Some(CONNECT_WITHIN.min(patience)));
    http
}

pub(crate) struct Https {
    client: Client<HttpsConnector<HttpConnector>, Empty<Bytes>>,
    /// With no `/` at its end.
    base: String,
    patience: Duration,
    traffic: Arc<Traffic>,
}

impl Https {
    pub(crate) fn open(
        spec: &Url,
        patience: Duration,
        traffic: Arc<Traffic>,
    ) -> anyhow::Result<Https> {
        let roots = HttpsConnectorBuilder::new()
            .with_native_roots()
            .context("the machine's certificates cannot be read")?;
        let connector = roots.https_or_http().enable_http1().wrap_connector(connector(patience));
        let client = Client::builder(TokioExecutor::new()).build(connector);
        let base = spec.as_str().trim_end_matches('/').to_owned();
        Ok(Https { client, base, patience, traffic })
    }

    pub(crate) fn place(&self) -> String {
        self.base.clone()
    }

    fn address(&self, name: &str) -> String {
        format!("{}/{}", self.base, name.replace('/', "."))
    }

    /// The answer to `method` on `name`, from `offset` on, after its redirects; `None` when
    /// nothing is there.
    async fn ask(
        &self,
        method: Method,
        name: &str,
        offset: u64,
    ) -> anyhow::Result<Option<hyper::Response<Incoming>>> {
        let mut address = self.address(name);
        for _ in 0..=REDIRECTS {
            let mut request = Request::builder().method(method.clone()).uri(&address);
            if offset != 0 {
                request = request.header(header::RANGE, format!("bytes={offset}-"));
            }
            let request = request.body(Empty::new())?;
            self.traffic.operations.fetch_add(1, Ordering::Relaxed);
            let answer = tokio::time::timeout(self.patience, self.client.request(request))
                .await
                .with_context(|| format!("no answer for {:?} from {address}", self.patience))?
                .with_context(|| format!("asking {address}"))?;
            let status = answer.status();
            if status.is_redirection() {
                let to = answer.headers().get(header::LOCATION).and_then(|it| it.to_str().ok());
                let to = to.with_context(|| format!("{address} redirects nowhere"))?;
                address = Url::parse(&address)?.join(to)?.to_string();
                continue;
            }
            return match status {
                StatusCode::NOT_FOUND => Ok(None),
                StatusCode::OK if offset == 0 => Ok(Some(answer)),
                StatusCode::PARTIAL_CONTENT if offset != 0 => Ok(Some(answer)),
                StatusCode::OK => bail!("{address} sends the whole object when asked for its rest"),
                // What the server refuses it refuses each time.
                it if it.is_client_error() => Err(Refused(format!("{address}: {it}")).into()),
                it => bail!("{address}: {it}"),
            };
        }
        Err(Refused(format!("{address}: more than {REDIRECTS} redirects")).into())
    }

    pub(crate) async fn read_from(
        &self,
        name: &str,
        offset: u64,
    ) -> anyhow::Result<Option<Incoming>> {
        Ok(self.ask(Method::GET, name, offset).await?.map(hyper::Response::into_body))
    }

    pub(crate) async fn read_small(
        &self,
        name: &str,
        most: u64,
    ) -> anyhow::Result<Option<(Vec<u8>, Version)>> {
        let Some(mut body) = self.read_from(name, 0).await? else { return Ok(None) };
        let mut bytes = Vec::new();
        while let Some(chunk) = next(&mut body).await? {
            if (bytes.len() + chunk.len()) as u64 > most {
                return Err(TooLong { name: name.to_owned(), most }.into());
            }
            self.traffic.received.fetch_add(chunk.len() as u64, Ordering::Relaxed);
            bytes.extend_from_slice(&chunk);
        }
        Ok(Some((bytes, Version(None))))
    }

    pub(crate) async fn found(&self, name: &str) -> anyhow::Result<Option<Object>> {
        let Some(answer) = self.ask(Method::HEAD, name, 0).await? else { return Ok(None) };
        let len = answer.headers().get(header::CONTENT_LENGTH).and_then(|it| it.to_str().ok());
        let len = len.and_then(|it| it.parse().ok()).unwrap_or(0);
        Ok(Some(Object { name: name.to_owned(), len, written: SystemTime::now() }))
    }
}

/// The next bytes of an answer's body, or `None` at its end.
pub(crate) async fn next(body: &mut Incoming) -> anyhow::Result<Option<Vec<u8>>> {
    while let Some(frame) = body.frame().await {
        if let Ok(data) = frame?.into_data() {
            return Ok(Some(data.to_vec()));
        }
    }
    Ok(None)
}
