# Build context: repository root (web client + Rust backend).
FROM node:22-slim AS web
WORKDIR /app
COPY apps/desktop/package.json apps/desktop/package-lock.json ./
RUN npm ci --ignore-scripts
COPY apps/desktop/index.html apps/desktop/vite.config.ts apps/desktop/tsconfig.json ./
COPY apps/desktop/src ./src
COPY apps/desktop/public ./public
RUN npm run build

FROM rust:1-bookworm AS build
# Multi-arch builds run in parallel: caches are per platform (a shared target dir would mix
# architectures) and the registry is locked (concurrent unpacking fails with "File exists").
ARG TARGETPLATFORM
WORKDIR /src
COPY Cargo.toml Cargo.lock ./
COPY crates ./crates
RUN --mount=type=cache,id=cargo-registry-$TARGETPLATFORM,target=/usr/local/cargo/registry,sharing=locked \
    --mount=type=cache,id=cargo-target-$TARGETPLATFORM,target=/src/target,sharing=locked \
    cargo build --release --locked -p nextkloud-backend && cp target/release/nextkloud /nextkloud

FROM debian:bookworm-slim
RUN apt-get update \
 && apt-get install -y --no-install-recommends ca-certificates \
 && rm -rf /var/lib/apt/lists/*
COPY --from=build /nextkloud /usr/local/bin/nextkloud
COPY --from=web /app/dist /web
ENV NEXTKLOUD_ADDR=0.0.0.0:8014 NEXTKLOUD_WEB=/web
VOLUME /data
EXPOSE 8014
CMD ["nextkloud"]
