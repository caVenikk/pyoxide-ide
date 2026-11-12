#!/usr/bin/env bash
# Exercise gate/proof rejection cases, including stale draft survivors (#335).
set -euo pipefail
here=$(cd "$(dirname "$0")" && pwd)
exec python3 "$here/test_ci_control.py"
