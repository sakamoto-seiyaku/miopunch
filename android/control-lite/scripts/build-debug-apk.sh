#!/usr/bin/env bash
set -euo pipefail

script_dir="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)"
app_dir="$(cd -- "${script_dir}/.." && pwd)"
repo_root="$(cd -- "${app_dir}/../.." && pwd)"

sdk_root="${ANDROID_SDK_ROOT:-${ANDROID_HOME:-/mnt/c/Android/SDK}}"
platform="${ANDROID_PLATFORM:-android-36}"
build_tools_version="${ANDROID_BUILD_TOOLS_VERSION:-}"
go_bin="${GO_BIN:-}"

die() {
  echo "error: $*" >&2
  exit 1
}

find_go() {
  if [[ -n "${go_bin}" ]]; then
    command -v "${go_bin}" >/dev/null 2>&1 || [[ -x "${go_bin}" ]] || die "GO_BIN is not executable: ${go_bin}"
    printf '%s\n' "${go_bin}"
    return 0
  fi
  if command -v go >/dev/null 2>&1; then
    command -v go
    return 0
  fi
  if [[ -x /usr/local/go/bin/go ]]; then
    printf '%s\n' /usr/local/go/bin/go
    return 0
  fi
  die "missing Go toolchain"
}

latest_dir() {
  local root="$1"
  [[ -d "${root}" ]] || return 1
  find "${root}" -mindepth 1 -maxdepth 1 -type d | sort -V | tail -1
}

tool_path() {
  local dir="$1"
  local name="$2"
  for candidate in "${dir}/${name}" "${dir}/${name}.exe" "${dir}/${name}.bat"; do
    [[ -f "${candidate}" ]] && {
      printf '%s\n' "${candidate}"
      return 0
    }
  done
  return 1
}

to_win_path() {
  if command -v wslpath >/dev/null 2>&1; then
    wslpath -w "$1"
    return 0
  fi
  if [[ "$1" =~ ^/mnt/([a-zA-Z])/(.*)$ ]]; then
    local drive="${BASH_REMATCH[1]}"
    local rest="${BASH_REMATCH[2]//\//\\}"
    printf '%s:\\%s\n' "${drive^^}" "${rest}"
    return 0
  fi
  printf '%s\n' "$1"
}

