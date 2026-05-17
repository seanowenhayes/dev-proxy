use httparse::{Request, Response, Status};
use once_cell::sync::Lazy;
use rcgen::{
    BasicConstraints, Certificate, CertificateParams, DistinguishedName, DnType,
    IsCa, Issuer, KeyPair, KeyUsagePurpose, SanType,
};
use rustls::pki_types::{CertificateDer, PrivateKeyDer, PrivatePkcs8KeyDer, ServerName};
use rustls::{ClientConfig, RootCertStore, ServerConfig};
use std::collections::HashMap;
use std::io::Cursor;
use std::pin::Pin;
use std::sync::Arc;
use std::task::{Context, Poll};
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt, ReadBuf};
use tokio::net::TcpStream;
use tokio_rustls::TlsAcceptor;

use crate::proxy::ProxyEvent;
use tokio::sync::mpsc;

struct BufferedStream<S> {
    prefix: Cursor<Vec<u8>>,
    inner: S,
}

impl<S: AsyncRead + Unpin> AsyncRead for BufferedStream<S> {
    fn poll_read(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut ReadBuf<'_>,
    ) -> Poll<std::io::Result<()>> {
        let this = self.get_mut();
        let prefix_remaining = this.prefix.get_ref().len() - this.prefix.position() as usize;
        if prefix_remaining > 0 {
            let n = std::io::Read::read(&mut this.prefix, buf.initialize_unfilled()).unwrap_or(0);
            buf.advance(n);
            Poll::Ready(Ok(()))
        } else {
            Pin::new(&mut this.inner).poll_read(cx, buf)
        }
    }
}

impl<S: AsyncWrite + Unpin> AsyncWrite for BufferedStream<S> {
    fn poll_write(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &[u8],
    ) -> Poll<Result<usize, std::io::Error>> {
        Pin::new(&mut self.get_mut().inner).poll_write(cx, buf)
    }
    fn poll_flush(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
    ) -> Poll<Result<(), std::io::Error>> {
        Pin::new(&mut self.get_mut().inner).poll_flush(cx)
    }
    fn poll_shutdown(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
    ) -> Poll<Result<(), std::io::Error>> {
        Pin::new(&mut self.get_mut().inner).poll_shutdown(cx)
    }
}

struct MitmCa {
    params: CertificateParams,
    key: KeyPair,
    cert: Certificate,
}

static MITM_CA: Lazy<MitmCa> = Lazy::new(|| {
    let mut params = CertificateParams::new(Vec::<String>::new()).expect("CA params");
    let mut dn = DistinguishedName::new();
    dn.push(DnType::CommonName, "dev-proxy-mitm-ca");
    dn.push(DnType::OrganizationName, "dev-proxy");
    params.distinguished_name = dn;
    params.is_ca = IsCa::Ca(BasicConstraints::Unconstrained);
    params.key_usages = vec![
        KeyUsagePurpose::KeyCertSign,
        KeyUsagePurpose::CrlSign,
    ];

    let key = KeyPair::generate().expect("CA key");
    let cert = params.self_signed(&key).expect("CA self-signed");

    MitmCa { params, key, cert }
});

/// Write the CA certificate as PEM to the given path.
pub fn export_ca_cert(path: &str) -> Result<(), String> {
    let pem = MITM_CA.cert.pem();
    std::fs::write(path, pem.as_bytes())
        .map_err(|e| format!("write CA cert: {e}"))
}

pub fn ca_cert_info() -> String {
    let pem = MITM_CA.cert.pem();
    let subject = pem
        .lines()
        .filter(|l| !l.starts_with("-----"))
        .take(1)
        .next()
        .unwrap_or("?");
    format!("CA cert: dev-proxy-mitm-ca ({subject}...)")
}

struct LeafCert {
    cert_der: CertificateDer<'static>,
    key_der: Vec<u8>,
}

static LEAF_CACHE: once_cell::sync::Lazy<std::sync::Mutex<HashMap<String, LeafCert>>> =
    once_cell::sync::Lazy::new(|| std::sync::Mutex::new(HashMap::new()));

