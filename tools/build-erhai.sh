#!/usr/bin/env bash
# OnePlus Pad 2 Pro port. Build and package locally; never writes a device.
set -euo pipefail
cd -- "$(dirname -- "$0")/.."
if [[ -f .build-tools/env.sh ]]; then
    source .build-tools/env.sh
fi
: "${ANDROID_NDK_ROOT:?Set ANDROID_NDK_ROOT or run tools/bootstrap-android-rust.sh}"
# The displayed label can contain an underscore. Update checks compare SemVer.
export SKYWARD_DISPLAY_VERSION="${SKYWARD_DISPLAY_VERSION:-${SKYWARD_BUILD_VERSION:-0.3.1-Gold_bug}}"
export SKYWARD_BUILD_VERSION="${SKYWARD_BUILD_VERSION:-0.3.1-erhai.3}"
ndk_bin="$ANDROID_NDK_ROOT/toolchains/llvm/prebuilt/linux-x86_64/bin"
export CC_aarch64_linux_android="${CC_aarch64_linux_android:-$ndk_bin/aarch64-linux-android35-clang}"
export AR_aarch64_linux_android="${AR_aarch64_linux_android:-$ndk_bin/llvm-ar}"
export CARGO_TARGET_AARCH64_LINUX_ANDROID_LINKER="$CC_aarch64_linux_android"
export CARGO_TARGET_AARCH64_LINUX_ANDROID_RUSTFLAGS="-C link-arg=-Wl,-z,max-page-size=16384"
stock="${SKYWARD_STOCK_IMAGE:-recovery.img}"
stock_sum=076c237ff4cf4ff53914b88c353042cf0f2c5ccc74c836ba11ee42b77055faae
actual_sum=$(sha256sum "$stock")
[[ "${actual_sum%% *}" == "$stock_sum" ]] || { echo 'Stock recovery SHA256 differs from the reviewed input.' >&2; exit 1; }
out=out/erhai
mkdir -p "$out"
cargo build --locked --release -p skyward-pack
cargo build --locked --release --target aarch64-linux-android -p skyward
RUSTFLAGS='-C target-feature=+crt-static -C link-arg=-Wl,-z,max-page-size=16384' \
    cargo build --locked --release --target aarch64-linux-android \
    --target-dir target/system-init -p skyward-system-init
xz -9 -c resources/fonts/MiSansVF.ttf > "$out/MiSansVF.ttf.xz"
image="$out/skyward-v$SKYWARD_DISPLAY_VERSION-OP615EL1.img"
target/release/skyward-pack recovery --stock "$stock" \
    --binary target/aarch64-linux-android/release/skyward \
    --system-init target/system-init/aarch64-linux-android/release/system-init \
    --config devices/erhai.toml --font "$out/MiSansVF.ttf.xz" \
    --mono resources/fonts/KirkMono-Regular.ttf \
    --mono-license resources/fonts/KirkMono-OFL.txt --omit-oplus-ui --out "$image"
python3 tools/verify-erhai-image.py --stock "$stock" --image "$image" \
    --binary target/aarch64-linux-android/release/skyward \
    --system-init target/system-init/aarch64-linux-android/release/system-init \
    --config devices/erhai.toml --font "$out/MiSansVF.ttf.xz" \
    --mono resources/fonts/KirkMono-Regular.ttf \
    --mono-license resources/fonts/KirkMono-OFL.txt
sha256sum "$image" > "$out/SHA256SUMS"
printf 'Built %s\n' "$image"
