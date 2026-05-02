use httparse::{Request, Response, Status};
use once_cell::sync::Lazy;
use rcgen::{
    BasicConstraints, CertificateParams, DistinguishedName, DnType,
    IsCa, Issuer, KeyPair, KeyUsagePurpose, SanType,
};
use rustls::pki_types::{CertificateDer, PrivateKeyDer, PrivatePkcs8KeyDer, ServerName};
use rustls::{ClientConfig, RootCertStore, ServerConfig};
use std::collections::HashMap;
use std::sync::Arc;
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};
use tokio::net::TcpStream;
use tokio_rustls::TlsAcceptor;

use crate::proxy::ProxyEvent;
use tokio::sync::mpsc;

struct MitmCa {
    params: CertificateParams,
    key: KeyPair,
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
    let _ca_cert = params.self_signed(&key).expect("CA self-signed");

    MitmCa { params, key }
});

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

    let mut client_tls = acceptor
        .accept(client_stream)
        .await
        .map_err(|e| format!("tls accept: {e}"))?;

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
                let server_name = ServerName::try_from(domain.to_string())
                    .map_err(|e| format!("server name: {e}"))?
                    .to_owned();

                let connector = tokio_rustls::TlsConnector::from(client_config);
                let server_tcp = TcpStream::connect(addr)
                    .await
                    .map_err(|e| format!("connect to server: {e}"))?;
                let mut server_tls = connector
                    .connect(server_name, server_tcp)
                    .await
                    .map_err(|e| format!("tls connect to server: {e}"))?;

                server_tls
                    .write_all(&header_bytes)
                    .await
                    .map_err(|e| format!("write request to server: {e}"))?;
                if body_len > 0 {
                    server_tls
                        .write_all(&body)
                        .await
                        .map_err(|e| format!("write body to server: {e}"))?;
                }

                let (status, response_headers, response_line) =
                    read_http_response(&mut server_tls).await?;

                let status_text = match status {
                    200 => "OK",
                    201 => "Created",
                    204 => "No Content",
                    301 => "Moved Permanently",
                    302 => "Found",
                    304 => "Not Modified",
                    400 => "Bad Request",
                    401 => "Unauthorized",
                    403 => "Forbidden",
                    404 => "Not Found",
                    500 => "Internal Server Error",
                    _ => "",
                }
                .to_string();

                let mut response_buf = response_line.into_bytes();
                for (name, value) in &response_headers {
                    response_buf.extend_from_slice(name.as_bytes());
                    response_buf.extend_from_slice(b": ");
                    response_buf.extend_from_slice(value.as_bytes());
                    response_buf.extend_from_slice(b"\r\n");
                }
                response_buf.extend_from_slice(b"\r\n");

                client_tls
                    .write_all(&response_buf)
                    .await
                    .map_err(|e| format!("write response to client: {e}"))?;

                let is_chunked = response_headers
                    .get("transfer-encoding")
                    .map(|v| v.to_lowercase().contains("chunked"))
                    .unwrap_or(false);

                let content_length: Option<u64> = response_headers
                    .get("content-length")
                    .and_then(|v| v.parse().ok());

                let mut total_body = 0u64;

                if is_chunked {
                    loop {
                        let chunk_size = read_chunk_size(&mut server_tls).await?;
                        if chunk_size == 0 {
                            break;
                        }
                        let mut chunk = vec![0u8; chunk_size as usize];
                        server_tls
                            .read_exact(&mut chunk)
                            .await
                            .map_err(|e| format!("read chunk: {e}"))?;
                        client_tls
                            .write_all(&chunk)
                            .await
                            .map_err(|e| format!("write chunk to client: {e}"))?;
                        total_body += chunk_size as u64;

                        let mut crlf = [0u8; 2];
                        server_tls
                            .read_exact(&mut crlf)
                            .await
                            .map_err(|e| format!("read chunk crlf: {e}"))?;
                    }
                    let mut crlf = [0u8; 2];
                    let _ = server_tls.read_exact(&mut crlf).await;
                } else if let Some(len) = content_length {
                    if len > 0 {
                        let mut body_buf = vec![0u8; len as usize];
                        server_tls
                            .read_exact(&mut body_buf)
                            .await
                            .map_err(|e| format!("read body: {e}"))?;
                        client_tls
                            .write_all(&body_buf)
                            .await
                            .map_err(|e| format!("write body to client: {e}"))?;
                        total_body = len;
                    }
                } else {
                    let mut buf = [0u8; 4096];
                    loop {
                        let n = server_tls.read(&mut buf).await.unwrap_or(0);
                        if n == 0 {
                            break;
                        }
                        client_tls
                            .write_all(&buf[..n])
                            .await
                            .map_err(|e| format!("write to client: {e}"))?;
                        total_body += n as u64;
                    }
                }

                let _ = event_tx
                    .send(ProxyEvent::MitmResponse {
                        id: id.to_string(),
                        status,
                        status_text,
                        headers: response_headers,
                        body_size: total_body,
                    })
                    .await;

                client_tls.shutdown().await.ok();

                return Ok(());
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

async fn read_http_response<S: AsyncRead + AsyncWrite + Unpin>(
    stream: &mut S,
) -> Result<(u16, HashMap<String, String>, String), String> {
    let mut buf = Vec::new();
    let mut temp_buf = [0u8; 4096];

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
        match response.parse(&buf) {
            Ok(Status::Complete(_)) => {
                let status = response.code.unwrap_or(200);
                let reason = response.reason.unwrap_or("");
                let version = response.version.unwrap_or(1);

                let mut headers: HashMap<String, String> = HashMap::new();
                for h in response.headers {
                    headers.insert(
                        h.name.to_string(),
                        String::from_utf8_lossy(h.value).to_string(),
                    );
                }

                let response_line = format!("HTTP/1.{version} {status} {reason}\r\n");
                return Ok((status, headers, response_line));
            }
            Ok(Status::Partial) => continue,
            Err(e) => return Err(format!("parse response: {e}")),
        }
    }
}

async fn read_chunk_size<S: AsyncRead + Unpin>(stream: &mut S) -> Result<u64, String> {
    let mut line = Vec::new();
    let mut byte = [0u8; 1];
    loop {
        stream
            .read_exact(&mut byte)
            .await
            .map_err(|e| format!("read chunk size: {e}"))?;
        if byte[0] == b'\n' {
            break;
        }
        if byte[0] != b'\r' {
            line.push(byte[0]);
        }
    }
    let line_str = String::from_utf8_lossy(&line);
    let size_str = line_str
        .split(';')
        .next()
        .unwrap_or("0")
        .trim();
    u64::from_str_radix(size_str, 16).map_err(|e| format!("parse chunk size: {e}"))
}