fn get_or_create_leaf(host: &str) -> Result<(CertificateDer<'static>, PrivateKeyDer<'static>), String> {
    let mut cache = LEAF_CACHE
        .lock()
        .map_err(|e| format!("leaf cache lock: {e}"))?;

    if let Some(leaf) = cache.get(host) {
        let key_der = PrivatePkcs8KeyDer::from(leaf.key_der.clone());
        return Ok((leaf.cert_der.clone(), PrivateKeyDer::Pkcs8(key_der.into())));
    }

    let mut params = CertificateParams::new(vec![host.to_string()]).map_err(|e| format!("cert params: {e}"))?;
    let mut dn = DistinguishedName::new();
    dn.push(DnType::CommonName, host.to_string());
    params.distinguished_name = dn;
    params
        .subject_alt_names
        .push(SanType::DnsName(host.try_into().map_err(|e| format!("dns name: {e}"))?));

    let key = KeyPair::generate().map_err(|e| format!("key gen: {e}"))?;
    let issuer = Issuer::from_params(&MITM_CA.params, &MITM_CA.key);
    let cert = params
        .signed_by(&key, &issuer)
        .map_err(|e| format!("sign by CA: {e}"))?;

    let cert_der: CertificateDer<'static> = cert.der().clone();
    let key_der_vec = key.serialize_der();

    cache.insert(
        host.to_string(),
        LeafCert {
            cert_der: cert_der.clone(),
            key_der: key_der_vec.clone(),
        },
    );

    let key_der = PrivatePkcs8KeyDer::from(key_der_vec);
    Ok((cert_der, PrivateKeyDer::Pkcs8(key_der.into())))
}

pub fn make_server_config_for_host(host: &str) -> Result<ServerConfig, String> {
    let (cert_der, key_der) = get_or_create_leaf(host)?;

    ServerConfig::builder()
        .with_no_client_auth()
        .with_single_cert(vec![cert_der], key_der)
        .map_err(|e| format!("server config: {e}"))
}

fn make_server_config(host: &str) -> Result<ServerConfig, String> {
    let (cert_der, key_der) = get_or_create_leaf(host)?;

    ServerConfig::builder()
        .with_no_client_auth()
        .with_single_cert(vec![cert_der], key_der)
        .map_err(|e| format!("server config: {e}"))
}

fn make_client_config() -> ClientConfig {
    let mut root_store = RootCertStore::empty();
    root_store.extend(webpki_roots::TLS_SERVER_ROOTS.iter().cloned());

    ClientConfig::builder()
        .with_root_certificates(root_store)
        .with_no_client_auth()
}

pub async fn mitm_handler(
    client_stream: impl AsyncRead + AsyncWrite + Unpin + Send,
    addr: &str,
    id: &str,
    event_tx: mpsc::Sender<ProxyEvent>,
) -> Result<(), String> {
    let host = addr
        .split(':')
        .next()
        .unwrap_or(addr)
        .to_string();

    let server_config = Arc::new(make_server_config(&host)?);
    let acceptor = TlsAcceptor::from(server_config);

    let client_tls = acceptor
        .accept(client_stream)
        .await
        .map_err(|e| format!("tls accept: {e}"))?;

    mitm_handle_decrypted(client_tls, &host, id, event_tx).await
}

pub async fn mitm_handler_from_buffered(
    client_stream: impl AsyncRead + AsyncWrite + Unpin + Send,
    prefix: Vec<u8>,
    addr: &str,
    id: &str,
    event_tx: mpsc::Sender<ProxyEvent>,
) -> Result<(), String> {
    let host = addr
        .split(':')
        .next()
        .unwrap_or(addr)
        .to_string();

    let server_config = Arc::new(make_server_config(&host)?);
    let acceptor = TlsAcceptor::from(server_config);

    let buffered = BufferedStream {
        prefix: Cursor::new(prefix),
        inner: client_stream,
    };
    let client_tls = acceptor
        .accept(buffered)
        .await
        .map_err(|e| format!("tls accept: {e}"))?;

    mitm_handle_decrypted(client_tls, &host, id, event_tx).await
}

