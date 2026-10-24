#!/usr/bin/env bash
set -euo pipefail

APP_ID="io.github.readrss.ReadRSS"
SERVICE="readrss.service"
HERE="$(cd "$(dirname "$0")" && pwd)"

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
RUNTIME_PACKAGES="libgtk-4-1 libadwaita-1-0 libwebkitgtk-6.0-4 libsqlite3-0 gstreamer1.0-plugins-bad"
BUILD_PACKAGES="rustc cargo libgtk-4-dev libadwaita-1-dev libwebkitgtk-6.0-dev libsqlite3-dev libssl-dev gstreamer1.0-plugins-bad"

log() { printf '\033[1m==>\033[0m %s\n' "$*"; }
die() { printf 'Erro: %s\n' "$*" >&2; exit 1; }

usage() {
    cat <<EOF
Uso: $0 [--uninstall]

Instala (ou atualiza) o readrss em $OPT_DIR e ativa o serviço de usuário $SERVICE.
Rodado de dentro de um pacote de release (com bin/readrss ao lado), instala o binário pronto;
rodado a partir do código-fonte (scripts/install.sh), compila antes de instalar.

  --uninstall   remove o readrss (seus feeds e artigos são mantidos)
EOF
}

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

prepare_prebuilt() {
    local missing
    missing=$(ldd "$HERE/bin/readrss" 2>/dev/null | awk '/not found/{print $1}' | tr '\n' ' ')
    [[ -z "$missing" ]] || die "faltam bibliotecas do sistema ($missing). Instale com:
  sudo apt install $RUNTIME_PACKAGES
O binário pronto exige Ubuntu 26.04 ou equivalente; em sistemas mais antigos, compile a partir do código-fonte."
    payload="$HERE"
}

prepare_from_source() {
    local root="$HERE/.."
    command -v cargo >/dev/null || die "cargo não encontrado. Instale com: sudo apt install $BUILD_PACKAGES"
    pkg-config --exists gtk4 libadwaita-1 webkitgtk-6.0 sqlite3 openssl ||
        die "faltam bibliotecas de desenvolvimento. Instale com: sudo apt install $BUILD_PACKAGES"
    log "Compilando (cargo build --release)"
    cargo build --release --locked --quiet --manifest-path "$root/Cargo.toml"
    payload="$stage/payload"
    install -Dm755 "$root/target/release/readrss" "$payload/bin/readrss"
    mkdir -p "$payload/share/readrss"
    cp -R "$root/data/locale" "$root/data/icons" "$payload/share/readrss/"
}

do_install() {
    stage=$(mktemp -d)
    trap 'rm -rf "$stage"' EXIT
    if [[ -x "$HERE/bin/readrss" ]]; then
        prepare_prebuilt
    elif [[ -f "$HERE/../Cargo.toml" ]]; then
        prepare_from_source
    else
        die "rode este script de dentro do pacote de release ou a partir do código-fonte (scripts/install.sh)."
    fi
    "$payload/bin/readrss" --help >/dev/null || die "o executável não funciona neste sistema."

    write_desktop_file "$stage/$APP_ID.desktop"
    write_service_file "$stage/$SERVICE"
    quit_running_instance

    log "Instalando em $DESTDIR$OPT_DIR"
    [[ -z "$DESTDIR" ]] && echo "   (será pedida a senha do sudo)"
    as_root mkdir -p "$(dirname "$DESTDIR$OPT_DIR")"
    as_root rm -rf "$DESTDIR$OPT_DIR.new" "$DESTDIR$OPT_DIR.old"
    as_root mkdir -p "$DESTDIR$OPT_DIR.new"
    as_root cp -R "$payload/bin" "$payload/share" "$DESTDIR$OPT_DIR.new/"
    as_root chmod -R u+rwX,go+rX,go-w "$DESTDIR$OPT_DIR.new"
    if [[ -e "$DESTDIR$OPT_DIR" ]]; then
        as_root mv "$DESTDIR$OPT_DIR" "$DESTDIR$OPT_DIR.old"
    fi
    as_root mv "$DESTDIR$OPT_DIR.new" "$DESTDIR$OPT_DIR"
    as_root rm -rf "$DESTDIR$OPT_DIR.old"

    as_root mkdir -p "$(dirname "$DESTDIR$BIN_LINK")"
    as_root ln -sfn "$OPT_DIR/bin/readrss" "$DESTDIR$BIN_LINK"
    for icon in "${ICON_FILES[@]}"; do
        as_root install -Dm644 "$payload/share/readrss/icons/hicolor/$icon" "$DESTDIR$SHARE/icons/hicolor/$icon"
    done
    as_root install -Dm644 "$stage/$APP_ID.desktop" "$DESTDIR$DESKTOP_FILE"
    as_root install -Dm644 "$stage/$SERVICE" "$DESTDIR$UNIT_DIR/$SERVICE"
    refresh_caches

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
    -h | --help) usage ;;
    *) usage >&2; exit 2 ;;
esac
