# Use an official Ubuntu base image
FROM ubuntu:latest

# Install necessary dependencies
RUN apt-get update && apt-get install -y \
    curl \
    build-essential \
    pkg-config \
    libssl-dev \
    && rm -rf /var/lib/apt/lists/*

USER ubuntu

# Install Rust
RUN curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y
ENV PATH=/home/ubuntu/.cargo/bin:$PATH

WORKDIR /home/ubuntu

# Copy your Rust project files to the container
COPY . .
USER root
RUN chown -R ubuntu:ubuntu /home/ubuntu
USER ubuntu
# Build the Rust project
RUN cargo build --release

# Expose the UDP port for communication
EXPOSE 3618
