# Build environment for OpenDeck. Ubuntu 26.04 to match the target host, so the
# produced .deb links against the same glibc and webkit2gtk.
FROM ubuntu:26.04

ENV DEBIAN_FRONTEND=noninteractive
RUN apt-get update && apt-get install -y --no-install-recommends \
		build-essential ca-certificates curl file git pkg-config unzip wget \
		libayatana-appindicator3-dev libdbus-1-dev libgtk-3-dev librsvg2-dev \
		libssl-dev libudev-dev libwebkit2gtk-4.1-dev patchelf \
	&& rm -rf /var/lib/apt/lists/*

# The container runs as the invoking user's uid, so the toolchains have to be
# world-writable: cargo and rustup both write into their own homes.
ENV RUSTUP_HOME=/opt/rustup \
	CARGO_HOME=/opt/cargo \
	DENO_INSTALL=/opt/deno \
	PATH=/opt/cargo/bin:/opt/deno/bin:$PATH

# edition 2024 needs rustc >= 1.85
RUN curl -fsSL https://sh.rustup.rs | sh -s -- -y --default-toolchain stable --profile minimal \
	&& chmod -R a+rwX /opt/rustup /opt/cargo

RUN curl -fsSL https://deno.land/install.sh | sh -s -- -y \
	&& chmod -R a+rwX /opt/deno
