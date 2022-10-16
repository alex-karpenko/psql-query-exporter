use clap::Parser;
use std::{net::Ipv4Addr, path::PathBuf, str::FromStr};

use tracing::debug;
use tracing_subscriber::{
    filter::{EnvFilter, LevelFilter},
    fmt,
};

#[derive(Parser, Debug)]
#[clap(author, version, about)]
pub struct AppConfig {
    /// Enable extrime logging (debug)
    #[clap(short, long)]
    pub debug: bool,

    /// Enable additional logging (info)
    #[clap(short, long)]
    pub verbose: bool,

    /// IP/hostname to listen on
    #[clap(long, default_value_t = Ipv4Addr::new(0, 0, 0, 0), value_parser = AppConfig::parse_listen_to_ip)]
    pub listen_on: Ipv4Addr,

    /// Port to serve http on
    #[clap(long, default_value_t = 9090, value_parser = clap::value_parser!(u16).range(1..=65535))]
    pub port: u16,

    /// Path to config file
    #[clap(long, short)]
    pub config: PathBuf,
}

impl AppConfig {
    pub fn new() -> AppConfig {
        let config: AppConfig = Parser::parse();

        AppConfig::configure_logging(&config);
        debug!("Application config: {:?}", config);

        config
    }

    fn parse_listen_to_ip(ip: &str) -> Result<Ipv4Addr, String> {
        Ipv4Addr::from_str(ip).map_err(|_| String::from("unable to parse IP address"))
    }

    fn configure_logging(config: &AppConfig) {
        let level_filter = if config.debug {
            LevelFilter::DEBUG
        } else if config.verbose {
            LevelFilter::INFO
        } else {
            LevelFilter::WARN
        };

        let log_filter = EnvFilter::from_default_env().add_directive(level_filter.into());
        let log_format = fmt::format().with_level(true).with_target(true).compact();

        tracing_subscriber::fmt()
            .event_format(log_format)
            .with_env_filter(log_filter)
            .init();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_correct_ip() {
        assert_eq!(
            AppConfig::parse_listen_to_ip("1.2.3.4"),
            Ok(Ipv4Addr::new(1, 2, 3, 4))
        );
        assert_eq!(
            AppConfig::parse_listen_to_ip("0.0.0.0"),
            Ok(Ipv4Addr::new(0, 0, 0, 0))
        );
    }

    #[test]
    fn parse_incorrect_ip() {
        assert_eq!(
            AppConfig::parse_listen_to_ip(".0.0.0"),
            Err(String::from("unable to parse IP address"))
        );

        assert_eq!(
            AppConfig::parse_listen_to_ip("qwert"),
            Err(String::from("unable to parse IP address"))
        );

        assert_eq!(
            AppConfig::parse_listen_to_ip("1.2.3.444"),
            Err(String::from("unable to parse IP address"))
        );
    }
}
