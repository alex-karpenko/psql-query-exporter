use crate::db::PostgresSslMode;
use crate::metrics::MetricWithType;

use figment::{
    providers::{Format, Yaml},
    Error, Figment,
};
use prometheus::{
    opts, register_gauge, register_gauge_vec, register_int_gauge, register_int_gauge_vec,
};

use regex::Regex;
use serde::Deserialize;

use std::{
    collections::HashMap,
    env,
    path::PathBuf,
    time::{Duration, SystemTime},
};

const DEFAULT_SCRAPE_INTERVAL: Duration = Duration::from_secs(1800);
const DEFAULT_QUERY_TIMEOUT: Duration = Duration::from_secs(10);

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
    #[serde(default)]
    const_labels: Option<HashMap<String, String>>,
    #[serde(default)]
    pub var_labels: Option<Vec<String>>,
    #[serde(default)]
    pub values: ScrapeConfigValues, // These two vectors have the same size
    #[serde(skip)] // because they represent array of possible
    pub metric: Vec<MetricWithType>, // metrics with respect to possible multi-values query result
    #[serde(skip, default = "SystemTime::now")]
    pub next_query_time: SystemTime,
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
    field_type: FieldType,
}

#[derive(Deserialize, Debug)]
#[serde(deny_unknown_fields)]
pub struct FieldWithLabels {
    pub field: String,
    #[serde(rename = "type", default)]
    field_type: FieldType,
    pub labels: HashMap<String, String>,
}

#[derive(Deserialize, Debug)]
#[serde(deny_unknown_fields)]
pub struct FieldWithSuffix {
    pub field: String,
    #[serde(rename = "type", default)]
    field_type: FieldType,
    pub suffix: String,
}

#[derive(Deserialize, Debug)]
#[serde(deny_unknown_fields, rename_all = "lowercase")]
enum FieldType {
    Int,
    Float,
}

