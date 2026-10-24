#!/usr/bin/env bash
set -euo pipefail

export DEBIAN_FRONTEND=noninteractive
apt-get update -qq
apt-get install -y -qq --no-install-recommends \
    ca-certificates git pkg-config build-essential locales \
    rustc cargo rustfmt rust-clippy \
    libgtk-4-dev libadwaita-1-dev libwebkitgtk-6.0-dev libsqlite3-dev libssl-dev \
    librsvg2-common gettext >/dev/null
sed -i 's/^# *\(en_US.UTF-8\)/\1/' /etc/locale.gen
locale-gen >/dev/null
