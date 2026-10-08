# genegis-mcp: GeneGIS verified spatial analysis over MCP (stdio).
#
# Build from the repository root:
#   docker build -f deploy/mcp/Dockerfile -t genegis-mcp .
# Run as an MCP stdio server:
#   docker run -i --rm -v genegis-layers:/data genegis-mcp

FROM rust:1.97-slim-trixie AS build
WORKDIR /src
COPY . .
RUN cargo build --locked --release -p genegis-toolkit --bin genegis-mcp

FROM debian:trixie-slim
LABEL io.modelcontextprotocol.server.name="io.github.rsasaki0109/genegis"
LABEL org.opencontainers.image.source="https://github.com/rsasaki0109/GeneGIS"
LABEL org.opencontainers.image.description="GeneGIS MCP server: the model plans, GeneGIS validates, executes and verifies every step"
LABEL org.opencontainers.image.licenses="Apache-2.0 OR MIT"
RUN apt-get update \
    && apt-get install -y --no-install-recommends ca-certificates \
    && rm -rf /var/lib/apt/lists/*
COPY --from=build /src/target/release/genegis-mcp /usr/local/bin/genegis-mcp
# Bundled Nagoya sample layers for `load_sample_data` (synthetic fixtures and
# licensed open data; see examples/nagoya-population-density/data/README.md).
COPY examples/nagoya-population-density/data /opt/genegis/samples
ENV GENEGIS_SAMPLES_DIR=/opt/genegis/samples \
    GENEGIS_LAYER_DIR=/data/layers
WORKDIR /data
ENTRYPOINT ["genegis-mcp"]
