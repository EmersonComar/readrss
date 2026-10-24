//! Diálogos do menu de contexto de um feed: Editar e Filtrar.

use std::cell::RefCell;
use std::rc::Rc;
use std::time::Duration;

use adw::prelude::*;
use gtk::glib;

use crate::db::{Db, Feed};
use crate::filters::{FeedFilter, Mode};
use crate::i18n::{fill, gettext, ngettext};

const PREVIEW_DELAY: Duration = Duration::from_millis(250);
const MODES: [Option<Mode>; 3] = [None, Some(Mode::Hide), Some(Mode::Only)];

/// Edita URL e nome; `on_save(url, nome)` recebe os textos já sem espaços nas pontas.
pub fn show_edit_feed(
    parent: &impl IsA<gtk::Widget>,
    feed: &Feed,
    on_save: impl Fn(String, String) + 'static,
) -> adw::AlertDialog {
    let url_row = adw::EntryRow::builder().title(gettext("Feed or website address")).text(&feed.url).build();
    let name_row =
        adw::EntryRow::builder().title(gettext("Name")).text(feed.custom_title.as_deref().unwrap_or_default()).build();
    let fallback = if feed.title.is_empty() { &feed.url } else { &feed.title };
    let group = adw::PreferencesGroup::builder()
        .description(fill(
            &gettext("Leave the name empty to use the title provided by the feed: {title}"),
            &[("title", fallback)],
        ))
        .build();
    group.add(&url_row);
    group.add(&name_row);

    let dialog = adw::AlertDialog::builder()
        .heading(gettext("Edit feed"))
        .extra_child(&group)
        .default_response("save")
        .close_response("cancel")
        .build();
    dialog.add_response("cancel", &gettext("Cancel"));
    dialog.add_response("save", &gettext("Save"));
    dialog.set_response_appearance("save", adw::ResponseAppearance::Suggested);

    let weak_dialog = dialog.downgrade();
    url_row.connect_changed(move |row| {
        if let Some(dialog) = weak_dialog.upgrade() {
            dialog.set_response_enabled("save", !row.text().trim().is_empty());
        }
    });
    dialog.connect_response(None, move |_, response| {
        if response == "save" {
            on_save(url_row.text().trim().to_owned(), name_row.text().trim().to_owned());
        }
    });
    dialog.present(Some(parent));
    dialog
}

/// Configura o filtro do feed, com validação e prévia de quantos artigos seriam ocultados.
pub struct FilterDialog {
    pub dialog: adw::Dialog,
    pub mode_row: adw::ComboRow,
    pub regex_row: adw::SwitchRow,
    pub text_view: gtk::TextView,
    pub error_label: gtk::Label,
    pub preview_label: gtk::Label,
    pub save_button: gtk::Button,
    db: Rc<Db>,
    feed_id: i64,
    pending: RefCell<Option<glib::SourceId>>,
}

