//! Diálogos "Sobre" e "Changelog".

use adw::prelude::*;
use gtk::glib;

use crate::changelog::{RELEASES, release_notes_html, sections};
use crate::constants::APP_ID;
use crate::html::format_date;
use crate::i18n::{fill, gettext};

pub const APP_NAME: &str = "readrss";
pub const AUTHOR: &str = "Emerson Comar";
pub const REPOSITORY_URL: &str = "";
pub const NEWSFLASH_URL: &str = "https://gitlab.com/news-flash/news_flash_gtk";

/// Texto da página Detalhes do Sobre (marcação Pango): descrição, inspiração e desenvolvimento.
pub fn details_markup() -> String {
    let escape = |text: &str| glib::markup_escape_text(text).to_string();
    let link = format!("<a href=\"{NEWSFLASH_URL}\">NewsFlash</a>");
    let inspired = escape(&gettext("Inspired by {newsflash}.")).replace("{newsflash}", &link);
    [
        escape(&gettext("RSS/Atom feed reader for the Linux desktop")),
        inspired,
        escape(&gettext("Developed with the help of artificial intelligence to meet some particular needs of the author.")),
    ]
    .join("\n\n")
}

pub fn show_about(parent: &impl IsA<gtk::Widget>) -> adw::AboutDialog {
    let latest = &RELEASES[0];
    let dialog = adw::AboutDialog::builder()
        .application_name(APP_NAME)
        .application_icon(APP_ID)
        .version(env!("CARGO_PKG_VERSION"))
        .developer_name(AUTHOR)
        .developers([AUTHOR])
        .copyright(format!("© 2026 {AUTHOR}"))
        .license_type(gtk::License::Gpl30)
        .comments(details_markup())
        .release_notes_version(latest.version)
        .release_notes(release_notes_html(latest))
        .build();
    dialog.add_link("NewsFlash", NEWSFLASH_URL);
    if !REPOSITORY_URL.is_empty() {
        dialog.set_website(REPOSITORY_URL);
        dialog.set_issue_url(&format!("{}/issues", REPOSITORY_URL.trim_end_matches('/')));
    }
    dialog.present(Some(parent));
    dialog
}

pub fn show_changelog(parent: &impl IsA<gtk::Widget>) -> adw::Dialog {
    let page = adw::PreferencesPage::new();
    for release in RELEASES {
        let (y, m, d) = release.date;
        let timestamp = chrono::NaiveDate::from_ymd_opt(y, m, d)
            .and_then(|date| date.and_hms_opt(12, 0, 0))
            .map(|dt| dt.and_utc().timestamp() as f64)
            .unwrap_or_default();
        let group = adw::PreferencesGroup::builder()
            .title(fill(&gettext("Version {version}"), &[("version", &release.version)]))
            .description(format_date(timestamp, &gettext("%m/%d/%Y")))
            .build();
        for section in sections(release) {
            let expander = adw::ExpanderRow::builder().title(&section.title).expanded(true).build();
            expander.add_prefix(&gtk::Image::from_icon_name(section.icon));
            for item in &section.items {
                expander.add_row(&adw::ActionRow::builder().title(item).title_lines(0).use_markup(false).build());
            }
            group.add(&expander);
        }
        page.add(&group);
    }
    let view = adw::ToolbarView::new();
    view.add_top_bar(&adw::HeaderBar::new());
    view.set_content(Some(&page));
    let dialog = adw::Dialog::builder()
        .title(gettext("Changelog"))
        .child(&view)
        .content_width(600)
        .content_height(640)
        .build();
    dialog.present(Some(parent));
    dialog
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn details_markup_is_valid_pango_with_link() {
        let markup = details_markup();
        let link = format!("<a href=\"{NEWSFLASH_URL}\">NewsFlash</a>");
        assert!(markup.contains(&link));
        let plain = markup.replace(&link, "NewsFlash");
        assert!(gtk::pango::parse_markup(&plain, '\0').is_ok(), "links à parte (extensão do GtkLabel), o resto é Pango válido: {markup}");
        assert!(markup.contains("artificial intelligence"));
    }
}
