# Use an official Ubuntu base image
FROM ubuntu:latest

# Install necessary dependencies
RUN apt-get update && apt-get install -y \
    curl \
    build-essential \
    pkg-config \
    libssl-dev \
    && rm -rf /var/lib/apt/lists/*

# Install Rust
RUN curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y
ENV PATH=/root/.cargo/bin:$PATH

# Set the working directory in the container
WORKDIR /app

# Copy your Rust project files to the container
COPY . .

# Build the Rust project
RUN cargo build --release

# Expose the UDP port for communication
EXPOSE 3618

# Run the binary
CMD ["./target/release/p2p_file_sync"]