/// Handle an already-decrypted TLS stream (TLS accept done externally).
pub async fn mitm_handle_decrypted(
    mut client_tls: impl AsyncRead + AsyncWrite + Unpin,
    host: &str,
    id: &str,
    event_tx: mpsc::Sender<ProxyEvent>,
) -> Result<(), String> {
    let host = host.to_string();
    let mut client_buf = [0u8; 16384];
    let mut client_read = 0;

    loop {
        let n = client_tls
            .read(&mut client_buf[client_read..])
            .await
            .map_err(|e| format!("read from client: {e}"))?;
        if n == 0 {
            return Ok(());
        }
        client_read += n;

        let mut headers_buf = [httparse::EMPTY_HEADER; 64];
        let mut request = Request::new(&mut headers_buf);
        match request.parse(&client_buf[..client_read]) {
            Ok(Status::Complete(parsed_len)) => {
                let method = request.method.unwrap_or("GET").to_string();
                let path = request.path.unwrap_or("/").to_string();
                let version = request.version.unwrap_or(1);

                let mut headers: HashMap<String, String> = HashMap::new();
                let mut host_header_val = None;
                for h in request.headers.iter() {
                    let name = h.name.to_string();
                    let value = String::from_utf8_lossy(h.value).to_string();
                    if name.to_lowercase() == "host" {
                        host_header_val = Some(value.clone());
                    }
                    headers.insert(name, value);
                }
                let host_header = host_header_val.unwrap_or_default();

                let url = if host_header.is_empty() || host_header.starts_with(&host) {
                    format!("https://{host}{path}")
                } else {
                    let scheme = if host.contains(":443") || !host.contains(':') {
                        "https"
                    } else {
                        "http"
                    };
                    format!("{scheme}://{host_header}{path}")
                };

                let request_line = format!("{method} {path} HTTP/1.{version}\r\n");
                let mut header_bytes = Vec::new();
                header_bytes.extend_from_slice(request_line.as_bytes());
                for h in request.headers.iter() {
                    header_bytes.extend_from_slice(h.name.as_bytes());
                    header_bytes.extend_from_slice(b": ");
                    header_bytes.extend_from_slice(h.value);
                    header_bytes.extend_from_slice(b"\r\n");
                }
                header_bytes.extend_from_slice(b"\r\n");

                let body_start = parsed_len;
                let body_len = client_read - body_start;
                let body = client_buf[body_start..client_read].to_vec();

                let _ = event_tx
                    .send(ProxyEvent::MitmRequest {
                        id: id.to_string(),
                        method: method.clone(),
                        url: url.clone(),
                        headers: headers.clone(),
                    })
                    .await;

                let client_config = Arc::new(make_client_config());
                let domain = host_header
                    .split(':')
                    .next()
                    .unwrap_or(&host);
                let sn = ServerName::try_from(domain.to_string())
                    .map_err(|e| format!("server name: {e}"))?
                    .to_owned();

                let connector = tokio_rustls::TlsConnector::from(client_config);
                let server_addr = if host_header.contains(':') {
                    host_header.clone()
                } else {
                    format!("{host_header}:443")
                };
                let server_tcp = TcpStream::connect(&server_addr)
                    .await
                    .map_err(|e| format!("connect to server: {e}"))?;
                let mut s_tls = connector
                    .connect(sn, server_tcp)
                    .await
                    .map_err(|e| format!("tls connect to server: {e}"))?;

                s_tls
                    .write_all(&header_bytes)
                    .await
                    .map_err(|e| format!("write request to server: {e}"))?;
                if body_len > 0 {
                    s_tls
                        .write_all(&body)
                        .await
                        .map_err(|e| format!("write body to server: {e}"))?;
                }

                let resp_bytes = read_full_response(&mut s_tls).await?;

                tracing::info!("Got response from server: {} bytes for {}", resp_bytes.len(), url);

                let mut hb = [httparse::EMPTY_HEADER; 64];
                let mut r = Response::new(&mut hb);
                match r.parse(&resp_bytes) {
                    Ok(Status::Complete(parsed_len)) => {
                        let status = r.code.unwrap_or(200);
                        let mut response_headers: HashMap<String, String> = HashMap::new();
                        for h in r.headers {
                            response_headers.insert(h.name.to_string(), String::from_utf8_lossy(h.value).to_string());
                        }
                        let status_text = match status {
                            200 => "OK", 201 => "Created", 204 => "No Content",
                            301 => "Moved Permanently", 302 => "Found", 304 => "Not Modified",
                            400 => "Bad Request", 401 => "Unauthorized", 403 => "Forbidden",
                            404 => "Not Found", 500 => "Internal Server Error", _ => "",
                        }.to_string();

                        client_tls.write_all(&resp_bytes).await.map_err(|e| format!("write resp: {e}"))?;
                        client_tls.flush().await.map_err(|e| format!("flush resp: {e}"))?;
                        tracing::info!("Wrote {} bytes to client for {}", resp_bytes.len(), url);

                        let body_size = resp_bytes.len() as u64 - parsed_len as u64;
                        let _ = event_tx.send(ProxyEvent::MitmResponse {
                            id: id.to_string(), status, status_text,
                            headers: response_headers, body_size,
                        }).await;

                        client_read = 0;
                        continue;
                    }
                    _ => {
                        client_tls.write_all(&resp_bytes).await.map_err(|e| format!("write resp: {e}"))?;
                        client_tls.flush().await.ok();
                        return Ok(());
                    }
                }
            }
            Ok(Status::Partial) => {
                if client_read == client_buf.len() {
                    return Err("request headers too large".into());
                }
                continue;
            }
            Err(e) => {
                return Err(format!("parse request: {e}"));
            }
        }
    }
}

