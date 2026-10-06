#!/bin/sh
# Runs ConfigProbe.java on BlueMap 5.28's real Configurate stack. Usage: probe.sh raw <files…> | probe.sh typed <Class> <file>
# tests/hocon/*.expected were produced with: probe.sh expect tests/hocon/*.conf
here="$(cd "$(dirname "$0")" && pwd)"
main="$(git -C "$here" rev-parse --path-format=absolute --git-common-dir)/.."
dl="${BM_DOWNLOADS:-$main/work/downloads}"
exec "$dl/jdk25/bin/java" -cp "$dl/bluemap-5.28-cli.jar" "$here/ConfigProbe.java" "$@"
