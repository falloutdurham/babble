# Build the binary against the same glibc the runtime image uses.
FROM rust:1-slim-bookworm AS build

# rusqlite's `bundled` feature compiles SQLite from source, so the builder
# needs a C toolchain. The runtime image does not.
RUN apt-get update \
 && apt-get install -y --no-install-recommends gcc libc6-dev \
 && rm -rf /var/lib/apt/lists/*

WORKDIR /src

# Dependencies first: this layer is cached until Cargo.toml or Cargo.lock moves.
COPY Cargo.toml Cargo.lock ./
RUN mkdir -p src \
 && echo 'fn main() {}' > src/main.rs \
 && echo '' > src/lib.rs \
 && cargo build --release --locked \
 && rm -rf src

COPY src ./src
# Cargo skips a rebuild when only mtimes changed, so nudge the real sources.
RUN touch src/main.rs src/lib.rs \
 && cargo build --release --locked \
 && strip target/release/board

# Stage the data directory here so it lands in the runtime image already owned
# by the unprivileged user — distroless has no shell to chown it afterwards.
RUN mkdir -p /out/data && chown 65532:65532 /out/data

# distroless/cc carries glibc and libgcc (what the binary links against) and
# ca-certificates for using this same image as a client against an HTTPS board.
# It has no shell and no package manager.
FROM gcr.io/distroless/cc-debian12:nonroot AS runtime

COPY --from=build /src/target/release/board /usr/local/bin/board
COPY --from=build --chown=nonroot:nonroot /out/data /data

WORKDIR /data
# The database lives here; mount a volume to keep it across container restarts.
VOLUME ["/data"]
EXPOSE 7420

# Must bind 0.0.0.0: 127.0.0.1 would only be reachable inside the container.
ENTRYPOINT ["board"]
CMD ["serve", "--db", "/data/board.sqlite", "--bind", "0.0.0.0:7420"]
