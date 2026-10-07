#!/usr/bin/env bash
set -euo pipefail

studio_root=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd)
cd -- "$studio_root"
studio_web=false
for studio_argument in "$@"; do
  case "$studio_argument" in
    -h|--help)
      cat <<'USAGE'
Dataset Studio · Linux 源码开发启动器

用法：bash ./启动开发版.sh [--web] [--engine-profile=debug|release]

默认打开原生桌面窗口，Linux 默认使用 Debug 引擎。
--web 仅启动本机浏览器开发环境。
其余参数传给现有开发入口；PYTHON 可指定 DuckDB 安装步骤使用的解释器。
脚本检查 pnpm 依赖并准备 DuckDB，启动日志保存在 .local/logs/。
首次采集前另运行 pnpm setup:lake --dev，详情见 docs/architecture/linux-development.md。
USAGE
      exit 0
      ;;
    --web) studio_web=true ;;
  esac
done

fail() {
  printf '启动失败：%s\n参见 docs/architecture/linux-development.md。\n' "$1" >&2
  exit 1
}

[[ $(uname -s) == Linux && $(uname -m) == x86_64 ]] || fail '此启动器需要 Linux x86_64。'
for studio_tool in node pnpm cargo rustc tee; do
  command -v "$studio_tool" >/dev/null 2>&1 || fail "缺少 $studio_tool，请先安装开发环境。"
done
node -e 'const [major, minor] = process.versions.node.split(".").map(Number); process.exit(process.platform === "linux" && process.arch === "x64" && (major > 22 || major === 22 && minor >= 12) ? 0 : 1)' \
  || fail '需要 Linux 版 Node.js 22.12 或更新版本。'

if [[ -n ${PYTHON:-} ]]; then
  studio_python=$PYTHON
elif [[ -x "$studio_root/.local/runtime/lake-worker/bin/python" ]]; then
  studio_python="$studio_root/.local/runtime/lake-worker/bin/python"
else
  studio_python=python3
fi
command -v "$studio_python" >/dev/null 2>&1 || fail '缺少 Python 3，可通过 PYTHON 指定解释器。'
export PYTHON="$studio_python"

if [[ $studio_web == false ]]; then
  [[ -n ${DISPLAY:-} || -n ${WAYLAND_DISPLAY:-} ]] || fail '未检测到图形桌面会话；请在桌面或 WSLg 中运行，或使用 --web。'
  command -v pkg-config >/dev/null 2>&1 || fail '缺少 pkg-config，请安装文档列出的 Linux 桌面依赖。'
  pkg-config --exists gtk+-3.0 webkit2gtk-4.1 || fail '缺少 GTK / WebKitGTK 开发依赖。'
fi

studio_log_dir="$studio_root/.local/logs"
mkdir -p -- "$studio_log_dir"
studio_launch_key="linux-$(date +%Y%m%d-%H%M%S)-$$"
studio_dev_log="$studio_log_dir/startup-dev-$studio_launch_key.log"
run_setup() {
  local step=$1 log=$2
  shift 2
  printf '%s…\n' "$step"
  if "$@" >"$log" 2>&1; then
    return
  else
    local status=$?
    tail -n 25 -- "$log" >&2
    printf '%s失败，日志：%s\n' "$step" "$log" >&2
    exit "$status"
  fi
}

printf 'Dataset Studio · Linux 开发模式\n'
run_setup '检查依赖与锁文件' "$studio_log_dir/startup-install-$studio_launch_key.log" pnpm install --frozen-lockfile
run_setup '检查元数据运行库' "$studio_log_dir/startup-duckdb-$studio_launch_key.log" node tooling/setup-duckdb.mjs
if [[ ! -x "$studio_root/.local/runtime/lake-worker/bin/python" ]]; then
  printf '首次采集前请运行 pnpm setup:lake --dev，配置数据湖 Python 运行环境。\n'
fi
printf '启动日志：%s\n' "$studio_dev_log"
# Replace the shell so signals and exit status belong to the existing dev manager.
: >"$studio_dev_log"
exec node "$studio_root/tooling/dev.mjs" "$@" > >(tee -a -- "$studio_dev_log") 2>&1
