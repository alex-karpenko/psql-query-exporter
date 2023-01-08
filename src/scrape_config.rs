use crate::db::PostgresSslMode;

use figment::{
    providers::{Format, Yaml},
    Error, Figment,
};

use regex::Regex;
use serde::Deserialize;

use std::{collections::HashMap, env, path::PathBuf, time::Duration};

const DEFAULT_SCRAPE_INTERVAL: Duration = Duration::from_secs(1800);
const DEFAULT_QUERY_TIMEOUT: Duration = Duration::from_secs(10);
const DEFAULT_METRIC_EXPIRATION_TIME: Duration = Duration::ZERO;
const DB_CONNECTION_DEFAULT_BACKOFF_INTERVAL: Duration = Duration::from_secs(10);
const DB_CONNECTION_MAXIMUM_BACKOFF_INTERVAL: Duration = Duration::from_secs(300);

#[derive(Deserialize, Debug)]
#[serde(deny_unknown_fields)]
pub struct ScrapeConfig {
    #[serde(default)]
    defaults: ScrapeConfigDefaults,
    pub sources: HashMap<String, ScrapeConfigSource>,
}

#[derive(Deserialize, Debug)]
#[serde(deny_unknown_fields, default)]
struct ScrapeConfigDefaults {
    #[serde(with = "humantime_serde")]
    scrape_interval: Duration,
    #[serde(with = "humantime_serde")]
    query_timeout: Duration,
    #[serde(with = "humantime_serde")]
    backoff_interval: Duration,
    #[serde(with = "humantime_serde")]
    max_backoff_interval: Duration,
    #[serde(with = "humantime_serde")]
    metric_expiration_time: Duration,
    metric_prefix: Option<String>,
    ssl_verify: Option<bool>,
}

#[derive(Deserialize, Debug)]
#[serde(deny_unknown_fields)]
pub struct ScrapeConfigSource {
    host: String,
    #[serde(default = "ScrapeConfigSource::default_port")]
    port: u16,
    user: String,
    password: String,
    #[serde(default)]
    sslmode: PostgresSslMode,
    #[serde(with = "humantime_serde", default)]
    scrape_interval: Duration,
    #[serde(with = "humantime_serde", default)]
    query_timeout: Duration,
    #[serde(with = "humantime_serde", default)]
    backoff_interval: Duration,
    #[serde(with = "humantime_serde", default)]
    max_backoff_interval: Duration,
    #[serde(with = "humantime_serde", default)]
    metric_expiration_time: Duration,
    metric_prefix: Option<String>,
    ssl_verify: Option<bool>,
    pub databases: Vec<ScrapeConfigDatabase>,
}

#[derive(Deserialize, Debug)]
#[serde(deny_unknown_fields)]
pub struct ScrapeConfigDatabase {
    pub dbname: String,
    #[serde(skip)]
    pub connection_string: String,
    #[serde(with = "humantime_serde", default)]
    scrape_interval: Duration,
    #[serde(with = "humantime_serde", default)]
    query_timeout: Duration,
    #[serde(with = "humantime_serde", default)]
    pub backoff_interval: Duration,
    #[serde(with = "humantime_serde", default)]
    pub max_backoff_interval: Duration,
    #[serde(with = "humantime_serde", default)]
    metric_expiration_time: Duration,
    metric_prefix: Option<String>,
    #[serde(skip)]
    pub ssl_verify: Option<bool>,
    pub queries: Vec<ScrapeConfigQuery>,
}

#[derive(Deserialize, Debug)]
#[serde(deny_unknown_fields)]
pub struct ScrapeConfigQuery {
    pub query: String,
    pub metric_name: String,
    metric_prefix: Option<String>,
    #[serde(with = "humantime_serde", default)]
    pub scrape_interval: Duration,
    #[serde(with = "humantime_serde", default)]
    pub query_timeout: Duration,
    #[serde(with = "humantime_serde", default)]
    pub metric_expiration_time: Duration,
    #[serde(default)]
    pub const_labels: Option<HashMap<String, String>>,
    #[serde(default)]
    pub var_labels: Option<Vec<String>>,
    #[serde(default)]
    pub values: ScrapeConfigValues, // These two vectors have the same size
}

