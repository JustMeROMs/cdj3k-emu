#!/usr/bin/env bash
set -euo pipefail

# Build the patched CDJ3K QEMU backend for 64-bit Windows under MSYS2/MINGW64.
# This produces a self-contained runtime directory consumed by the Windows app.

ROOT_DIR="$(cd "$(dirname "$0")/.." && pwd)"
QEMU_DIR="${ROOT_DIR}/qemu"
SRC_DIR="${QEMU_DIR}/src-win"
BUILD_DIR="${QEMU_DIR}/build-win"
OUT_DIR="${QEMU_DIR}/windows-out"
PATCHES_DIR="${QEMU_DIR}/patches"
QEMU_REF="ee7eb612be8f8886d48c1d0c1f1c65e495138f83"
QEMU_REPO="https://github.com/qemu/qemu.git"

# Do not globally disable MSYS2 path conversion. QEMU's configure invokes the
# native MinGW Python with POSIX paths (for example /d/a/...). MSYS2 must
# translate those arguments to D:/a/...; disabling conversion produces bogus
# paths such as D:/d:/a/... and breaks python/scripts/mkvenv.py.
unset MSYS2_ARG_CONV_EXCL || true

rm -rf "${OUT_DIR}"
mkdir -p "${OUT_DIR}"

if [ ! -d "${SRC_DIR}/.git" ]; then
  echo "==> Fetching QEMU @ ${QEMU_REF}"
  rm -rf "${SRC_DIR}" "${BUILD_DIR}"
  git init -q "${SRC_DIR}"
  git -C "${SRC_DIR}" remote add origin "${QEMU_REPO}"
  git -C "${SRC_DIR}" fetch --depth=1 origin "${QEMU_REF}"
  git -C "${SRC_DIR}" checkout -q FETCH_HEAD
else
  current_sha="$(git -C "${SRC_DIR}" rev-parse HEAD 2>/dev/null || true)"
  if [ "${current_sha}" != "${QEMU_REF}" ]; then
    echo "==> QEMU source changed; refreshing pinned checkout"
    rm -rf "${SRC_DIR}" "${BUILD_DIR}"
    git init -q "${SRC_DIR}"
    git -C "${SRC_DIR}" remote add origin "${QEMU_REPO}"
    git -C "${SRC_DIR}" fetch --depth=1 origin "${QEMU_REF}"
    git -C "${SRC_DIR}" checkout -q FETCH_HEAD
  fi
fi

# Return the source to the exact pinned commit before applying our patch stack.
git -C "${SRC_DIR}" reset --hard -q "${QEMU_REF}"
git -C "${SRC_DIR}" clean -fdx -q

# The upstream patch stack contains several macOS-only performance/audio
# patches. They modify CoreAudio, Mach thread scheduling and HVF. Those are
# intentionally excluded from the Windows build; Windows uses the normal
# QEMU audio path (DirectSound for now) and TCG on x64.
for p in "${PATCHES_DIR}"/*.patch; do
  base="$(basename "$p")"
  case "$base" in
    07-coreaudio-bypass.patch|08-virtio-snd-bypass.patch|09-system-main-qos.patch|12-hvf-vcpu-qos.patch)
      echo "==> Skipping macOS-only patch ${base}"
      continue
      ;;
  esac
  echo "==> Applying ${base}"
  git -C "${SRC_DIR}" apply --check "$p"
  git -C "${SRC_DIR}" apply "$p"
done

# Alpha 5.2 deliberately disables libslirp. The GitHub Windows/MSYS2 runner
# does not reliably expose mingw-w64-x86_64-libslirp. Networking will be
# restored after the patched QEMU core builds and self-tests successfully.
SLIRP_OPT="--disable-slirp"
echo "==> Alpha 5.2: building QEMU without libslirp networking"

mkdir -p "${BUILD_DIR}"
if [ ! -f "${BUILD_DIR}/build.ninja" ]; then
  echo "==> Alpha 5.4: MSYS2 path conversion enabled for QEMU configure"
  echo "==> Configuring Windows QEMU (aarch64-softmmu, TCG; ${SLIRP_OPT})"
  (
    cd "${BUILD_DIR}"
    "${SRC_DIR}/configure" \
      --target-list=aarch64-softmmu \
      --enable-tcg \
      --disable-whpx \
      ${SLIRP_OPT} \
      --disable-gtk \
      --disable-sdl \
      --disable-curses \
      --disable-vnc \
      --disable-docs \
      --disable-guest-agent \
      --disable-plugins \
      --disable-werror \
      --enable-tools
  )
fi

JOBS="${NUMBER_OF_PROCESSORS:-4}"
echo "==> Building patched QEMU with ${JOBS} jobs"
ninja -C "${BUILD_DIR}" -j"${JOBS}" qemu-system-aarch64 qemu-img

cp "${BUILD_DIR}/qemu-system-aarch64.exe" "${OUT_DIR}/"
cp "${BUILD_DIR}/qemu-img.exe" "${OUT_DIR}/"

# Copy all MinGW runtime DLLs referenced by the two executables. ldd output on
# MSYS2 contains POSIX-style /mingw64/bin paths; they remain valid to cp here.
copy_deps() {
  local exe="$1"
  while IFS= read -r dll; do
    [ -n "$dll" ] || continue
    [ -f "$dll" ] || continue
    cp -n "$dll" "${OUT_DIR}/" || true
  done < <(ldd "$exe" 2>/dev/null | awk '
    /=> \/mingw64\// { print $3 }
    /^[[:space:]]*\/mingw64\// { print $1 }
  ' | sort -u)
}
copy_deps "${OUT_DIR}/qemu-system-aarch64.exe"
copy_deps "${OUT_DIR}/qemu-img.exe"

# Smoke tests prove the packaged binaries can start with the copied DLL set.
echo "==> QEMU smoke test"
"${OUT_DIR}/qemu-system-aarch64.exe" --version
"${OUT_DIR}/qemu-img.exe" --version

cat > "${OUT_DIR}/README-QEMU-WINDOWS.txt" <<'TXT'
This directory contains the patched QEMU backend used by cdj3k-emu on Windows.
It is built from the pinned upstream QEMU commit plus the qemu/patches stack.
Windows x64 runs the ARM64 CDJ guest with TCG emulation.
TXT

echo "==> Windows QEMU runtime ready at ${OUT_DIR}"
find "${OUT_DIR}" -maxdepth 1 -type f -printf '%f\n' 2>/dev/null || ls -la "${OUT_DIR}"
