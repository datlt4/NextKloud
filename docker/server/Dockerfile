FROM rust:1.80-slim AS build
WORKDIR /app
COPY Cargo.toml Cargo.lock* ./
COPY crates ./crates
RUN cargo build --release -p nextkloud-backend

FROM debian:bookworm-slim
RUN apt-get update && apt-get install -y ca-certificates && rm -rf /var/lib/apt/lists/*
COPY --from=build /app/target/release/nextkloud /usr/local/bin/nextkloud
EXPOSE 8014
CMD ["nextkloud"]
