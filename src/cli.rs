use clap::Parser;
use std::{net::Ipv4Addr, str::FromStr};
use tracing_subscriber::{filter::EnvFilter, fmt};

const INVALID_IP_ADDRESS_ERROR: &str = "IP address isn't valid";

#[derive(Parser, Debug)]
#[clap(author, version, about)]
pub struct CliParams {
    /// Write logs in JSON format
    #[clap(short, long)]
    pub json_log: bool,

    /// IP/hostname to listen on
    #[clap(short, long, default_value_t = Ipv4Addr::new(0, 0, 0, 0), value_parser = CliParams::parse_ip_address)]
    pub listen_on: Ipv4Addr,

    /// Port to serve http on
    #[clap(short, long, default_value_t = 9090, value_parser = clap::value_parser!(u16).range(1..=65535))]
    pub port: u16,

    /// Path to the config file
    #[clap(long, short)]
    pub config: String,
}

impl CliParams {
    #[allow(clippy::new_without_default)]
    pub fn new() -> CliParams {
        let config: CliParams = Parser::parse();

        config.setup_logger();
        config
    }

    fn setup_logger(&self) {
        let log_filter = EnvFilter::from_default_env();
        let log_format = fmt::format().with_level(true);

        let subscriber = tracing_subscriber::fmt().with_env_filter(log_filter);
        if self.json_log {
            subscriber
                .event_format(log_format.json().flatten_event(true))
                .init();
        } else {
            subscriber.event_format(log_format.compact()).init();
        };
    }

    fn parse_ip_address(ip: &str) -> Result<Ipv4Addr, String> {
        Ipv4Addr::from_str(ip).map_err(|_| String::from(INVALID_IP_ADDRESS_ERROR))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_correct_ip() {
        assert_eq!(
            CliParams::parse_ip_address("1.2.3.4"),
            Ok(Ipv4Addr::new(1, 2, 3, 4))
        );
        assert_eq!(
            CliParams::parse_ip_address("0.0.0.0"),
            Ok(Ipv4Addr::new(0, 0, 0, 0))
        );
    }

    #[test]
    fn parse_incorrect_ip() {
        assert_eq!(
            CliParams::parse_ip_address(".0.0.0"),
            Err(String::from(INVALID_IP_ADDRESS_ERROR))
        );

        assert_eq!(
            CliParams::parse_ip_address("qwerty"),
            Err(String::from(INVALID_IP_ADDRESS_ERROR))
        );

        assert_eq!(
            CliParams::parse_ip_address("1.2.3.444"),
            Err(String::from(INVALID_IP_ADDRESS_ERROR))
        );
    }
}
