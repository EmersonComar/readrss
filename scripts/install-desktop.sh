#!/usr/bin/env bash
set -euo pipefail

APP_ID="io.github.readrss.ReadRSS"
SERVICE="readrss.service"
ROOT="$(cd "$(dirname "$0")/.." && pwd)"

DESTDIR="${DESTDIR:-}"
OPT_DIR="/opt/readrss"
BIN_LINK="/usr/local/bin/readrss"
SHARE="/usr/local/share"
UNIT_DIR="/etc/systemd/user"
DESKTOP_FILE="$SHARE/applications/$APP_ID.desktop"
ICON_FILES=(
    "scalable/apps/$APP_ID.svg"
    "symbolic/apps/$APP_ID-symbolic.svg"
)

log() { printf '\033[1m==>\033[0m %s\n' "$*"; }
die() { printf 'Erro: %s\n' "$*" >&2; exit 1; }

as_root() {
    if [[ -n "$DESTDIR" ]]; then "$@"; else sudo "$@"; fi
}

user_systemctl() {
    [[ -n "$DESTDIR" ]] && return 0
    systemctl --user "$@"
}

refresh_caches() {
    as_root gtk4-update-icon-cache -qtf "$DESTDIR$SHARE/icons/hicolor" 2>/dev/null ||
        as_root gtk-update-icon-cache -qtf "$DESTDIR$SHARE/icons/hicolor" 2>/dev/null || true
    as_root update-desktop-database -q "$DESTDIR$SHARE/applications" 2>/dev/null || true
}

quit_running_instance() {
    [[ -n "$DESTDIR" ]] && return 0
    user_systemctl stop "$SERVICE" 2>/dev/null || true
    local owner
    owner=$(gdbus call --session --dest org.freedesktop.DBus --object-path /org/freedesktop/DBus \
        --method org.freedesktop.DBus.NameHasOwner "$APP_ID" 2>/dev/null || echo "(false,)")
    if [[ "$owner" == "(true,)" ]]; then
        log "Fechando o readrss que está aberto"
        gdbus call --session --dest "$APP_ID" --object-path "/${APP_ID//./\/}" \
            --method org.freedesktop.Application.ActivateAction quit "[]" "{}" >/dev/null || true
        sleep 2
    fi
}

remove_legacy_user_install() {
    [[ -n "$DESTDIR" ]] && return 0
    local data="${XDG_DATA_HOME:-$HOME/.local/share}"
    rm -f "$data/applications/$APP_ID.desktop"
    local removed=0
    for icon in "${ICON_FILES[@]}"; do
        if [[ -e "$data/icons/hicolor/$icon" ]] && rm -f "$data/icons/hicolor/$icon" 2>/dev/null; then
            removed=1
        fi
    done
    if [[ $removed == 1 || -f "$data/icons/hicolor/icon-theme.cache" ]]; then
        gtk4-update-icon-cache -qtf "$data/icons/hicolor" 2>/dev/null ||
            gtk-update-icon-cache -qtf "$data/icons/hicolor" 2>/dev/null || true
    fi
}

check_requirements() {
    command -v cargo >/dev/null || die "cargo não encontrado. Instale com: sudo apt install rustc cargo"
    pkg-config --exists gtk4 libadwaita-1 webkitgtk-6.0 sqlite3 openssl || die "faltam bibliotecas de desenvolvimento. Instale com:
  sudo apt install libgtk-4-dev libadwaita-1-dev libwebkitgtk-6.0-dev libsqlite3-dev libssl-dev gstreamer1.0-plugins-bad"
}

write_desktop_file() {
    cat > "$1" <<EOF
[Desktop Entry]
Type=Application
Name=readrss
Comment=Leitor de feeds RSS/Atom
Comment[en]=RSS/Atom feed reader
Comment[es]=Lector de feeds RSS/Atom
Exec=$OPT_DIR/bin/readrss
Icon=$APP_ID
Terminal=false
Categories=Network;News;Feed;
Keywords=rss;atom;feed;news;notícias;noticias;
StartupNotify=true
StartupWMClass=$APP_ID
X-GNOME-UsesNotifications=true
EOF
}

