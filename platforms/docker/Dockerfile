# Drop-in for ghcr.io/bluemap-minecraft/bluemap: same WORKDIR, entrypoint args and env, so upstream's docker run /
# compose lines work with only the image name changed. Context is platforms/natives (tools/build_core.py output):
#   docker buildx build --platform linux/amd64,linux/arm64 -f platforms/docker/Dockerfile platforms/natives
FROM scratch AS core-amd64
COPY linux-x64/bluemap-core /bluemap

FROM scratch AS core-arm64
COPY linux-arm64/bluemap-core /bluemap

FROM core-${TARGETARCH} AS core

# alpine over distroless: upstream's image has a shell, which users rely on for `docker exec` and healthchecks
FROM alpine:3.22
COPY --from=core --chmod=755 /bluemap /usr/local/bin/bluemap
LABEL org.opencontainers.image.source="https://github.com/Hexay/bluemap-rs" \
      org.opencontainers.image.description="Rust drop-in for the BlueMap CLI" \
      org.opencontainers.image.licenses="MIT"
WORKDIR /app
ENV BLUEMAP_COMMAND="docker run --rm -it ghcr.io/hexay/bluemap-rs"
ENTRYPOINT ["bluemap"]
