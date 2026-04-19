#!/bin/bash
# End-to-end APK build script
# Prerequisites: cargo-ndk, Android NDK, Android SDK, Gradle
set -e

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
PROJECT_ROOT="$(dirname "$SCRIPT_DIR")"

echo "=== Ern-OS Android APK Build ==="
echo "Project root: $PROJECT_ROOT"

# 1. Install cargo-ndk if needed
if ! command -v cargo-ndk &>/dev/null; then
    echo "Installing cargo-ndk..."
    cargo install cargo-ndk
fi

# 2. Add Android target
echo "Ensuring aarch64-linux-android target..."
rustup target add aarch64-linux-android

# 3. Cross-compile Rust engine → libernos.so
echo "Building Rust engine for Android..."
cd "$PROJECT_ROOT"
cargo ndk -t arm64-v8a build --release --lib --no-default-features --features "android,file-extract"

# 4. Copy .so to jniLibs
echo "Copying libernos.so..."
JNILIB_DIR="$SCRIPT_DIR/app/src/main/jniLibs/arm64-v8a"
mkdir -p "$JNILIB_DIR"
cp "$PROJECT_ROOT/target/aarch64-linux-android/release/libernos.so" "$JNILIB_DIR/"
ls -lh "$JNILIB_DIR/libernos.so"

# 5. Build llama-server (optional — skip if not available)
if [ -d "$PROJECT_ROOT/../llama.cpp" ]; then
    echo "Building llama-server for Android..."
    cd "$SCRIPT_DIR"
    bash llama-build.sh "$PROJECT_ROOT/../llama.cpp"
else
    echo "Skipping llama-server build (llama.cpp not found at $PROJECT_ROOT/../llama.cpp)"
fi

# 6. Build APK via Gradle
echo "Building APK..."
cd "$SCRIPT_DIR"
./gradlew assembleRelease

# 7. Report
APK_PATH=$(find "$SCRIPT_DIR/app/build/outputs/apk/release" -name "*.apk" 2>/dev/null | head -1)
if [ -n "$APK_PATH" ]; then
    echo ""
    echo "=== BUILD COMPLETE ==="
    echo "APK: $APK_PATH"
    ls -lh "$APK_PATH"
else
    echo "WARNING: APK not found in expected location"
fi
