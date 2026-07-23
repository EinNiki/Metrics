FROM rust:1.80-slim-bookworm AS builder

# Install build dependencies
RUN apt-get update && apt-get install -y git && rm -rf /var/lib/apt/lists/*

WORKDIR /app

# Copy the workspace configuration and files
COPY Cargo.toml ./
COPY metrics-api ./metrics-api
COPY metrics-core ./metrics-core
COPY modules ./modules

# Compile release binaries and dynamic libraries
RUN cargo build --release

# Copy the built libraries to a folder for final packaging
RUN mkdir -p modules_bin && \
    if [ -f target/release/libmetrics_system.so ]; then cp target/release/libmetrics_system.so modules_bin/; fi && \
    if [ -f target/release/libmetrics_zfs.so ]; then cp target/release/libmetrics_zfs.so modules_bin/; fi && \
    if [ -f target/release/libmetrics_immich.so ]; then cp target/release/libmetrics_immich.so modules_bin/; fi

# --- Runtime Stage ---
FROM rust:1.80-slim-bookworm

# Install runtime dependencies (git is needed for custom modules, zfsutils-linux for ZFS execution, curl for healthcheck)
RUN apt-get update && apt-get install -y git zfsutils-linux curl && rm -rf /var/lib/apt/lists/*

WORKDIR /app

# Copy the workspace structure and metadata so runtime cargo can compile new plugins
COPY Cargo.toml ./
COPY metrics-api ./metrics-api
COPY modules ./modules

# Copy compiled resources from builder
COPY --from=builder /app/target/release/metrics-core ./metrics-core
COPY --from=builder /app/modules_bin ./modules_bin
COPY --from=builder /app/target ./target

# Configure environment
ENV PORT=3000
ENV ADMIN_PASSWORD=admin123
ENV DB_PATH=/data

# Create data directory
RUN mkdir -p /data

EXPOSE 3000

# Healthcheck
HEALTHCHECK --interval=30s --timeout=5s --start-period=10s --retries=3 \
  CMD curl -f http://localhost:3000/ || exit 1

CMD ["./metrics-core"]
