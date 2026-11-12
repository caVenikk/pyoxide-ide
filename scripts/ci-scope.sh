#!/usr/bin/env bash
# CI orchestration lives in a testable Python module shared with the final gate.
set -euo pipefail
exec python3 "$(dirname "$0")/ci-control.py" "${1:-scope}"
