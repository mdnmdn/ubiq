# Harness Multiplexer - Development Commands

# Default recipe
default:
    @just --list

# Start development server with hot reload
dev:
    cargo tauri dev

# Build the application for production
build:
    cargo tauri build

# Build the frontend only
build-frontend:
    npm run build

# Check Rust code without building
check:
    cd src-tauri && cargo check

# Check Rust code with all warnings
check-all:
    cd src-tauri && cargo check --all-targets

# Run clippy for Rust linting
clippy:
    cd src-tauri && cargo clippy -- -D warnings

# Format Rust code
format-rust:
    cd src-tauri && cargo fmt

# Format JavaScript/TypeScript code
format-js:
    npm run format

# Install npm dependencies
install:
    npm install

# Clean build artifacts
clean:
    cd src-tauri && cargo clean
    rm -rf dist
    rm -rf node_modules

# Run tests (when available)
test:
    cd src-tauri && cargo test
    npm test

# Update dependencies
update:
    cd src-tauri && cargo update
    npm update

# Run the application in release mode
run-release:
    cargo tauri dev --release

# Generate Tauri icons from a source image (requires ImageMagick)
generate-icons source="icon.png":
    # Generate 32x32
    magick {{source}} -resize 32x32 src-tauri/icons/32x32.png
    # Generate 128x128
    magick {{source}} -resize 128x128 src-tauri/icons/128x128.png
    # Generate 128x128@2x
    magick {{source}} -resize 256x256 src-tauri/icons/128x128@2x.png
    # Copy as icon.icns and icon.ico (placeholders)
    cp src-tauri/icons/128x128.png src-tauri/icons/icon.icns
    cp src-tauri/icons/128x128.png src-tauri/icons/icon.ico

# Display project information
info:
    cargo tauri info

# Check for security vulnerabilities
audit:
    cargo audit
    npm audit

# Run the application with verbose logging
verbose:
    RUST_LOG=debug cargo tauri dev

# Build for specific platform (e.g., just build-platform macos)
platform target:
    cargo tauri build --target {{target}}