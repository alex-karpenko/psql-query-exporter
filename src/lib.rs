pub mod app_config;
pub mod db;
pub mod errors;
pub mod metrics;
pub mod scrape_config;
pub mod utils;

use axum::{response::Html, routing::get, Router};
use metrics::collectors_task;
use scrape_config::ScrapeConfig;
use std::{error::Error, net::SocketAddr};
use tokio::net::TcpListener;
use tracing::{info, instrument};
use utils::SignalHandler;

const HOME_PAGE_CONTENT: &str = include_str!("../assets/index.html");

#[instrument("RunExporter", skip_all)]
pub async fn run_exporter(
    scrape_config: ScrapeConfig,
    addr: SocketAddr,
    signal_handler: SignalHandler,
) -> Result<(), Box<dyn Error>> {
    let shutdown_channel_rx = signal_handler.get_rx_channel();

    info!("starting metrics collector task");
    let metrics_collector_task =
        tokio::task::spawn(collectors_task(scrape_config, shutdown_channel_rx.clone()));

    info!(address = %addr, "starting web server task");
    let http_server_task = tokio::task::spawn(web_server(addr, signal_handler));

    tokio::select! {
        _ = metrics_collector_task => {info!("all collectors have been finished")},
        _ = http_server_task => {info!("web server has been finished")},
    }

    Ok(())
}

#[instrument("WebServer", skip_all, fields(addr))]
async fn web_server(
    addr: SocketAddr,
    mut signal_handler: SignalHandler,
) -> Result<(), std::io::Error> {
    let app = Router::new()
        .route("/", get(Html(HOME_PAGE_CONTENT)))
        .route("/health", get("healthy\n"))
        .route("/metrics", get(metrics::compose_reply));

    let listener = TcpListener::bind(&addr)
        .await
        .unwrap_or_else(|_| panic!("unable to bind to address {:?}", addr));
    let server = axum::serve(listener, app).with_graceful_shutdown(async move {
        signal_handler.shutdown_on_signal().await;
    });

    server.await
}
