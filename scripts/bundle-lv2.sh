#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
OUT_DIR="${1:-${ROOT_DIR}/target/lv2/prophet-xii.lv2}"
PROFILE="${PROPHET_CARGO_PROFILE:-release}"

if [[ "${PROFILE}" == "debug" ]]; then
    TARGET_SUBDIR="debug"
    CARGO_FLAG=""
else
    TARGET_SUBDIR="release"
    CARGO_FLAG="--release"
fi

cargo build --manifest-path "${ROOT_DIR}/Cargo.toml" ${CARGO_FLAG} --locked --offline -p prophet-lv2

BIN_SO="${ROOT_DIR}/target/${TARGET_SUBDIR}/libprophet_lv2.so"
if [[ ! -f "${BIN_SO}" ]]; then
    echo "ERROR: Expected compiled LV2 plugin at ${BIN_SO}" >&2
    exit 1
fi

rm -rf "${OUT_DIR}"
mkdir -p "${OUT_DIR}"
cp "${BIN_SO}" "${OUT_DIR}/libprophet_lv2.so"

"${ROOT_DIR}/target/${TARGET_SUBDIR}/gen_ttl" "${OUT_DIR}" "libprophet_lv2.so"
echo "Bundled LV2 plugin at ${OUT_DIR}"
