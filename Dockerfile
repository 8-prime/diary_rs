# ---- build ----
FROM rust:1.95-bookworm AS builder

WORKDIR /app

# libsqlite3-sys and libwebp-sys compile vendored C sources
RUN apt-get update && apt-get install -y --no-install-recommends \
      build-essential \
    && rm -rf /var/lib/apt/lists/*

# use the committed .sqlx cache; never reach for a live database
ENV SQLX_OFFLINE=true

COPY . .
RUN cargo build --release --locked -p server

# ---- runtime ----
FROM gcr.io/distroless/cc-debian12 AS runtime

COPY --from=builder /app/target/release/server /usr/local/bin/server

# sqlite file + image blobs
VOLUME ["/data"]
EXPOSE 3000

USER nonroot:nonroot
CMD ["/usr/local/bin/server"]
