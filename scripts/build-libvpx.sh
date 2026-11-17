#!/usr/bin/env bash
# Builds libvpx as a static library for the desktop client's screen share encoder (vpx-rs), into desktop/.libvpx/
# (lib/ and include/). desktop/.cargo/config.toml points the desktop build there, so run this once before the first
# desktop build. Static, so the Linux tarball and AppImage do not depend on a distro's libvpx soname.
#   Linux: from a pinned source tarball. Needs a C compiler, make, and nasm or yasm; without either it first builds a
#     pinned nasm into desktop/.libvpx/nasm (no root needed).
#   Windows (Git Bash): vcpkg's libvpx, triplet x64-windows-static-md (a static library on the dynamic C runtime, as
#     Rust uses). Needs vcpkg (VCPKG_INSTALLATION_ROOT or VCPKG_ROOT; the GitHub runners have it).
# Does nothing when desktop/.libvpx already holds this build. Usage: scripts/build-libvpx.sh
set -euo pipefail
VERSION=1.17.0
SHA256=1020f184046187baa2985dbde38e0691f49c44088bca7a1842b0236c6081dc0a
NASM_VERSION=2.16.03
NASM_SHA256=1412a1c760bbd05db026b6c0d1657affd6631cd0a63cddb6f73cc6d4aa616148
root=$(cd "$(dirname "$0")/.." && pwd)
out="$root/desktop/.libvpx"
want="libvpx $VERSION $(uname -s)"
if [ "$(cat "$out/built" 2>/dev/null)" = "$want" ]; then
  echo "$want already in $out"
  exit 0
fi
rm -rf "$out"
mkdir -p "$out"
tmp=$(mktemp -d)
trap 'rm -rf "$tmp"' EXIT
jobs=$(nproc 2>/dev/null || echo 4)

case "$(uname -s)" in
  MINGW* | MSYS* | CYGWIN*)
    vcpkg_root=${VCPKG_INSTALLATION_ROOT:-${VCPKG_ROOT:?set VCPKG_ROOT to a vcpkg checkout}}
    vcpkg_root=$(cygpath -u "$vcpkg_root")
    port=$(grep -o '"version[a-z-]*": *"[^"]*"' "$vcpkg_root/ports/libvpx/vcpkg.json" | head -1)
    echo "vcpkg libvpx port: $port (the Linux build pins $VERSION)"
    # A release-only triplet: static libvpx, dynamic CRT (/MD), as rustc links.
    mkdir -p "$tmp/triplets"
    cat > "$tmp/triplets/x64-windows-static-md-release.cmake" <<'EOF'
set(VCPKG_TARGET_ARCHITECTURE x64)
set(VCPKG_CRT_LINKAGE dynamic)
set(VCPKG_LIBRARY_LINKAGE static)
set(VCPKG_BUILD_TYPE release)
EOF
    "$vcpkg_root/vcpkg" install libvpx:x64-windows-static-md-release \
      --overlay-triplets="$(cygpath -w "$tmp/triplets")" --x-install-root="$(cygpath -w "$tmp/installed")"
    inst="$tmp/installed/x64-windows-static-md-release"
    mkdir -p "$out/lib"
    cp -r "$inst/include" "$out/include"
    # env-libvpx-sys links a static `libvpx.lib` on Windows; vcpkg names it vpx.lib (older ports vpxmd.lib).
    lib=$(ls "$inst"/lib/*vpx*.lib | head -1)
    cp "$lib" "$out/lib/libvpx.lib"
    ;;
  *)
    if ! command -v nasm > /dev/null && ! command -v yasm > /dev/null; then
      echo "== nasm $NASM_VERSION (no nasm or yasm installed)"
      curl -fsSLo "$tmp/nasm.tar.xz" "https://www.nasm.us/pub/nasm/releasebuilds/$NASM_VERSION/nasm-$NASM_VERSION.tar.xz"
      echo "$NASM_SHA256  $tmp/nasm.tar.xz" | sha256sum -c -
      tar -C "$tmp" -xf "$tmp/nasm.tar.xz"
      # gnu17: nasm 2.16.03 predates C23's bool keyword, which GCC 15 and later default to.
      (cd "$tmp/nasm-$NASM_VERSION" && CFLAGS="-O2 -std=gnu17" ./configure --prefix="$out/nasm" > /dev/null &&
        make -j"$jobs" > /dev/null && make install > /dev/null)
      export PATH="$out/nasm/bin:$PATH"
    fi
    echo "== libvpx $VERSION"
    curl -fsSLo "$tmp/libvpx.tar.gz" "https://github.com/webmproject/libvpx/archive/refs/tags/v$VERSION.tar.gz"
    echo "$SHA256  $tmp/libvpx.tar.gz" | sha256sum -c -
    tar -C "$tmp" -xzf "$tmp/libvpx.tar.gz"
    # VP9 stays in: vpx-rs names both codecs. Runtime CPU detection (the default) picks SIMD on the user's machine.
    (cd "$tmp/libvpx-$VERSION" && ./configure --prefix="$out" --enable-static --disable-shared --enable-pic \
      --disable-examples --disable-tools --disable-docs --disable-unit-tests > /dev/null &&
      make -j"$jobs" > /dev/null && make install > /dev/null)
    rm -rf "$out/lib/pkgconfig"
    ;;
esac
echo "$want" > "$out/built"
echo "libvpx $VERSION built into $out"
