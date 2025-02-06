# Default settings
ARG BUILD_PROFILE=maxperf
ARG RUSTFLAGS="-C target-cpu=native"
ARG FEATURES=""

FROM lukemathwalker/cargo-chef:latest-rust-1 AS chef
WORKDIR /app

# Install system dependencies
RUN apt-get update && apt-get -y upgrade && apt-get install -y libclang-dev pkg-config

# Builds a cargo-chef plan
FROM chef AS planner
COPY . .
RUN cargo chef prepare --recipe-path recipe.json

FROM chef AS builder
COPY --from=planner /app/recipe.json recipe.json

# Build profile, maxperf by default
ARG BUILD_PROFILE
ENV BUILD_PROFILE=$BUILD_PROFILE

# Extra Cargo flags
ARG RUSTFLAGS
ENV RUSTFLAGS="$RUSTFLAGS"

# Extra Cargo features
ARG FEATURES
ENV FEATURES=$FEATURES

# Builds dependencies
RUN cargo chef cook --profile $BUILD_PROFILE --features "$FEATURES" --recipe-path recipe.json

# Build application
COPY . .
RUN cargo build --profile $BUILD_PROFILE --features "$FEATURES" --locked --bin fui-reth

# ARG is not resolved in COPY so we have to hack around it by copying the
# binary to a temporary location
RUN cp /app/target/$BUILD_PROFILE/fui-reth /app/fui-reth

FROM chef AS op-builder
COPY --from=planner /app/recipe.json recipe.json

# Build profile, maxperf by default
ARG BUILD_PROFILE
ENV BUILD_PROFILE=$BUILD_PROFILE

# Extra Cargo flags
ARG RUSTFLAGS
ENV RUSTFLAGS="$RUSTFLAGS"

# Extra Cargo features
ARG FEATURES
ENV FEATURES="optimism,$FEATURES"

# Builds dependencies
RUN cargo chef cook --profile $BUILD_PROFILE --features "$FEATURES" --recipe-path recipe.json

# Build application
COPY . .
RUN cargo build --profile $BUILD_PROFILE --features "$FEATURES" --locked --bin fui-op-reth

# ARG is not resolved in COPY so we have to hack around it by copying the
# binary to a temporary location
RUN cp /app/target/$BUILD_PROFILE/fui-op-reth /app/fui-op-reth

# Use Ubuntu as the release image
FROM ubuntu AS runtime
WORKDIR /app

RUN apt-get update && apt-get -y upgrade && apt-get install -y ca-certificates

# Copy reth over from the build stage
COPY --from=builder /app/fui-reth /usr/local/bin
COPY --from=op-builder /app/fui-op-reth /usr/local/bin

# Copy licenses
COPY LICENSE-* ./

EXPOSE 30303 30303/udp 9001 8545 8546
CMD ["/usr/local/bin/fui-reth"]