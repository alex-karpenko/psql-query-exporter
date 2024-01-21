ARG RUST_VERSION=1.75-bookworm
FROM rust:${RUST_VERSION} AS builder

WORKDIR /src
COPY . .

RUN cargo build --release
RUN strip target/release/psql-query-exporter

# Runtime stage
FROM debian:12-slim

RUN apt update && apt install -y ca-certificates && apt clean

USER nobody
WORKDIR /app
COPY --from=builder /src/target/release/psql-query-exporter /app/

ENTRYPOINT ["/app/psql-query-exporter"]
CMD ["--help"]
