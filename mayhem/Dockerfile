FROM --platform=linux/amd64 ubuntu:22.04 AS builder

RUN apt-get update && DEBIAN_FRONTEND=noninteractive apt-get install -y \
    curl build-essential

RUN curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y --default-toolchain nightly
ENV PATH="/root/.cargo/bin:${PATH}"
RUN cargo install cargo-fuzz

ADD . /src
WORKDIR /src

# Remove toolchain pin (1.63.0) so nightly is used for fuzz builds
RUN rm -f rust-toolchain.toml rust-toolchain

# Fix new nightly lint mismatched_lifetime_syntaxes breaking all deny(warnings) crates.
RUN grep -rl '#!\[deny(warnings)\]' /src/lib /src/src 2>/dev/null | \
    xargs sed -i 's/#!\[deny(warnings)\]/#![deny(warnings)]\n#![allow(mismatched_lifetime_syntaxes)]/'

RUN echo building instrumented harnesses && \
    bash -c "pushd lib/vrl/parser/fuzz && cargo fuzz build && popd" && \
    mv lib/vrl/parser/fuzz/target/x86_64-unknown-linux-gnu/release/parse /parse && \
    echo done

RUN echo building non-instrumented harnesses && \
    export RUSTFLAGS="--cfg fuzzing -Clink-dead-code -Cdebug-assertions -C codegen-units=1" && \
    bash -c "pushd lib/vrl/parser/fuzz && cargo +nightly build --release && popd" && \
    mv lib/vrl/parser/fuzz/target/release/parse /parse_no_inst && \
    echo done

FROM ubuntu:22.04

COPY --from=builder /parse /parse_no_inst /
