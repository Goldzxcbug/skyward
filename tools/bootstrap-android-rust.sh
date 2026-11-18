#!/usr/bin/env bash
# Install the official compiler and matching std without changing global Rust.
set -euo pipefail

repo_root=$(CDPATH= cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd)
tools_dir="$repo_root/.build-tools"
downloads="$tools_dir/downloads"
prefix="$tools_dir/sysroot"
mkdir -p "$downloads" "$prefix" "$tools_dir/cargo"

fetch_install() {
    local component=$1 target=$2 expected=$3
    local package="$component-1.100.0-$target"
    local archive="$downloads/$package.tar.xz"
    local url="https://static.rust-lang.org/dist/$package.tar.xz"
    if ! [[ -f "$archive" ]] || ! printf '%s  %s\n' "$expected" "$archive" | sha256sum --check --status; then
        if command -v aria2c >/dev/null; then
            aria2c --continue=true --allow-overwrite=true --file-allocation=none \
                --max-connection-per-server=16 --split=16 --min-split-size=1M \
                --dir="$downloads" --out="$package.tar.xz" "$url"
        else
            curl --fail --location --retry 3 --output "$archive" "$url"
        fi
    fi
    printf '%s  %s\n' "$expected" "$archive" | sha256sum --check
    tar --extract --file "$archive" --directory "$downloads"
    bash "$downloads/$package/install.sh" --prefix="$prefix" --disable-ldconfig
}

# Official dist SHA-256 files, pinned with Rust 1.100.0 (2026-11-12).
fetch_install rustc x86_64-unknown-linux-gnu f1d0b1dec81dab6f523cd098507763487b0d4bd7fd2c87582f154128719be03b
fetch_install rust-std x86_64-unknown-linux-gnu 4591b6486594ded52ecd6135db95bf9355bbd3ca7a380888b141528bbcf9a5d4
fetch_install rust-std aarch64-linux-android 5e7503cc0cb87b710457d13ec97b2e7729a1210a2fdce9ad99bd0c86e0f69186
fetch_install clippy x86_64-unknown-linux-gnu 0d18df3bff6c852ef8243727fed075ecb33dbf93b8ab8c138c2f45e3f270102a
fetch_install rustfmt x86_64-unknown-linux-gnu 4c9389e4722b16499e5d47d89837ec84a4ef755f105ed2a84c038156064e01ab

cat > "$tools_dir/env.sh" <<'ENV'
# Source this from the repository root: . .build-tools/env.sh
export SKYWARD_BUILD_TOOLS="${SKYWARD_BUILD_TOOLS:-$PWD/.build-tools}"
export ANDROID_NDK_ROOT="${ANDROID_NDK_ROOT:-/opt/android-sdk/ndk/29.0.13113456}"
export CARGO_HOME="$SKYWARD_BUILD_TOOLS/cargo"
export RUSTC="$SKYWARD_BUILD_TOOLS/sysroot/bin/rustc"
export RUSTDOC="$SKYWARD_BUILD_TOOLS/sysroot/bin/rustdoc"
export PATH="$SKYWARD_BUILD_TOOLS/sysroot/bin:$PATH"
export CC_aarch64_linux_android="$ANDROID_NDK_ROOT/toolchains/llvm/prebuilt/linux-x86_64/bin/aarch64-linux-android35-clang"
export AR_aarch64_linux_android="$ANDROID_NDK_ROOT/toolchains/llvm/prebuilt/linux-x86_64/bin/llvm-ar"
export CARGO_TARGET_AARCH64_LINUX_ANDROID_LINKER="$CC_aarch64_linux_android"
export CARGO_TARGET_AARCH64_LINUX_ANDROID_RUSTFLAGS="-C link-arg=-Wl,-z,max-page-size=16384"
ENV

"$prefix/bin/rustc" --version
printf 'Ready. From the repository root, run: . .build-tools/env.sh\n'
