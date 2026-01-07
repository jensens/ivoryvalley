# Build stage - install from crates.io
FROM rust:1-slim-bookworm AS builder

# Install ivoryvalley from crates.io
# The version is passed as a build argument
ARG VERSION
RUN cargo install ivoryvalley --version ${VERSION}

# Runtime stage - minimal image
FROM debian:bookworm-slim

# Install ca-certificates for HTTPS connections and create non-root user
RUN apt-get update && apt-get install -y --no-install-recommends \
    ca-certificates \
    && rm -rf /var/lib/apt/lists/* \
    && useradd -r -u 1000 -m ivoryvalley

# Copy the binary from builder
COPY --from=builder /usr/local/cargo/bin/ivoryvalley /usr/local/bin/ivoryvalley

# Create data directory for SQLite database
RUN mkdir -p /data && chown ivoryvalley:ivoryvalley /data

# Switch to non-root user
USER ivoryvalley
WORKDIR /data

# Default environment variables
ENV IVORYVALLEY_HOST=0.0.0.0 \
    IVORYVALLEY_PORT=8080 \
    IVORYVALLEY_DATABASE_PATH=/data/ivoryvalley.db

# Expose the proxy port
EXPOSE 8080

# Run the proxy
ENTRYPOINT ["ivoryvalley"]
