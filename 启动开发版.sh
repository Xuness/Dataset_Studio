#!/usr/bin/env bash
set -euo pipefail
studio_root=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)
exec bash "$studio_root/tooling/start-dev.sh" "$@"
