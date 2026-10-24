use std::path::PathBuf;

pub const APP_ID: &str = "io.github.readrss.ReadRSS";
pub const FILTER_ICON: &str = "io.github.readrss.ReadRSS-filter-symbolic";
pub const DOMAIN: &str = "readrss";

const DEV_SHARE: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/data");

fn installed_share() -> Option<PathBuf> {
    let exe = std::env::current_exe().ok()?.canonicalize().ok()?;
    let share = exe.parent()?.parent()?.join("share").join("readrss");
    share.is_dir().then_some(share)
}

fn resource(name: &str, env: &str) -> PathBuf {
    if let Some(dir) = std::env::var_os(env) {
        return PathBuf::from(dir);
    }
    installed_share().map(|share| share.join(name)).unwrap_or_else(|| PathBuf::from(DEV_SHARE).join(name))
}

/// Catálogos de tradução: instalado em `<prefixo>/share/readrss/locale`, no repositório
/// em `data/locale`.
pub fn locale_dir() -> PathBuf {
    resource("locale", "READRSS_LOCALEDIR")
}

/// Tema de ícones embutido: `hicolor/{scalable,symbolic}/apps/<APP_ID>[-symbolic].svg`.
pub fn icons_dir() -> PathBuf {
    resource("icons", "READRSS_ICONSDIR")
}
