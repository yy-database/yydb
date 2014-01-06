# Runtime image for `yydb` CLI / serve.
# Built in Release from the linux-x64 engine artifact (glibc).
FROM debian:bookworm-slim

RUN apt-get update \
  && apt-get install -y --no-install-recommends ca-certificates \
  && rm -rf /var/lib/apt/lists/* \
  && useradd --system --uid 10001 --create-home --home-dir /data yydb

COPY --chmod=755 yydb /usr/local/bin/yydb

USER yydb
WORKDIR /data
EXPOSE 7777

# Container must bind non-loopback; --insecure-bind is required by design for that.
# Mount a volume on /data and pass your DB path, e.g.:
#   docker run --rm -p 7700:7700 -v yydb-data:/data \
#     ghcr.io/yy-database/yydb:latest \
#     serve /data/app.yydb --bind 0.0.0.0:7700 --insecure-bind
ENTRYPOINT ["yydb"]
CMD ["serve", "/data/app.yydb", "--bind", "0.0.0.0:7777", "--insecure-bind"]
