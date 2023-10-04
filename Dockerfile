FROM alpine
ARG TARGETARCH

COPY /${TARGETARCH}-executables/xlr /usr/bin/

ENTRYPOINT "/usr/bin/xlr"
