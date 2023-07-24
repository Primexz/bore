//! Server implementation for the `bore` service.

use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;

use anyhow::Result;
use dashmap::DashMap;
use tokio::io::AsyncWriteExt;
use tokio::net::{TcpListener, TcpStream};
use tokio::time::{sleep, timeout};
use tokio_rustls::TlsAcceptor;
use tracing::{info, info_span, warn, Instrument};
use uuid::Uuid;

use crate::auth::Authenticator;
use crate::metrics::{CONNECTED_CLIENTS, HEARTBEATS, DATA_CHANNELS};
use crate::shared::{proxy, ClientMessage, Delimited, ServerMessage, StreamTrait, CONTROL_PORT};

/// State structure for the server.
pub struct Server {
    /// The minimum TCP port that can be forwarded.
    min_port: u16,

    /// Optional secret used to authenticate clients.
    auth: Option<Authenticator>,

    /// Concurrent map of IDs to incoming connections.
    conns: Arc<DashMap<Uuid, TcpStream>>,

    /// Optional tls configuration
    tls: Option<TlsAcceptor>,
}

impl Server {
    /// Create a new server with a specified minimum port number.
    pub fn new(min_port: u16, secret: Option<&str>) -> Self {
        Server::new_with_tls(min_port, secret, None)
    }

    /// Create a new server with a specified minimum port number and tls is configurable.
    pub fn new_with_tls(min_port: u16, secret: Option<&str>, tls: Option<TlsAcceptor>) -> Self {
        Server {
            min_port,
            conns: Arc::new(DashMap::new()),
            auth: secret.map(Authenticator::new),
            tls,
        }
    }

    /// Start the server, listening for new connections.
    pub async fn listen(self) -> Result<()> {
        let this = Arc::new(self);
        let addr = SocketAddr::from(([0, 0, 0, 0], CONTROL_PORT));
        let listener = TcpListener::bind(&addr).await?;
        info!(?addr, "server listening");

        loop {
            let (stream, addr) = listener.accept().await?;
            let this = Arc::clone(&this);
            let stream: Box<dyn StreamTrait> = match &this.tls {
                Some(acceptor) => {
                    let stream = match acceptor.accept(stream).await {
                        Ok(stream) => stream,
                        Err(err) => {
                            warn!(%err,"failed to accept tls connection");
                            continue;
                        }
                    };
                    Box::new(stream)
                }
                None => Box::new(stream),
            };
            tokio::spawn(
                async move {
                    info!("incoming connection");
                    DATA_CHANNELS.inc();
                    if let Err(err) = this.handle_connection(stream).await {
                        warn!(%err, "connection exited with error");
                    } else {
                        info!("connection exited");
                    }
                    DATA_CHANNELS.dec();
                }
                .instrument(info_span!("control", ?addr)),
            );
        }
    }

    async fn handle_connection(&self, stream: Box<dyn StreamTrait>) -> Result<()> {
        let mut stream = Delimited::new(stream);

        if let Some(auth) = &self.auth {
            if let Err(err) = auth.server_handshake(&mut stream).await {
                warn!(%err, "server handshake failed");
                stream.send(ServerMessage::Error(err.to_string())).await?;
                return Ok(());
            }
        }

        match stream.recv_timeout().await? {
            Some(ClientMessage::Authenticate(_)) => {
                warn!("unexpected authenticate");
                Ok(())
            }
            Some(ClientMessage::Hello(port)) => {
                if port != 0 && port < self.min_port {
                    warn!(?port, "client port number too low");
                    return Ok(());
                }
                CONNECTED_CLIENTS.inc();
                info!(?port, "new client");
                let listener = match TcpListener::bind(("0.0.0.0", port)).await {
                    Ok(listener) => listener,
                    Err(_) => {
                        warn!(?port, "could not bind to local port");
                        stream
                            .send(ServerMessage::Error("port already in use".into()))
                            .await?;
                        CONNECTED_CLIENTS.dec();
                        return Ok(());
                    }
                };
                let port = listener.local_addr()?.port();
                stream.send(ServerMessage::Hello(port)).await?;

                loop {
                    info!("sending connection heartbeat");
                    HEARTBEATS.inc();

                    if stream.send(ServerMessage::Heartbeat).await.is_err() {
                        // Assume that the TCP connection has been dropped.
                        CONNECTED_CLIENTS.dec();
                        return Ok(());
                    }
                    const TIMEOUT: Duration = Duration::from_millis(2000);
                    if let Ok(result) = timeout(TIMEOUT, listener.accept()).await {
                        let (stream2, addr) = result?;
                        info!(?addr, ?port, "new connection");

                        let id = Uuid::new_v4();
                        let conns = Arc::clone(&self.conns);

                        conns.insert(id, stream2);
                        tokio::spawn(async move {
                            // Remove stale entries to avoid memory leaks.
                            sleep(Duration::from_secs(10)).await;
                            if conns.remove(&id).is_some() {
                                warn!(%id, "removed stale connection");
                            }
                        });
                        stream.send(ServerMessage::Connection(id)).await?;
                    }
                }
            }
            Some(ClientMessage::Accept(id)) => {
                info!(%id, "forwarding connection");
                match self.conns.remove(&id) {
                    Some((_, mut stream2)) => {
                        let parts = stream.into_parts();
                        debug_assert!(parts.write_buf.is_empty(), "framed write buffer not empty");
                        stream2.write_all(&parts.read_buf).await?;

                        proxy(parts.io, stream2).await?
                    }
                    None => warn!(%id, "missing connection"),
                }
                CONNECTED_CLIENTS.dec();
                Ok(())
            }
            None => {
                warn!("unexpected EOF");
                CONNECTED_CLIENTS.dec();
                Ok(())
            }
        }
    }
}

impl Default for Server {
    fn default() -> Self {
        Server::new(1024, None)
    }
}
