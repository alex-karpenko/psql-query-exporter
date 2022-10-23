# psql-query-exporter

[Prometheus](https://prometheus.io/docs/introduction/overview/) exporter to produce metrics from [PostgreSQL](https://www.postgresql.org/) queries on periodic basis.

## Features

- Allows querying of multiple DB instances (hosts) and multiple databases within each instace independently.
- Flexible configuration of quieris, sources (fields) of metric's values, labels, queruing intervals, etc.
- Fast, lightweight and scalable.

## Usage

The easiest way to run exporter is to use [Docker image](#docker-image). If you use Kubernetes to run workload you can use [Helm chart](#helm-chart) to configure and deploy exporter. The third way to run exporter is to [build native Rust binary](#build-your-own-binary) using Cargo utility and run it.

Anyway, to run exporter we need a [configuration file](#configuration) with definition of the scraping targets: hosts, databases, queries, labels, etc.

### Docker image

Just use the following command to get usage help, the same as running it with `--help` command line option:

```console
docker run --rm alexkarpenko/psql-query-exporter:latest
```

Typical output is:

```console
psql-query-exporter 0.1.0
Oleksii Karpenko <alexkarpenko@yahoo.com>
PostgreSQL Query Prometheus exporter

USAGE:
    psql-query-exporter [OPTIONS] --config <CONFIG>

OPTIONS:
    -c, --config <CONFIG>          Path to config file
    -d, --debug                    Enable extrime logging (debug)
    -h, --help                     Print help information
        --listen-on <LISTEN_ON>    IP/hostname to listen on [default: 0.0.0.0]
        --port <PORT>              Port to serve http on [default: 9090]
    -v, --verbose                  Enable additional logging (info)
    -V, --version                  Print version information
```

The only mandatory parameter is a path to configuration file. Datailed explanation of all possible configuration options is in the dedicated [Configuration](#configuration) section. Just for test purpose, there is an [example config](config.yaml) file to query PosygreSQL server at `localhost` for replication lag values. To use it:

```console
docker run --rm --name psql-query-exporter -v $PWD/config.yaml:/config.yaml -e PG_USER=postgres -e PG_PASSWORD=postgres alexkarpenko/psql-query-exporter:latest --config /config.yaml -v
```

### Helm chart

Just create your own values file with overrides of the default values and your own config section and deploy Helm release to your K8s cluster:

```console
helm install psql-query-exporter ./helm/psql-query-exporter -f my-values.yaml
```

For example your values can be like below. Don't forget to create secret `psql-query-exporter` with two keys `PG_USER` and `PG_PASSWORD` with username and password to access DB.

```yaml
# info or debug, anything else - warning
logLevel: info

secrets:
  - psql-query-exporter

config:
  defaults:
    scrape_interval: 30s
    query_timeout: 5s

  sources:
    postgres:
      host: psql-server.postgres.svc.cluster.local
      user: ${PG_USER}
      password: ${PG_PASSWORD}
      sslmode: require
      metric_prefix: postgres_state

      databases:
        - dbname: postgres
          queries:
            - metric_name: replication_lag
              query: |
                select slot_name, slot_type, active::text, 
                (case when not pg_is_in_recovery() then pg_current_wal_lsn() - restart_lsn end)::float as lag_bytes
                from pg_replication_slots;
              values:
                single:
                  field: lag_bytes
                  type: float
              var_labels:
                - slot_name
                - slot_type
                - active

```

### Build your own binary

Since exported is written in Rust, you can use standart Rust tools to build binary for any platform you need. Of cource, you have to have [Rust](https://rust-lang.org) tool-chain installed.

```console
cargo build --release
```

And run it:

```console
target/release/psql-query-exporter --config ./confg.yaml -v
```

## Configuration

TBW
