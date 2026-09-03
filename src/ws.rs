//! WebSocket transport (feature `ws`): Obfuscated2 framing over `wss://`.
//!
//! Used as a fallback when raw TCP is blocked (DPI throttling, regional
//! blocks). The WebSocket layer is just a byte pipe: after the WSS
//! handshake, the exact same Obfuscated2 init exchange and Intermediate
//! framing run inside binary WS messages. Server endpoint:
//! `wss://{dc_host}:443/apiws_p`, subprotocol `binary`.
//!
//! SPEC §11.3 gap #15 (P3), BS-6 transport fallback.

use crate::error::{Error, Result};
use crate::transport::{Obfuscated2Transport, TransportProtocol, obfuscated2_keys};

use futures_util::{SinkExt, StreamExt};
use tokio_tungstenite::Connector;
use tokio_tungstenite::tungstenite::client::IntoClientRequest;
use tokio_tungstenite::tungstenite::http::header::HOST;
use tokio_tungstenite::tungstenite::protocol::Message;

/// A live Obfuscated2-over-WebSocket connection.
pub struct WsTransport {
    sink: futures_util::stream::SplitSink<WsStream, Message>,
    stream: futures_util::stream::SplitStream<WsStream>,
    /// Binary bytes received but not yet consumed (frames straddle WS
    /// message boundaries, so reads coalesce here).
    pending: Vec<u8>,
}

pub type WsStream =
    tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>>;

impl WsTransport {
    /// Raw write of already-framed bytes: one WS binary message.
    async fn raw_write(&mut self, bytes: &[u8]) -> Result<()> {
        self.sink
            .send(Message::Binary(bytes.to_vec().into()))
            .await
            .map_err(ws_err)
    }

    /// Fill `buf` fully, coalescing across WS binary messages.
    async fn raw_read_exact(&mut self, buf: &mut [u8]) -> Result<()> {
        while self.pending.len() < buf.len() {
            let msg = self
                .stream
                .next()
                .await
                .ok_or_else(|| Error::Transport("ws stream closed".into()))?
                .map_err(ws_err)?;
            match msg {
                Message::Binary(data) => self.pending.extend_from_slice(&data),
                Message::Close(c) => {
                    return Err(Error::Transport(format!("ws closed by peer: {c:?}")));
                }
                Message::Ping(p) => {
                    // M7: a split stream does NOT auto-reply to pings —
                    // queue the Pong explicitly or the server kills an
                    // idle connection on its ping timeout.
                    self.sink.send(Message::Pong(p)).await.map_err(ws_err)?;
                }
                Message::Pong(_) | Message::Text(_) => {}
                Message::Frame(_) => unreachable!("raw frames handled by tungstenite"),
            }
        }
        buf.copy_from_slice(&self.pending[..buf.len()]);
        self.pending.drain(..buf.len());
        Ok(())
    }
}

impl crate::transport::FrameStream for WsTransport {
    async fn fs_write_all(&mut self, bytes: &[u8]) -> Result<()> {
        self.raw_write(bytes).await
    }
    async fn fs_read_exact(&mut self, buf: &mut [u8]) -> Result<()> {
        self.raw_read_exact(buf).await
    }
}

fn ws_err(e: tokio_tungstenite::tungstenite::Error) -> Error {
    Error::Transport(format!("ws: {e}"))
}

/// `wss://` host for a DC (TLS SNI / Host header — raw IPs would fail cert
/// validation).
pub fn dc_ws_host(dc_id: i32) -> Result<String> {
    let host = match dc_id.abs() {
        1 => "pluto.web.telegram.org",
        2 => "venus.web.telegram.org",
        3 => "aurora.web.telegram.org",
        4 => "vesta.web.telegram.org",
        5 => "flora.web.telegram.org",
        201 => "venus-1.web.telegram.org", // test DC
        _ => return Err(Error::Transport(format!("unknown DC ID: {dc_id}"))),
    };
    Ok(host.to_string())
}

/// Connect with Obfuscated2 framing over a `wss://` WebSocket and return
/// the codec. Same wire behaviour as
/// [`transport::connect_obfuscated2`](crate::transport::connect_obfuscated2):
/// 64-byte init (protocol tag at 56..60, DC ID at 60..62, bytes 56..64
/// replaced by their CTR-encrypted form) sent as the first binary frame;
/// Intermediate frames ride inside binary WS messages afterwards.
pub async fn connect_obfuscated2_ws(
    dc_id: i32,
    protocol: TransportProtocol,
) -> Result<Obfuscated2Transport<WsTransport>> {
    let host = dc_ws_host(dc_id)?;
    let url = format!("wss://{host}:443/apiws_p");
    let mut request = url
        .as_str()
        .into_client_request()
        .map_err(|e| Error::Transport(format!("ws request: {e}")))?;
    request.headers_mut().insert(
        HOST,
        format!("{host}:443")
            .parse()
            .map_err(|_| Error::Transport("ws host header".into()))?,
    );
    request.headers_mut().append(
        "Sec-WebSocket-Protocol",
        "binary".parse().expect("static header value"),
    );

    let (ws, _resp) = tokio_tungstenite::connect_async_tls_with_config(
        request,
        None,
        false,
        Some(Connector::Rustls(std::sync::Arc::new(
            rustls_client_config(),
        ))),
    )
    .await
    .map_err(ws_err)?;

    let (sink, stream) = ws.split();
    let mut pipe = WsTransport {
        sink,
        stream,
        pending: Vec::new(),
    };

    let (enc, dec, init_copy) = obfuscated2_keys(protocol, dc_id);
    pipe.raw_write(&init_copy).await?;

    Ok(Obfuscated2Transport::new(pipe, enc, dec))
}
fn rustls_client_config() -> tokio_rustls::rustls::ClientConfig {
    let mut roots = tokio_rustls::rustls::RootCertStore::empty();
    for cert in rustls_native_certs::load_native_certs().certs {
        let _ = roots.add(cert);
    }
    tokio_rustls::rustls::ClientConfig::builder()
        .with_root_certificates(roots)
        .with_no_client_auth()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ws_hosts_cover_known_dcs() {
        assert_eq!(dc_ws_host(2).unwrap(), "venus.web.telegram.org");
        assert_eq!(dc_ws_host(-4).unwrap(), "vesta.web.telegram.org");
        assert_eq!(dc_ws_host(201).unwrap(), "venus-1.web.telegram.org");
        assert!(dc_ws_host(7).is_err());
    }
}
