mod app_config;
mod db;
mod metrics;
mod scrape_config;

use app_config::AppConfig;
use scrape_config::ScrapeConfig;
use std::error::Error;
use tracing::instrument;

use tokio::signal;
use warp::Filter;

const HOME_PAGE_CONTENT: &str = include_str!("../assets/index.html");

#[tokio::main]
#[instrument]
async fn main() -> Result<(), Box<dyn Error>> {
    let app_config = AppConfig::new();
    let scrape_config = ScrapeConfig::from(&app_config.config);

    // GET /
    let home_route = warp::path::end().map(|| warp::reply::html(HOME_PAGE_CONTENT));
    // GET /health
    let health_route = warp::path("health").map(|| "healthy\n");
    // GET /metrics
    let metrics_route = warp::path("metrics").and_then(metrics::compose_reply);

    let routes = warp::get().and(health_route.or(metrics_route).or(home_route));
    let (_addr, http_server) = warp::serve(routes)
        .bind_with_graceful_shutdown((app_config.listen_on, app_config.port), async {
            shutdown().await
        });

    let metrics_collecting_task = tokio::task::spawn(metrics::collecting_task(scrape_config));
    let http_server_task = tokio::task::spawn(http_server);

    tokio::select! {
        _ = metrics_collecting_task => {},
        _ = http_server_task => {},
        _ = shutdown() => {},
    }

    Ok(())
}

async fn shutdown() {
    // Wait for the CTRL+C signal
    signal::ctrl_c()
        .await
        .expect("failed to install termination signal handler");
}
