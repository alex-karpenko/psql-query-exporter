mod app_config;
mod db;
mod errors;
mod metrics;
mod scrape_config;
mod utils;

use app_config::AppConfig;
use scrape_config::ScrapeConfig;
use utils::SignalHandler;

use std::error::Error;
use tracing::{info, instrument};

use warp::Filter;

const HOME_PAGE_CONTENT: &str = include_str!("../assets/index.html");

#[tokio::main]
#[instrument]
async fn main() -> Result<(), Box<dyn Error>> {
    let app_config = AppConfig::new();
    let scrape_config = ScrapeConfig::from(&app_config.config)?;

    // GET /
    let home_route = warp::path::end().map(|| warp::reply::html(HOME_PAGE_CONTENT));
    // GET /health
    let health_route = warp::path("health").map(|| "healthy\n");
    // GET /metrics
    let metrics_route = warp::path("metrics").and_then(metrics::compose_reply);
    let routes = warp::get().and(health_route.or(metrics_route).or(home_route));

    let mut signal_handler = SignalHandler::new()?;
    let shutdown_channel_rx = signal_handler.get_rx_channel();

    let (_addr, http_server) = warp::serve(routes).bind_with_graceful_shutdown(
        (app_config.listen_on, app_config.port),
        async move {
            signal_handler.shutdown_on_signal().await;
        },
    );

    let metrics_collecting_task = tokio::task::spawn(metrics::collecting_task(
        scrape_config,
        shutdown_channel_rx.clone(),
    ));
    let http_server_task = tokio::task::spawn(http_server);

    tokio::select! {
        _ = metrics_collecting_task => {info!("all collecting tasks have been finished")},
        _ = http_server_task => {info!("web server has been finished")},
    }

    Ok(())
}
