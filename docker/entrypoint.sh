#!/bin/sh
# One image for Docker and the Home Assistant add-on.
# - Add-on (the Supervisor passes a token and writes /data/options.json):
#   run with the add-on options as root, which owns the add-on's /data.
# - Docker: run as the unprivileged user 1000, also when started as root.
set -eu

if [ -n "${SUPERVISOR_TOKEN:-}" ] && [ -f /data/options.json ]; then
  exec /usr/local/bin/airslice-addon
fi
if [ "$(id -u)" = "0" ]; then
  exec setpriv --reuid=1000 --regid=1000 --init-groups --inh-caps=-all env HOME=/home/airslice airslice "$@"
fi
exec airslice "$@"