/// Read the full HTTP response (headers + body) into a single buffer.
async fn read_full_response<S: AsyncRead + AsyncWrite + Unpin>(
    stream: &mut S,
) -> Result<Vec<u8>, String> {
    let mut buf = Vec::new();
    let mut temp_buf = [0u8; 8192];

    // Read until we have complete headers
    loop {
        let n = stream
            .read(&mut temp_buf)
            .await
            .map_err(|e| format!("read response: {e}"))?;
        if n == 0 {
            return Err("server closed before response".into());
        }
        buf.extend_from_slice(&temp_buf[..n]);

        let mut headers_buf = [httparse::EMPTY_HEADER; 64];
        let mut response = Response::new(&mut headers_buf);
        if let Ok(Status::Complete(parsed_len)) = response.parse(&buf) {
            // Now read the body
            let is_chunked = response
                .headers
                .iter()
                .any(|h| h.name.eq_ignore_ascii_case("transfer-encoding")
                    && String::from_utf8_lossy(h.value).to_lowercase().contains("chunked"));

            let content_length: Option<u64> = response
                .headers
                .iter()
                .find(|h| h.name.eq_ignore_ascii_case("content-length"))
                .and_then(|h| String::from_utf8_lossy(h.value).parse().ok());

            if is_chunked {
                // Read chunked body
                loop {
                    let mut line = Vec::new();
                    loop {
                        let mut byte = [0u8; 1];
                        stream.read_exact(&mut byte).await.map_err(|e| format!("read chunk: {e}"))?;
                        if byte[0] == b'\n' { break; }
                        if byte[0] != b'\r' { line.push(byte[0]); }
                    }
                    let line_str = String::from_utf8_lossy(&line);
                    let chunk_size_str = line_str.split(';').next().unwrap_or("0").trim();
                    let chunk_size = u64::from_str_radix(chunk_size_str, 16).map_err(|e| format!("parse chunk size: {e}"))?;
                    if chunk_size == 0 { break; }
                    let mut chunk = vec![0u8; chunk_size as usize];
                    stream.read_exact(&mut chunk).await.map_err(|e| format!("read chunk data: {e}"))?;
                    buf.extend_from_slice(&chunk);
                    // Read trailing CRLF
                    let mut crlf = [0u8; 2];
                    stream.read_exact(&mut crlf).await.ok();
                }
                // Read final CRLF
                let mut crlf = [0u8; 2];
                stream.read_exact(&mut crlf).await.ok();
            } else if let Some(len) = content_length {
                let remaining = len as usize - (buf.len() - parsed_len);
                if remaining > 0 {
                    let mut body = vec![0u8; remaining];
                    stream.read_exact(&mut body).await.map_err(|e| format!("read body: {e}"))?;
                    buf.extend_from_slice(&body);
                }
            } else {
                // No content-length, read until close
                loop {
                    let n = stream.read(&mut temp_buf).await.unwrap_or(0);
                    if n == 0 { break; }
                    buf.extend_from_slice(&temp_buf[..n]);
                }
            }

            return Ok(buf);
        }
    }
}
