FROM docker.io/library/debian:bookworm AS builder

WORKDIR /src
COPY . .
RUN scripts/oci-build.sh

FROM gcr.io/distroless/static-debian11
COPY --from=builder /src/target/release/ddcp /ddcp
COPY --from=builder /src/target/release/crsqlite.so /crsqlite.so
VOLUME /data
CMD ["/ddcp", "--app-dir", "/data"] 