#[derive(Deserialize, Debug)]
#[serde(deny_unknown_fields)]
pub enum ScrapeConfigValues {
    #[serde(rename = "single")]
    ValueFrom(FieldWithType),
    #[serde(rename = "multi_labels")]
    ValuesWithLabels(Vec<FieldWithLabels>),
    #[serde(rename = "multi_suffixes")]
    ValuesWithSuffixes(Vec<FieldWithSuffix>),
}

#[derive(Deserialize, Debug)]
#[serde(deny_unknown_fields)]
pub struct FieldWithType {
    pub field: Option<String>,
    #[serde(rename = "type", default)]
    pub field_type: FieldType,
}

#[derive(Deserialize, Debug)]
#[serde(deny_unknown_fields)]
pub struct FieldWithLabels {
    pub field: String,
    #[serde(rename = "type", default)]
    pub field_type: FieldType,
    pub labels: HashMap<String, String>,
}

#[derive(Deserialize, Debug)]
#[serde(deny_unknown_fields)]
pub struct FieldWithSuffix {
    pub field: String,
    #[serde(rename = "type", default)]
    pub field_type: FieldType,
    pub suffix: String,
}

#[derive(Deserialize, Debug)]
#[serde(deny_unknown_fields, rename_all = "lowercase")]
pub enum FieldType {
    Int,
    Float,
}

impl ScrapeConfig {
    pub fn from(filename: &PathBuf) -> ScrapeConfig {
        let config: Result<ScrapeConfig, Error> =
            Figment::new().merge(Yaml::file(filename)).extract();

        let mut config = match config {
            Ok(config) => config,
            Err(e) => panic!(
                "error parsing config file {filename}: {e}",
                filename = filename.to_str().expect("looks like a BUG")
            ),
        };

        config.sources.iter_mut().for_each(|(_name, instance)| {
            instance.merge_env_vars();
            instance.propagate_defaults(&config.defaults);
        });

        config
    }

    pub fn len(&self) -> usize {
        self.sources.len()
    }
}

impl Default for ScrapeConfigDefaults {
    fn default() -> Self {
        Self {
            scrape_interval: DEFAULT_SCRAPE_INTERVAL,
            query_timeout: DEFAULT_QUERY_TIMEOUT,
            backoff_interval: DB_CONNECTION_DEFAULT_BACKOFF_INTERVAL,
            max_backoff_interval: DB_CONNECTION_MAXIMUM_BACKOFF_INTERVAL,
            metric_expiration_time: DEFAULT_METRIC_EXPIRATION_TIME,
            metric_prefix: None,
            ssl_verify: None,
        }
    }
}

impl ScrapeConfigSource {
    fn default_port() -> u16 {
        5432
    }

    fn propagate_defaults(&mut self, defaults: &ScrapeConfigDefaults) {
        let defaults = ScrapeConfigDefaults {
            scrape_interval: if self.scrape_interval == Duration::default() {
                self.scrape_interval = defaults.scrape_interval;
                defaults.scrape_interval
            } else {
                self.scrape_interval
            },
            query_timeout: if self.query_timeout == Duration::default() {
                self.query_timeout = defaults.query_timeout;
                defaults.query_timeout
            } else {
                self.query_timeout
            },
            backoff_interval: if self.backoff_interval == Duration::default() {
                self.backoff_interval = defaults.backoff_interval;
                defaults.backoff_interval
            } else {
                self.backoff_interval
            },
            max_backoff_interval: if self.max_backoff_interval == Duration::default() {
                self.max_backoff_interval = defaults.max_backoff_interval;
                defaults.max_backoff_interval
            } else {
                self.max_backoff_interval
            },
            metric_expiration_time: if self.metric_expiration_time == Duration::default() {
                self.metric_expiration_time = defaults.metric_expiration_time;
                defaults.metric_expiration_time
            } else {
                self.metric_expiration_time
            },
            metric_prefix: match self.metric_prefix {
                None => {
                    self.metric_prefix = defaults.metric_prefix.clone();
                    defaults.metric_prefix.clone()
                }
                _ => self.metric_prefix.clone(),
            },
            ssl_verify: match self.ssl_verify {
                None => {
                    self.ssl_verify = defaults.ssl_verify;
                    defaults.ssl_verify
                }
                _ => self.ssl_verify,
            },
        };

        self.databases.iter_mut().for_each(|db| {
            let conn_string = format!("host={host} port={port} dbname={dbname} user={user} password='{password}' sslmode={sslmode}", host=self.host, port=self.port, user=self.user, password=self.password, sslmode=self.sslmode, dbname=db.dbname);
            db.propagate_defaults(&defaults, conn_string);
        });
    }

