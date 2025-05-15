# Use a Rust base image with Cargo installed
FROM rust:1.86.0-bookworm AS builder

RUN apt-get update \
    && apt-get install -y --no-install-recommends ca-certificates gcc libssl-dev

# Set the working directory inside the container
WORKDIR /usr/src/app

# Copy the Cargo.toml and Cargo.lock files
COPY Cargo.toml Cargo.lock ./
#
## Create an empty src directory to trick Cargo into thinking it's a valid Rust project
#RUN mkdir src && echo "fn main() { println!(\"Hello world!\"); }" > src/main.rs
#
## Build the dependencies without the actual source code to cache dependencies separately
#RUN cargo build --release

# Now copy the source code
COPY ./src ./src


# Build your application
RUN cargo build --release

# Start a new stage to create a smaller image without unnecessary build dependencies
FROM debian:bookworm-slim

RUN apt-get update \
    && apt-get install -y --no-install-recommends ca-certificates libssl3

# Set the working directory
WORKDIR /usr/src/app

# Copy the built binary from the previous stage
COPY --from=builder /usr/src/app/target/release/tiltify-twitchbot ./
COPY ./.env.example ./.env
COPY ./channels.json ./channels.json
COPY ./config.toml ./config.toml

EXPOSE 28257
# Command to run the application
CMD ["./tiltify-twitchbot"]