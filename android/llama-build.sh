#!/bin/bash
# Build llama-server for Android aarch64
# Requires: Android NDK, CMake
#
# Usage: ./llama-build.sh [path-to-llama.cpp]
set -e

LLAMA_DIR="${1:-../../llama.cpp}"
BUILD_DIR="$LLAMA_DIR/build-android"

if [ -z "$ANDROID_NDK" ] && [ -z "$ANDROID_NDK_HOME" ]; then
    # Try common locations
    if [ -d "$HOME/Library/Android/sdk/ndk" ]; then
        ANDROID_NDK=$(ls -d "$HOME/Library/Android/sdk/ndk"/*/ 2>/dev/null | tail -1)
    fi
fi
NDK="${ANDROID_NDK:-$ANDROID_NDK_HOME}"

if [ -z "$NDK" ] || [ ! -d "$NDK" ]; then
    echo "ERROR: Android NDK not found. Set ANDROID_NDK or ANDROID_NDK_HOME."
    exit 1
fi

echo "Using NDK: $NDK"
echo "Building llama-server for Android aarch64..."

cmake \
    -DCMAKE_TOOLCHAIN_FILE="$NDK/build/cmake/android.toolchain.cmake" \
    -DANDROID_ABI=arm64-v8a \
    -DANDROID_PLATFORM=android-28 \
    -DCMAKE_C_FLAGS="-march=armv8.7a" \
    -DCMAKE_CXX_FLAGS="-march=armv8.7a" \
    -DGGML_OPENMP=OFF \
    -DGGML_LLAMAFILE=OFF \
    -DGGML_CURL=OFF \
    -S "$LLAMA_DIR" \
    -B "$BUILD_DIR"

cmake --build "$BUILD_DIR" --config Release -j$(nproc 2>/dev/null || sysctl -n hw.ncpu)

# Copy to assets
DEST="app/src/main/assets/bin"
mkdir -p "$DEST"
cp "$BUILD_DIR/bin/llama-server" "$DEST/"
echo "Done: $DEST/llama-server"
ls -lh "$DEST/llama-server"