pub fn show_filter_feed(
    parent: &impl IsA<gtk::Widget>,
    db: Rc<Db>,
    feed: &Feed,
    on_saved: impl Fn(usize, bool) + 'static,
) -> Rc<FilterDialog> {
    let current = db.get_filter(feed.id);
    let mode_row = adw::ComboRow::builder()
        .title(gettext("Mode"))
        .use_subtitle(true)
        .model(&gtk::StringList::new(&[
            &gettext("Off"),
            &gettext("Hide articles that match"),
            &gettext("Show only articles that match"),
        ]))
        .build();
    mode_row.set_selected(MODES.iter().position(|m| *m == current.as_ref().map(|f| f.mode)).unwrap_or(0) as u32);
    let regex_row = adw::SwitchRow::builder()
        .title(gettext("Use regular expressions"))
        .subtitle(gettext("Each line is a regular expression"))
        .active(current.as_ref().is_some_and(|f| f.use_regex))
        .build();
    let text_view = gtk::TextView::builder()
        .wrap_mode(gtk::WrapMode::WordChar)
        .top_margin(8)
        .bottom_margin(8)
        .left_margin(10)
        .right_margin(10)
        .accepts_tab(false)
        .monospace(regex_row.is_active())
        .build();
    text_view.buffer().set_text(&current.as_ref().map(FeedFilter::rules_text).unwrap_or_default());
    let scroller = gtk::ScrolledWindow::builder()
        .child(&text_view)
        .min_content_height(160)
        .css_classes(["card"])
        .hscrollbar_policy(gtk::PolicyType::Never)
        .build();
    let error_label =
        gtk::Label::builder().xalign(0.0).wrap(true).css_classes(["error"]).visible(false).margin_top(8).build();
    let preview_label = gtk::Label::builder().xalign(0.0).wrap(true).css_classes(["dim-label"]).margin_top(8).build();

    let settings = adw::PreferencesGroup::new();
    settings.add(&mode_row);
    settings.add(&regex_row);
    let keywords = adw::PreferencesGroup::builder()
        .title(gettext("Keywords"))
        .description(gettext("One per line. Searched in the title and summary, ignoring case and accents. Plain words match whole words only."))
        .build();
    let column = gtk::Box::new(gtk::Orientation::Vertical, 0);
    column.append(&scroller);
    column.append(&error_label);
    column.append(&preview_label);
    keywords.add(&column);
    let page = adw::PreferencesPage::new();
    page.add(&settings);
    page.add(&keywords);

    let cancel = gtk::Button::with_label(&gettext("Cancel"));
    let save_button = gtk::Button::builder().label(gettext("Save")).css_classes(["suggested-action"]).build();
    let header = adw::HeaderBar::builder().show_start_title_buttons(false).show_end_title_buttons(false).build();
    header.pack_start(&cancel);
    header.pack_end(&save_button);
    let view = adw::ToolbarView::new();
    view.add_top_bar(&header);
    view.set_content(Some(&page));
    let dialog = adw::Dialog::builder()
        .title(fill(&gettext("Filter “{title}”"), &[("title", &feed.display_title())]))
        .content_width(540)
        .content_height(600)
        .child(&view)
        .build();

    let this = Rc::new(FilterDialog {
        dialog,
        mode_row,
        regex_row,
        text_view,
        error_label,
        preview_label,
        save_button,
        db,
        feed_id: feed.id,
        pending: RefCell::new(None),
    });

    let weak = Rc::downgrade(&this);
    cancel.connect_clicked(move |_| {
        if let Some(this) = weak.upgrade() {
            this.dialog.close();
        }
    });
    let weak = Rc::downgrade(&this);
    let on_saved = Rc::new(on_saved);
    this.save_button.connect_clicked(move |_| {
        let Some(this) = weak.upgrade() else { return };
        let filter = this.current_filter();
        match this.db.set_filter(this.feed_id, filter.as_ref()) {
            Ok(hidden) => {
                this.dialog.close();
                on_saved(hidden, filter.is_some());
            }
            Err(_) => this.update(),
        }
    });
    let schedule = {
        let weak = Rc::downgrade(&this);
        Rc::new(move || {
            if let Some(this) = weak.upgrade() {
                this.schedule_update();
            }
        })
    };
    let s = schedule.clone();
    this.mode_row.connect_selected_notify(move |_| s());
    let s = schedule.clone();
    this.regex_row.connect_active_notify(move |_| s());
    this.text_view.buffer().connect_changed(move |_| schedule());
    this.update();
    this.dialog.present(Some(parent));
    let keep = this.clone();
    this.dialog.connect_closed(move |_| {
        let _ = &keep;
    });
    this
}

impl FilterDialog {
    /// Filtro descrito pelo formulário; `None` quando desligado ou sem regras.
    pub fn current_filter(&self) -> Option<FeedFilter> {
        let mode = MODES.get(self.mode_row.selected() as usize).copied().flatten()?;
        let buffer = self.text_view.buffer();
        let text = buffer.text(&buffer.start_iter(), &buffer.end_iter(), false);
        let filter = FeedFilter::from_text(mode, &text, self.regex_row.is_active());
        (!filter.rules.is_empty()).then_some(filter)
    }

    fn schedule_update(self: &Rc<Self>) {
        self.text_view.set_monospace(self.regex_row.is_active());
        if let Some(source) = self.pending.borrow_mut().take() {
            source.remove();
        }
        let weak = Rc::downgrade(self);
        let source = glib::timeout_add_local_once(PREVIEW_DELAY, move || {
            if let Some(this) = weak.upgrade() {
                this.pending.borrow_mut().take();
                this.update();
            }
        });
        *self.pending.borrow_mut() = Some(source);
    }

    pub fn update(&self) {
        self.error_label.set_visible(false);
        self.save_button.set_sensitive(true);
        let Some(filter) = self.current_filter() else {
            self.preview_label.set_label(&gettext("No filter: all articles of this feed are shown."));
            return;
        };
        match self.db.preview_filter(self.feed_id, &filter) {
            Ok((hidden, total)) => {
                let text = ngettext(
                    "{hidden} of {total} current articles would be hidden.",
                    "{hidden} of {total} current articles would be hidden.",
                    hidden as u32,
                );
                self.preview_label.set_label(&fill(&text, &[("hidden", &hidden), ("total", &total)]));
            }
            Err(error) => {
                let text = fill(
                    &gettext("Line {line}: invalid regular expression ({error})"),
                    &[("line", &error.line), ("error", &error.message)],
                );
                self.error_label.set_label(&text);
                self.error_label.set_visible(true);
                self.preview_label.set_label("");
                self.save_button.set_sensitive(false);
            }
        }
    }
}
