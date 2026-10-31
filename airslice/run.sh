#!/bin/sh
# Start AirSlice with the options from the add-on configuration.
set -eu

OPTIONS=/data/options.json
opt() {
  [ -f "$OPTIONS" ] || return 0
  jq -r --arg k "$1" '.[$k] // empty' "$OPTIONS"
}

level="$(opt log_level)"
case "$level" in
  warning) level=warn ;;
  "") level=info ;;
esac
export RUST_LOG="${level},tower_http=warn,rumqttc=warn"

password="$(opt admin_password)"
if [ -n "$password" ]; then
  export ADMIN_PASSWORD="$password"
else
  unset ADMIN_PASSWORD || true
fi

timeout="$(opt slice_timeout)"
[ -n "$timeout" ] && export SLICE_TIMEOUT_SECS="$timeout"
jobs="$(opt max_jobs)"
[ -n "$jobs" ] && export MAX_JOBS="$jobs"

# Home Assistant ingress requests come from the Supervisor and are already
# authenticated; everything else (the optional direct port) needs the
# AirSlice password or an API key.
export INGRESS_TRUSTED_PEER="${INGRESS_TRUSTED_PEER:-172.30.32.2}"
export DATA_DIR=/data
export BIND="0.0.0.0:8080"
export NO_COLOR=1

echo "Starting AirSlice (log level ${level})"
exec airslice