run_windows_tool() {
  local tool="$1"
  shift
  local cmd="/mnt/c/Windows/System32/cmd.exe"
  [[ -x /init && -x "${cmd}" ]] || die "cannot run Windows Android SDK tool from this WSL environment: ${tool}"

  local converted=()
  local arg
  for arg in "$@"; do
    if [[ "${arg}" == /* ]]; then
      converted+=("$(to_win_path "${arg}")")
    else
      converted+=("${arg}")
    fi
  done
  (
    cd /mnt/c/Windows/System32
    /init "${cmd}" /C "$(to_win_path "${tool}")" "${converted[@]}"
  )
}

run_native_or_windows_tool() {
  local tool="$1"
  shift
  case "${tool}" in
    *.exe|*.bat) run_windows_tool "${tool}" "$@" ;;
    *) "${tool}" "$@" ;;
  esac
}

is_windows_tool() {
  case "$1" in
    *.exe|*.bat) return 0 ;;
    *) return 1 ;;
  esac
}

require_file() {
  [[ -f "$1" ]] || die "missing file: $1"
}

require_cmd() {
  command -v "$1" >/dev/null 2>&1 || die "missing dependency: $1"
}

[[ -d "${sdk_root}" ]] || die "Android SDK not found: ${sdk_root}"
if [[ -z "${build_tools_version}" ]]; then
  build_tools_dir="$(latest_dir "${sdk_root}/build-tools")" || die "missing Android build-tools under ${sdk_root}"
else
  build_tools_dir="${sdk_root}/build-tools/${build_tools_version}"
fi
[[ -d "${build_tools_dir}" ]] || die "Android build-tools not found: ${build_tools_dir}"

android_jar="${sdk_root}/platforms/${platform}/android.jar"
require_file "${android_jar}"

aapt2="$(tool_path "${build_tools_dir}" aapt2)" || die "missing aapt2 in ${build_tools_dir}"
zipalign="$(tool_path "${build_tools_dir}" zipalign)" || die "missing zipalign in ${build_tools_dir}"
d8_jar="${build_tools_dir}/lib/d8.jar"
apksigner_jar="${build_tools_dir}/lib/apksigner.jar"
require_file "${d8_jar}"
require_file "${apksigner_jar}"
require_cmd javac
require_cmd keytool
require_cmd zip
require_cmd java

build_dir="${app_dir}/build"
classes_dir="${build_dir}/classes"
dex_dir="${build_dir}/dex"
gen_dir="${build_dir}/generated"
jni_root="${build_dir}/jni"
jni_dir="${jni_root}/lib/arm64-v8a"
intermediates="${build_dir}/intermediates"
outputs="${build_dir}/outputs"
manifest="${app_dir}/src/main/AndroidManifest.xml"
keystore="${build_dir}/debug.keystore"
win_work_root="${MIOPUNCH_ANDROID_WIN_WORKDIR:-/mnt/c/Users/Public/miopunch-control-lite-build}"

rm -rf "${classes_dir}" "${dex_dir}" "${gen_dir}" "${jni_root}" "${intermediates}" "${outputs}"
mkdir -p "${classes_dir}" "${dex_dir}" "${gen_dir}" "${jni_dir}" "${intermediates}" "${outputs}"

payload="${jni_dir}/libmiopunch.so"
echo "building Android payload: ${payload}"
(
  cd "${repo_root}"
  CGO_ENABLED=0 GOOS=android GOARCH=arm64 "$(find_go)" build \
    -trimpath -ldflags="-s -w" \
    -o "${payload}" ./cmd/miopunch
)
chmod 0755 "${payload}"

echo "compiling Java sources"
mapfile -t java_sources < <(find "${app_dir}/src/main/java" -name '*.java' | sort)
[[ "${#java_sources[@]}" -gt 0 ]] || die "no Java sources found"
javac -Xlint:-options -source 8 -target 8 -classpath "${android_jar}" -d "${classes_dir}" "${java_sources[@]}"

echo "dexing"
mapfile -t class_files < <(find "${classes_dir}" -name '*.class' | sort)
[[ "${#class_files[@]}" -gt 0 ]] || die "no compiled class files found"
java -cp "${d8_jar}" com.android.tools.r8.D8 \
  --min-api 31 \
  --lib "${android_jar}" \
  --output "${dex_dir}" \
  "${class_files[@]}"

base_apk="${intermediates}/base.apk"
unsigned_apk="${intermediates}/unsigned.apk"
aligned_apk="${intermediates}/aligned.apk"
debug_apk="${outputs}/miopunch-control-lite-debug.apk"

echo "linking APK manifest"
if is_windows_tool "${aapt2}"; then
  win_aapt_dir="${win_work_root}/aapt2"
  rm -rf "${win_aapt_dir}"
  mkdir -p "${win_aapt_dir}/generated"
  cp "${manifest}" "${win_aapt_dir}/AndroidManifest.xml"
  run_native_or_windows_tool "${aapt2}" link \
    -o "${win_aapt_dir}/base.apk" \
    -I "${android_jar}" \
    --manifest "${win_aapt_dir}/AndroidManifest.xml" \
    --java "${win_aapt_dir}/generated" \
    --min-sdk-version 31 \
    --target-sdk-version 36 \
    --version-code 1 \
    --version-name 0.1-debug \
    --debug-mode
  cp "${win_aapt_dir}/base.apk" "${base_apk}"
else
  run_native_or_windows_tool "${aapt2}" link \
    -o "${base_apk}" \
    -I "${android_jar}" \
    --manifest "${manifest}" \
    --java "${gen_dir}" \
    --min-sdk-version 31 \
    --target-sdk-version 36 \
    --version-code 1 \
    --version-name 0.1-debug \
    --debug-mode
fi

cp "${base_apk}" "${unsigned_apk}"
(
  cd "${dex_dir}"
  zip -q -r "${unsigned_apk}" classes.dex
)
(
  cd "${jni_root}"
  zip -q -r "${unsigned_apk}" lib
)
if [[ -d "${app_dir}/src/main/assets" ]]; then
  echo "packaging assets"
  (
    cd "${app_dir}/src/main"
    zip -q -r "${unsigned_apk}" assets
  )
fi

if [[ ! -f "${keystore}" ]]; then
  echo "creating debug keystore"
  keytool -genkeypair \
    -keystore "${keystore}" \
    -storepass android \
    -keypass android \
    -alias androiddebugkey \
    -keyalg RSA \
    -keysize 2048 \
    -validity 10000 \
    -dname "CN=Android Debug,O=miopunch,C=US" >/dev/null
fi

echo "zipalign"
if is_windows_tool "${zipalign}"; then
  win_zipalign_dir="${win_work_root}/zipalign"
  rm -rf "${win_zipalign_dir}"
  mkdir -p "${win_zipalign_dir}"
  cp "${unsigned_apk}" "${win_zipalign_dir}/unsigned.apk"
  run_native_or_windows_tool "${zipalign}" -f -p 4 "${win_zipalign_dir}/unsigned.apk" "${win_zipalign_dir}/aligned.apk"
  cp "${win_zipalign_dir}/aligned.apk" "${aligned_apk}"
else
  run_native_or_windows_tool "${zipalign}" -f -p 4 "${unsigned_apk}" "${aligned_apk}"
fi

echo "signing"
java -jar "${apksigner_jar}" sign \
  --ks "${keystore}" \
  --ks-pass pass:android \
  --key-pass pass:android \
  --ks-key-alias androiddebugkey \
  --out "${debug_apk}" \
  "${aligned_apk}"

java -jar "${apksigner_jar}" verify --print-certs "${debug_apk}" >/dev/null

echo "ok: ${debug_apk}"
