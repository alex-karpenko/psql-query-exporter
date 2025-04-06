use crate::{
    db::{PostgresConnectionString, PostgresSslMode},
    errors::PsqlExporterError,
};
use core::fmt::Display;
use regex::Regex;
use serde::Deserialize;
use std::{collections::HashMap, env, fs::read_to_string, time::Duration};

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
    sslrootcert: Option<String>,
    sslcert: Option<String>,
    sslkey: Option<String>,
    sslmode: PostgresSslMode,
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
    sslmode: Option<PostgresSslMode>,
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
    sslrootcert: Option<String>,
    sslcert: Option<String>,
    sslkey: Option<String>,
    pub databases: Vec<ScrapeConfigDatabase>,
}

#[derive(Deserialize, Debug)]
#[serde(deny_unknown_fields)]
pub struct ScrapeConfigDatabase {
    pub dbname: String,
    #[serde(skip)]
    pub connection_string: PostgresConnectionString,
    #[serde(skip)]
    pub sslmode: Option<PostgresSslMode>,
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
    pub sslrootcert: Option<String>,
    pub sslcert: Option<String>,
    pub sslkey: Option<String>,
    pub queries: Vec<ScrapeConfigQuery>,
}

#[derive(Deserialize, Debug)]
#[serde(deny_unknown_fields)]
pub struct ScrapeConfigQuery {
    pub query: String,
    pub metric_name: String,
    pub description: Option<String>,
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
    pub values: ScrapeConfigValues,
}

#[derive(Deserialize, Debug)]
#[serde(deny_unknown_fields, untagged)]
pub enum ScrapeConfigValues {
    #[serde(rename = "single")]
    ValueFrom { single: FieldWithType },
    #[serde(rename = "multi_labels")]
    ValuesWithLabels { multi_labels: Vec<FieldWithLabels> },
    #[serde(rename = "multi_suffixes")]
    ValuesWithSuffixes {
        multi_suffixes: Vec<FieldWithSuffix>,
    },
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

#[derive(Deserialize, Debug, Default)]
#[serde(deny_unknown_fields, rename_all = "lowercase")]
pub enum FieldType {
    #[default]
    Int,
    Float,
}

impl ScrapeConfig {
    pub fn from(filename: &String) -> Result<ScrapeConfig, PsqlExporterError> {
        let config = read_to_string(filename).map_err(|e| PsqlExporterError::LoadConfigFile {
            filename: filename.clone(),
            cause: e,
        })?;
        let mut config: ScrapeConfig = serde_yaml_ng::from_str(&config)?;

        config.defaults.merge_env_vars()?;
        for (_name, instance) in config.sources.iter_mut() {
            instance.merge_env_vars()?;
            instance.propagate_defaults(&config.defaults);
        }

        Ok(config)
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
            sslrootcert: None,
            sslcert: None,
            sslkey: None,
            sslmode: PostgresSslMode::default(),
        }
    }
}