write_service_file() {
    cat > "$1" <<EOF
[Unit]
Description=readrss: sincronização de feeds RSS/Atom em segundo plano
PartOf=graphical-session.target
After=graphical-session.target

[Service]
Type=simple
ExecStart=$OPT_DIR/bin/readrss --background
Restart=on-failure
RestartSec=10

[Install]
WantedBy=graphical-session.target
EOF
}

do_install() {
    check_requirements
    stage=$(mktemp -d)
    trap 'rm -rf "$stage"' EXIT

    log "Compilando (cargo build --release)"
    cargo build --release --quiet --manifest-path "$ROOT/Cargo.toml"

    log "Montando a instalação"
    install -Dm755 "$ROOT/target/release/readrss" "$stage/readrss/bin/readrss"
    mkdir -p "$stage/readrss/share/readrss"
    cp -R "$ROOT/data/locale" "$ROOT/data/icons" "$stage/readrss/share/readrss/"
    "$stage/readrss/bin/readrss" --help >/dev/null || die "o executável montado não funciona."

    write_desktop_file "$stage/$APP_ID.desktop"
    write_service_file "$stage/$SERVICE"

    quit_running_instance

    log "Instalando em $DESTDIR$OPT_DIR"
    [[ -z "$DESTDIR" ]] && echo "   (será pedida a senha do sudo)"
    as_root mkdir -p "$(dirname "$DESTDIR$OPT_DIR")"
    as_root rm -rf "$DESTDIR$OPT_DIR.new" "$DESTDIR$OPT_DIR.old"
    as_root cp -R "$stage/readrss" "$DESTDIR$OPT_DIR.new"
    as_root chmod -R u+rwX,go+rX,go-w "$DESTDIR$OPT_DIR.new"
    if [[ -e "$DESTDIR$OPT_DIR" ]]; then
        as_root mv "$DESTDIR$OPT_DIR" "$DESTDIR$OPT_DIR.old"
    fi
    as_root mv "$DESTDIR$OPT_DIR.new" "$DESTDIR$OPT_DIR"
    as_root rm -rf "$DESTDIR$OPT_DIR.old"

    as_root mkdir -p "$(dirname "$DESTDIR$BIN_LINK")"
    as_root ln -sfn "$OPT_DIR/bin/readrss" "$DESTDIR$BIN_LINK"
    for icon in "${ICON_FILES[@]}"; do
        as_root install -Dm644 "$ROOT/data/icons/hicolor/$icon" "$DESTDIR$SHARE/icons/hicolor/$icon"
    done
    as_root install -Dm644 "$stage/$APP_ID.desktop" "$DESTDIR$DESKTOP_FILE"
    as_root install -Dm644 "$stage/$SERVICE" "$DESTDIR$UNIT_DIR/$SERVICE"
    refresh_caches
    remove_legacy_user_install

    log "Ativando o serviço de usuário $SERVICE"
    user_systemctl daemon-reload
    user_systemctl enable "$SERVICE" --quiet
    user_systemctl restart "$SERVICE"

    log "Pronto. Abra o readrss pelo menu de aplicativos ou com: readrss"
    if [[ -z "$DESTDIR" ]]; then
        echo "   Status do serviço: systemctl --user status $SERVICE"
        echo "   Logs:              journalctl --user -u $SERVICE"
    fi
}

do_uninstall() {
    quit_running_instance
    log "Desativando o serviço"
    user_systemctl disable --now "$SERVICE" --quiet 2>/dev/null || true

    log "Removendo arquivos"
    [[ -z "$DESTDIR" ]] && echo "   (será pedida a senha do sudo)"
    as_root rm -rf "$DESTDIR$OPT_DIR"
    as_root rm -f "$DESTDIR$BIN_LINK" "$DESTDIR$DESKTOP_FILE" "$DESTDIR$UNIT_DIR/$SERVICE"
    for icon in "${ICON_FILES[@]}"; do
        as_root rm -f "$DESTDIR$SHARE/icons/hicolor/$icon"
    done
    refresh_caches
    user_systemctl daemon-reload

    log "readrss removido. Seus feeds e artigos continuam em ~/.local/share/readrss"
}

case "${1:-}" in
    "") do_install ;;
    --uninstall) do_uninstall ;;
    *) die "opção desconhecida: $1 (use --uninstall ou nenhuma)" ;;
esac
