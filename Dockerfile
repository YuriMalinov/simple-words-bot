FROM rust:1.75 as builder

RUN apt update && apt install -y protobuf-compiler
WORKDIR /usr/src/simple-words-builder
COPY . .
RUN cargo build --release

FROM debian:stable-slim

RUN apt update && apt install -y ca-certificates libssl-dev
WORKDIR /usr/src/words-bot
COPY data data
COPY --from=builder /usr/src/simple-words-builder/target/release/simple-words-bot .

CMD ["./simple-words-bot"]