impl ScrapeConfig {
    pub fn new(filename: &PathBuf) -> ScrapeConfig {
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
        let re = Regex::new(r"\$\{[a-zA-Z][A-Za-z0-9_]*\}").expect("looks like a BUG");
        let mut result = text.to_owned();
        for item in re.captures_iter(text) {
            let env_name = item.get(0).expect("looks like a BUG").as_str().to_string();
            let env_name = env_name.trim_start_matches("${").trim_end_matches('}');
            let env_value = env::var(env_name)
                .expect(format!("environment variable '{env_name}' expected").as_str());
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
            q.prepare_metric();
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
        self.metric_prefix = match self.metric_prefix {
            None => defaults.metric_prefix.clone(),
            _ => self.metric_prefix.clone(),
        };

        if let Some(prefix) = &self.metric_prefix {
            self.metric_name = format!("{}_{}", prefix, self.metric_name);
        }
    }

    fn prepare_metric(&mut self) {
        if self.metric.is_empty() {
            match &self.values {
                ScrapeConfigValues::ValueFrom(values) => {
                    let mut opts = opts!(self.metric_name.clone(), self.metric_name.clone());
                    if let Some(const_labels) = &self.const_labels {
                        opts = opts.const_labels(const_labels.clone());
                    }
                    if let Some(var_labels) = &self.var_labels {
                        let new_labels: Vec<&str> = var_labels.iter().map(AsRef::as_ref).collect();
                        self.metric = match values.field_type {
                            FieldType::Int => vec![MetricWithType::VectorInt(
                                register_int_gauge_vec!(opts, &new_labels).expect(
                                    format!("error while registering metric {}", self.metric_name)
                                        .as_str(),
                                ),
                            )],
                            FieldType::Float => vec![MetricWithType::VectorFloat(
                                register_gauge_vec!(opts, &new_labels).expect(
                                    format!("error while registering metric {}", self.metric_name)
                                        .as_str(),
                                ),
                            )],
                        }
                    } else {
                        self.metric = match values.field_type {
                            FieldType::Int => vec![MetricWithType::SingleInt(
                                register_int_gauge!(opts).expect(
                                    format!("error while registering metric {}", self.metric_name)
                                        .as_str(),
                                ),
                            )],
                            FieldType::Float => {
                                vec![MetricWithType::SingleFloat(
                                    register_gauge!(opts).expect(
                                        format!(
                                            "error while registering metric {}",
                                            self.metric_name
                                        )
                                        .as_str(),
                                    ),
                                )]
                            }
                        };
                    }
                }

                ScrapeConfigValues::ValuesWithLabels(values) => {
                    self.metric = vec![];

                    for value in values {
                        let mut opts = opts!(self.metric_name.clone(), self.metric_name.clone());
                        if let Some(const_labels) = &self.const_labels {
                            let mut const_labels = const_labels.clone();
                            value.labels.iter().for_each(|(k, v)| {
                                const_labels.insert(k.to_string(), v.to_string());
                            });
                            opts = opts.const_labels(const_labels);
                        }
                        let new_metric;
                        if let Some(var_labels) = &self.var_labels {
                            let new_labels: Vec<&str> =
                                var_labels.iter().map(AsRef::as_ref).collect();
                            new_metric = match value.field_type {
                                FieldType::Int => MetricWithType::VectorInt(
                                    register_int_gauge_vec!(opts, &new_labels).expect(
                                        format!(
                                            "error while registering metric {}",
                                            self.metric_name
                                        )
                                        .as_str(),
                                    ),
                                ),
                                FieldType::Float => MetricWithType::VectorFloat(
                                    register_gauge_vec!(opts, &new_labels).expect(
                                        format!(
                                            "error while registering metric {}",
                                            self.metric_name
                                        )
                                        .as_str(),
                                    ),
                                ),
                            }
                        } else {
                            new_metric = match value.field_type {
                                FieldType::Int => MetricWithType::SingleInt(
                                    register_int_gauge!(opts).expect(
                                        format!(
                                            "error while registering metric {}",
                                            self.metric_name
                                        )
                                        .as_str(),
                                    ),
                                ),
                                FieldType::Float => MetricWithType::SingleFloat(
                                    register_gauge!(opts).expect(
                                        format!(
                                            "error while registering metric {}",
                                            self.metric_name
                                        )
                                        .as_str(),
                                    ),
                                ),
                            };
                        }

                        self.metric.push(new_metric);
                    }
                }

                ScrapeConfigValues::ValuesWithSuffixes(values) => {
                    self.metric = vec![];

                    for value in values {
                        let metric_name = format!("{}_{}", self.metric_name, value.suffix);
                        let mut opts = opts!(metric_name.clone(), metric_name.clone());
                        if let Some(const_labels) = &self.const_labels {
                            opts = opts.const_labels(const_labels.clone());
                        }
                        let new_metric;
                        if let Some(var_labels) = &self.var_labels {
                            let new_labels: Vec<&str> =
                                var_labels.iter().map(AsRef::as_ref).collect();
                            new_metric = match value.field_type {
                                FieldType::Int => MetricWithType::VectorInt(
                                    register_int_gauge_vec!(opts, &new_labels).expect(
                                        format!("error while registering metric {}", metric_name)
                                            .as_str(),
                                    ),
                                ),
                                FieldType::Float => MetricWithType::VectorFloat(
                                    register_gauge_vec!(opts, &new_labels).expect(
                                        format!("error while registering metric {}", metric_name)
                                            .as_str(),
                                    ),
                                ),
                            }
                        } else {
                            new_metric = match value.field_type {
                                FieldType::Int => MetricWithType::SingleInt(
                                    register_int_gauge!(opts).expect(
                                        format!("error while registering metric {}", metric_name)
                                            .as_str(),
                                    ),
                                ),
                                FieldType::Float => MetricWithType::SingleFloat(
                                    register_gauge!(opts).expect(
                                        format!("error while registering metric {}", metric_name)
                                            .as_str(),
                                    ),
                                ),
                            };
                        }

                        self.metric.push(new_metric);
                    }
                }
            };
        };
    }

    pub fn schedule_next_query_time(&self) -> SystemTime {
        SystemTime::now() + self.scrape_interval
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