impl ScrapeConfigDefaults {
    fn merge_env_vars(&mut self) -> Result<(), PsqlExporterError> {
        if let Some(rootcert) = self.sslrootcert.clone() {
            self.sslrootcert = Some(apply_envs_to_string(&rootcert)?);
        }
        if let Some(cert) = self.sslcert.clone() {
            self.sslcert = Some(apply_envs_to_string(&cert)?);
        }
        if let Some(key) = self.sslkey.clone() {
            self.sslkey = Some(apply_envs_to_string(&key)?);
        }

        Ok(())
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
                    self.metric_prefix.clone_from(&defaults.metric_prefix);
                    defaults.metric_prefix.clone()
                }
                _ => self.metric_prefix.clone(),
            },
            sslrootcert: match self.sslrootcert {
                None => {
                    self.sslrootcert.clone_from(&defaults.sslrootcert);
                    defaults.sslrootcert.clone()
                }
                _ => self.sslrootcert.clone(),
            },
            sslcert: match self.sslcert {
                None => {
                    self.sslcert.clone_from(&defaults.sslcert);
                    defaults.sslcert.clone()
                }
                _ => self.sslcert.clone(),
            },
            sslkey: match self.sslkey {
                None => {
                    self.sslkey.clone_from(&defaults.sslkey);
                    defaults.sslkey.clone()
                }
                _ => self.sslkey.clone(),
            },
            sslmode: match self.sslmode {
                None => {
                    self.sslmode = Some(defaults.sslmode.clone());
                    defaults.sslmode.clone()
                }
                _ => self.sslmode.clone().unwrap(),
            },
        };

        self.databases.iter_mut().for_each(|db| {
            let conn_string = PostgresConnectionString {
                host: self.host.clone(),
                port: self.port,
                user: self.user.clone(),
                password: self.password.clone(),
                sslmode: self.sslmode.clone().unwrap(),
                dbname: db.dbname.clone(),
            };
            db.propagate_defaults(&defaults, conn_string);
        });
    }

    fn merge_env_vars(&mut self) -> Result<(), PsqlExporterError> {
        self.host = apply_envs_to_string(&self.host)?;
        self.user = apply_envs_to_string(&self.user)?;
        self.password = apply_envs_to_string(&self.password)?;
        if let Some(rootcert) = self.sslrootcert.clone() {
            self.sslrootcert = Some(apply_envs_to_string(&rootcert)?);
        }
        if let Some(cert) = self.sslcert.clone() {
            self.sslcert = Some(apply_envs_to_string(&cert)?);
        }
        if let Some(key) = self.sslkey.clone() {
            self.sslkey = Some(apply_envs_to_string(&key)?);
        }

        Ok(())
    }
}

impl ScrapeConfigDatabase {
    fn propagate_defaults(
        &mut self,
        defaults: &ScrapeConfigDefaults,
        connection_string: PostgresConnectionString,
    ) {
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
                    self.metric_prefix.clone_from(&defaults.metric_prefix);
                    defaults.metric_prefix.clone()
                }
                _ => self.metric_prefix.clone(),
            },
            sslrootcert: match self.sslrootcert {
                None => {
                    self.sslrootcert.clone_from(&defaults.sslrootcert);
                    defaults.sslrootcert.clone()
                }
                _ => self.sslrootcert.clone(),
            },
            sslcert: match self.sslcert {
                None => {
                    self.sslcert.clone_from(&defaults.sslcert);
                    defaults.sslcert.clone()
                }
                _ => self.sslcert.clone(),
            },
            sslkey: match self.sslkey {
                None => {
                    self.sslkey.clone_from(&defaults.sslkey);
                    defaults.sslkey.clone()
                }
                _ => self.sslkey.clone(),
            },
            sslmode: match self.sslmode {
                None => {
                    self.sslmode = Some(defaults.sslmode.clone());
                    defaults.sslmode.clone()
                }
                _ => self.sslmode.clone().unwrap(),
            },
        };

        self.queries.iter_mut().for_each(|q| {
            q.propagate_defaults(&defaults);
        });
    }
}

impl Display for ScrapeConfigDatabase {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "host: {}, port: {}, user: {}, dbname: {}",
            self.connection_string.host,
            self.connection_string.port,
            self.connection_string.user,
            self.connection_string.dbname
        )
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

        if self.description.is_none() {
            self.description = Some(self.metric_name.clone())
        }
    }
}

impl Default for ScrapeConfigValues {
    fn default() -> Self {
        Self::ValueFrom {
            single: FieldWithType {
                field: None,
                field_type: FieldType::Int,
            },
        }
    }
}

fn apply_envs_to_string(text: &str) -> Result<String, PsqlExporterError> {
    let re = Regex::new(r"\$\{[a-zA-Z][A-Za-z0-9_]*\}")
        .unwrap_or_else(|e| panic!("looks like a BUG: {e}"));
    let mut result = text.to_owned();
    for item in re.captures_iter(text) {
        let env_name = item.get(0).expect("looks like a BUG").as_str().to_string();
        let env_name = env_name.trim_start_matches("${").trim_end_matches('}');
        let env_value =
            env::var(env_name).map_err(|e| PsqlExporterError::EnvironmentVariableSubstitution {
                variable: env_name.to_string(),
                cause: e,
            })?;
        result = re.replace_all(&result, env_value).to_string();
    }

    Ok(result)
}