    fn merge_env_vars(&mut self) {
        self.host = self.apply_envs_to_string(&self.host);
        self.user = self.apply_envs_to_string(&self.user);
        self.password = self.apply_envs_to_string(&self.password);
    }

    fn apply_envs_to_string(&self, text: &str) -> String {
        let re = Regex::new(r"\$\{[a-zA-Z][A-Za-z0-9_]*\}")
            .unwrap_or_else(|e| panic!("looks like a BUG: {e}"));
        let mut result = text.to_owned();
        for item in re.captures_iter(text) {
            let env_name = item.get(0).expect("looks like a BUG").as_str().to_string();
            let env_name = env_name.trim_start_matches("${").trim_end_matches('}');
            let env_value = env::var(env_name)
                .unwrap_or_else(|_| panic!("environment variable '{env_name}' expected"));
            result = re.replace_all(&result, env_value).to_string();
        }

        result
    }
}

impl ScrapeConfigDatabase {
    fn propagate_defaults(&mut self, defaults: &ScrapeConfigDefaults, connection_string: String) {
        self.connection_string = connection_string;
        let defaults = ScrapeConfigDefaults {
            scrape_interval: if self.scrape_interval == Duration::default() {
                self.scrape_interval = defaults.scrape_interval;
                defaults.scrape_interval
            } else {
                self.scrape_interval
            },
            query_timeout: if self.query_timeout == Duration::default() {
                self.query_timeout = defaults.query_timeout;
                defaults.query_timeout
            } else {
                self.query_timeout
            },
            backoff_interval: if self.backoff_interval == Duration::default() {
                self.backoff_interval = defaults.backoff_interval;
                defaults.backoff_interval
            } else {
                self.backoff_interval
            },
            max_backoff_interval: if self.max_backoff_interval == Duration::default() {
                self.max_backoff_interval = defaults.max_backoff_interval;
                defaults.max_backoff_interval
            } else {
                self.max_backoff_interval
            },
            metric_expiration_time: if self.metric_expiration_time == Duration::default() {
                self.metric_expiration_time = defaults.metric_expiration_time;
                defaults.metric_expiration_time
            } else {
                self.metric_expiration_time
            },
            metric_prefix: match self.metric_prefix {
                None => {
                    self.metric_prefix = defaults.metric_prefix.clone();
                    defaults.metric_prefix.clone()
                }
                _ => self.metric_prefix.clone(),
            },
            ssl_verify: match self.ssl_verify {
                None => {
                    self.ssl_verify = defaults.ssl_verify;
                    defaults.ssl_verify
                }
                _ => self.ssl_verify,
            },
        };

        self.queries.iter_mut().for_each(|q| {
            q.propagate_defaults(&defaults);
        });
    }
}

impl ScrapeConfigQuery {
    fn propagate_defaults(&mut self, defaults: &ScrapeConfigDefaults) {
        self.scrape_interval = if self.scrape_interval == Duration::default() {
            defaults.scrape_interval
        } else {
            self.scrape_interval
        };
        self.query_timeout = if self.query_timeout == Duration::default() {
            defaults.query_timeout
        } else {
            self.query_timeout
        };
        self.metric_expiration_time = if self.metric_expiration_time == Duration::default() {
            defaults.metric_expiration_time
        } else {
            self.metric_expiration_time
        };
        self.metric_prefix = match self.metric_prefix {
            None => defaults.metric_prefix.clone(),
            _ => self.metric_prefix.clone(),
        };

        if let Some(prefix) = &self.metric_prefix {
            self.metric_name = format!("{}_{}", prefix, self.metric_name);
        }
    }
}

impl Default for ScrapeConfigValues {
    fn default() -> Self {
        Self::ValueFrom(FieldWithType {
            field: None,
            field_type: FieldType::Int,
        })
    }
}

impl Default for FieldType {
    fn default() -> Self {
        Self::Int
    }
}
