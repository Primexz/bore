use anyhow::{Ok, Result};
use bore_cli::{client::Client, metrics::start_metric_server, server::Server};
use clap::{Parser, Subcommand};
use rustls_pemfile::certs;
use std::{
    fs::File,
    io::{self, BufReader},
    path::PathBuf,
    sync::Arc,
};
use tokio_rustls::{
    rustls::{self, Certificate, OwnedTrustAnchor, PrivateKey},
    webpki, TlsAcceptor, TlsConnector,
};
use tracing::{error, info, info_span, Instrument};

#[derive(Parser, Debug)]
#[clap(author, version, about)]
#[clap(propagate_version = true)]
struct Args {
    #[clap(subcommand)]
    command: Command,
}

#[derive(Subcommand, Debug)]
enum Command {
    /// Starts a local proxy to the remote server.
    Local {
        /// The local port to expose.
        local_port: u16,

        /// The local host to expose.
        #[clap(short, long, value_name = "HOST", default_value = "localhost")]
        local_host: String,

        /// Address of the remote server to expose local ports to.
        #[clap(short, long)]
        to: String,

        /// Optional port on the remote server to select.
        #[clap(short, long, default_value_t = 0)]
        port: u16,

        /// Optional secret for authentication.
        #[clap(short, long, env = "BORE_SECRET", hide_env_values = true)]
        secret: Option<String>,

        /// Enable tls support for the tunnel.
        #[clap(long)]
        tls: bool,

        /// Path to cafile file for self signed certifactes, if tls is enabled.
        #[clap(long)]
        cafile: Option<PathBuf>,
    },

    /// Runs the remote proxy server.
    Server {
        /// Minimum TCP port number to accept.
        #[clap(long, default_value_t = 1024)]
        min_port: u16,

        /// Optional secret for authentication.
        #[clap(short, long, env = "BORE_SECRET", hide_env_values = true)]
        secret: Option<String>,

        /// Enable tls support for the tunnel.
        #[clap(long)]
        tls: bool,

        /// Path to cert file.
        #[clap(long)]
        cert: Option<PathBuf>,

        /// Path to key file.
        #[clap(long)]
        key: Option<PathBuf>,
    },
}

fn load_certs(path: &PathBuf) -> io::Result<Vec<Certificate>> {
    certs(&mut BufReader::new(File::open(path)?))
        .map_err(|_| io::Error::new(io::ErrorKind::InvalidInput, "invalid cert"))
        .map(|mut certs| certs.drain(..).map(Certificate).collect())
}

fn load_keys(path: &PathBuf) -> Result<PrivateKey> {
    let mut key_reader = std::io::BufReader::new(std::fs::File::open(path).unwrap());
    let key = rustls_pemfile::pkcs8_private_keys(&mut key_reader)
        .map_err(|_| error!("unable to load private key"))
        .unwrap()
        .remove(0);
    Ok(rustls::PrivateKey(key))
}

#[tokio::main]
async fn run(command: Command) -> Result<()> {
    match command {
        Command::Local {
            local_host,
            local_port,
            to,
            port,
            secret,
            tls,
            cafile,
        } => {
            info!("staring proxy client");
            loop {
                let client = if tls {
                    info!("using tls client");
                    let mut root_cert_store = rustls::RootCertStore::empty();
                    match &cafile {
                        Some(cafile) => {
                            let mut pem = BufReader::new(File::open(cafile)?);
                            let certs = rustls_pemfile::certs(&mut pem)?;
                            let trust_anchors = certs.iter().map(|cert| {
                                let ta = webpki::TrustAnchor::try_from_cert_der(&cert[..]).unwrap();
                                OwnedTrustAnchor::from_subject_spki_name_constraints(
                                    ta.subject,
                                    ta.spki,
                                    ta.name_constraints,
                                )
                            });
                            root_cert_store.add_server_trust_anchors(trust_anchors);
                        }
                        None => {
                            root_cert_store.add_server_trust_anchors(
                                webpki_roots::TLS_SERVER_ROOTS.0.iter().map(|ta| {
                                    OwnedTrustAnchor::from_subject_spki_name_constraints(
                                        ta.subject,
                                        ta.spki,
                                        ta.name_constraints,
                                    )
                                }),
                            );
                        }
                    }
                    let config = rustls::ClientConfig::builder()
                        .with_safe_defaults()
                        .with_root_certificates(root_cert_store)
                        .with_no_client_auth(); // i guess this was previously the default?
                    let connector = TlsConnector::from(Arc::new(config));

                    match Client::new_with_tls(
                        &local_host,
                        local_port,
                        &to,
                        port,
                        secret.as_deref(),
                        Some(connector),
                    )
                    .await
                    {
                        std::result::Result::Ok(client) => client,
                        Err(err) => {
                            error!("failed to create tls client: {:?}", err);
                            tokio::time::sleep(std::time::Duration::from_secs(1)).await;
                            continue;
                        }
                    }
                } else {
                    match Client::new(&local_host, local_port, &to, port, secret.as_deref()).await {
                        std::result::Result::Ok(client) => client,
                        Err(err) => {
                            error!("failed to create tcp client: {:?}", err);
                            tokio::time::sleep(std::time::Duration::from_secs(1)).await;
                            continue;
                        }
                    }
                };

                match client.listen().await {
                    std::result::Result::Ok(_) => info!("client exited"),
                    Err(err) => error!("client exited with error: {:?}", err),
                }

                error!("client exited");
                tokio::time::sleep(std::time::Duration::from_secs(1)).await;
            }
        }
        Command::Server {
            min_port,
            secret,
            tls,
            cert,
            key,
        } => {
            tokio::spawn(
                async move {
                    start_metric_server().await;
                }
                .instrument(info_span!("metrics")),
            );

            let server = if tls {
                let certs = load_certs(&cert.ok_or_else(|| {
                    io::Error::new(
                        io::ErrorKind::InvalidInput,
                        "cert path must be set, if tls is enabled",
                    )
                })?)?;
                let keys = load_keys(&key.unwrap())?;

                let config = rustls::ServerConfig::builder()
                    .with_safe_defaults()
                    .with_no_client_auth()
                    .with_single_cert(certs, keys)
                    .map_err(|err| io::Error::new(io::ErrorKind::InvalidInput, err))?;
                let acceptor = TlsAcceptor::from(Arc::new(config));
                Server::new_with_tls(min_port, secret.as_deref(), Some(acceptor))
            } else {
                Server::new(min_port, secret.as_deref())
            };
            server.listen().await?;
        }
    }

    Ok(())
}

fn main() -> Result<()> {
    tracing_subscriber::fmt::init();
    run(Args::parse().command)
}
