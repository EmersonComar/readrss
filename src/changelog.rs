//! Histórico de versões exibido no app (menu → Changelog e "Novidades" do Sobre).
//!
//! Ao lançar uma versão: adicione um `Release` no topo de `RELEASES` com a mesma versão do
//! Cargo.toml (há teste garantindo isso) e traduza as novas entradas.

use crate::html::escape;
use crate::i18n::{N_, gettext};

pub struct Release {
    pub version: &'static str,
    pub date: (i32, u32, u32),
    pub added: &'static [&'static str],
    pub changed: &'static [&'static str],
    pub fixed: &'static [&'static str],
}

pub struct Section {
    pub title: String,
    pub icon: &'static str,
    pub items: Vec<String>,
}

pub const RELEASES: &[Release] = &[Release {
    version: "0.1.0",
    date: (2026, 10, 20),
    added: &[
        N_("Subscribe to RSS/Atom feeds by entering the feed or website address; the feed is detected automatically."),
        N_(
            "Three reading modes: feed summary, full article extracted from the website, and the original page inside the app.",
        ),
        N_("Reliable website icons, validated and cached, with an initials avatar when no icon is available."),
        N_("Automatic sync every 5 minutes, 30 minutes or 1 hour, downloading only feeds that changed."),
        N_("Notifications for new articles and an option to keep syncing in the background."),
        N_("Unread and starred articles, search, and mark all as read."),
        N_("OPML import and export to migrate from other feed readers."),
        N_("Interface in Portuguese, English and Spanish, switchable without restarting."),
        N_("Keyboard shortcuts to navigate and read articles."),
        N_("About dialog and changelog in the main menu."),
        N_("Application icon, also used by the launcher."),
        N_(
            "System installation in /opt with a background service that starts with the session and keeps syncing and notifying without the window open.",
        ),
        N_("Edit a feed's address and name from its context menu."),
        N_("Per-feed filters by keywords or regular expressions, to hide matching articles or show only them."),
    ],
    changed: &[],
    fixed: &[
        N_("Window buttons no longer disappear when the reader pane is made narrow."),
        N_("No more graphics warnings when maximizing or resizing the window."),
        N_("Notifications for a newly added feed show its name instead of its address."),
        N_("Reading articles from many websites no longer piles up browser processes and memory."),
    ],
}];

/// Seções não vazias da versão, já traduzidas.
pub fn sections(release: &Release) -> Vec<Section> {
    [
        (N_("Added"), "list-add-symbolic", release.added),
        (N_("Changed"), "document-edit-symbolic", release.changed),
        (N_("Fixed"), "object-select-symbolic", release.fixed),
    ]
    .into_iter()
    .filter(|(_, _, items)| !items.is_empty())
    .map(|(title, icon, items)| Section {
        title: gettext(title),
        icon,
        items: items.iter().map(|item| gettext(*item)).collect(),
    })
    .collect()
}

/// Notas no subconjunto de HTML aceito por `AdwAboutDialog` (p, ul, li).
pub fn release_notes_html(release: &Release) -> String {
    sections(release)
        .into_iter()
        .map(|s| {
            let items: String = s.items.iter().map(|i| format!("<li>{}</li>", escape(i))).collect();
            format!("<p>{}</p><ul>{items}</ul>", escape(&s.title))
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn version_key(version: &str) -> Vec<u32> {
        version.split('.').map(|p| p.parse().unwrap()).collect()
    }

    #[test]
    fn latest_release_matches_package_version() {
        assert_eq!(
            RELEASES[0].version,
            env!("CARGO_PKG_VERSION"),
            "versão do Cargo.toml sem entrada no changelog: adicione um Release no topo de src/changelog.rs"
        );
    }

    #[test]
    fn releases_are_unique_newest_first_and_not_empty() {
        let versions: Vec<_> = RELEASES.iter().map(|r| version_key(r.version)).collect();
        let mut sorted = versions.clone();
        sorted.sort_by(|a, b| b.cmp(a));
        sorted.dedup();
        assert_eq!(versions, sorted);
        let dates: Vec<_> = RELEASES.iter().map(|r| r.date).collect();
        let mut sorted_dates = dates.clone();
        sorted_dates.sort_by(|a, b| b.cmp(a));
        assert_eq!(dates, sorted_dates);
        assert!(RELEASES.iter().all(|r| !(r.added.is_empty() && r.changed.is_empty() && r.fixed.is_empty())));
    }

    #[test]
    fn release_notes_html_is_well_formed() {
        let html = release_notes_html(&RELEASES[0]);
        assert!(html.starts_with("<p>"));
        assert_eq!(html.matches("<ul>").count(), html.matches("</ul>").count());
        assert!(!html.contains("<script"));
    }
}
