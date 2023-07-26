//! Metrics for the server

use lazy_static::lazy_static;
use prometheus::{Counter, IntCounter, IntGauge, Opts, Registry};
use tracing::info;
use warp::Filter;

lazy_static! {
    /// Count of total control channel connections
    pub static ref TOTAL_CONNECTIONS: IntGauge = IntGauge::new("total_connections", "Total TCP connections").expect("metric can be created");

    /// Count of total client connections
    pub static ref CONNECTED_CLIENTS: IntGauge = IntGauge::new("connected_clients", "Connected Clients").expect("metric can be created");

    /// Count of heartbets sent
    pub static ref HEARTBEATS: IntCounter = IntCounter::new("heartbeats", "Count of total Heartbeats sent").expect("metric can be created");

    /// Metric for incoming bytes
    pub static ref INCOMING_BYTES: Counter =
        Counter::with_opts(Opts::new("incoming_bytes", "Total incoming bytes"))
            .expect("metric can be created");

    /// Metric for outgoing bytes
    pub static ref OUTGOING_BYTES: Counter =
        Counter::with_opts(Opts::new("outgoing_bytes", "Total outgoing bytes"))
            .expect("metric can be created");

    /// Main registry for prometheus
    pub static ref REGISTRY: Registry = Registry::new();
}

/// Function to return metrics in prometheus format
pub fn metrics_handler() -> String {
    use prometheus::Encoder;
    let encoder = prometheus::TextEncoder::new();

    let mut buffer = Vec::new();
    if let Err(e) = encoder.encode(&REGISTRY.gather(), &mut buffer) {
        eprintln!("could not encode custom metrics: {}", e);
    };
    let mut res = String::from_utf8(buffer.clone()).unwrap();
    buffer.clear();

    let mut buffer = Vec::new();
    if let Err(e) = encoder.encode(&prometheus::gather(), &mut buffer) {
        eprintln!("could not encode prometheus metrics: {}", e);
    };
    let res_custom = String::from_utf8(buffer.clone()).unwrap();
    buffer.clear();

    res.push_str(&res_custom);
    res
}

/// Function to start the metric http server
pub async fn start_metric_server() {
    info!("starting metric server");

    register_metrics();

    let routes = warp::path("metrics").map(metrics_handler);
    warp::serve(routes).run(([127, 0, 0, 1], 1234)).await;
}

/// Function to register the prometheus metrics
fn register_metrics() {
    REGISTRY
        .register(Box::new(TOTAL_CONNECTIONS.clone()))
        .expect("failed to register metric");

    REGISTRY
        .register(Box::new(CONNECTED_CLIENTS.clone()))
        .expect("failed to register metric");

    REGISTRY
        .register(Box::new(HEARTBEATS.clone()))
        .expect("failed to register metric");

    REGISTRY
        .register(Box::new(INCOMING_BYTES.clone()))
        .expect("failed to register metric");

    REGISTRY
        .register(Box::new(OUTGOING_BYTES.clone()))
        .expect("failed to register metric");
}
