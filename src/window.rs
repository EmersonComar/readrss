//! Janela principal: fontes | lista de artigos | leitor.

use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::rc::{Rc, Weak};
use std::time::SystemTime;

use adw::prelude::*;
use gtk::{gdk, gio, glib, pango};
use webkit6::prelude::*;

use crate::app::App;
use crate::constants::FILTER_ICON;
use crate::db::{Article, ArticleQuery, Db, Feed, now};
use crate::i18n::{fill, gettext, ngettext_n};
use crate::{feed_dialogs, html, net};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Source {
    All,
    Unread,
    Starred,
    Feed(i64),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mode {
    Summary,
    Full,
    Web,
}

impl Mode {
    pub fn name(self) -> &'static str {
        match self {
            Mode::Summary => "summary",
            Mode::Full => "full",
            Mode::Web => "web",
        }
    }

    fn from_name(name: Option<&str>) -> Mode {
        match name {
            Some("full") => Mode::Full,
            Some("web") => Mode::Web,
            _ => Mode::Summary,
        }
    }
}

type WindowAction = (&'static str, fn(&Rc<Window>));
type FeedAction = (&'static str, fn(&Rc<Window>, i64));

pub struct ArticleRow {
    pub article: Article,
    pub row: gtk::ListBoxRow,
    title: gtk::Label,
    star: gtk::Image,
}

pub struct SidebarRow {
    pub source: Option<Source>,
    pub row: gtk::ListBoxRow,
    pub icons: Vec<String>,
}

#[derive(Default)]
struct State {
    source: Option<Source>,
    current: Option<Article>,
    mode: Option<Mode>,
    feeds: Vec<Feed>,
    sidebar: Vec<SidebarRow>,
    rows: Vec<ArticleRow>,
}

pub struct Window {
    pub window: adw::ApplicationWindow,
    app: Weak<App>,
    db: Rc<Db>,
    toasts: adw::ToastOverlay,
    pub sidebar_list: gtk::ListBox,
    pub article_list: gtk::ListBox,
    pub list_title: adw::WindowTitle,
    list_stack: gtk::Stack,
    pub search_entry: gtk::SearchEntry,
    search_bar: gtk::SearchBar,
    sync_stack: gtk::Stack,
    pub mode_group: adw::ToggleGroup,
    star_button: gtk::Button,
    pub reader: webkit6::WebView,
    content_stack: gtk::Stack,
    progress: gtk::ProgressBar,
    pub site_view: RefCell<Option<webkit6::WebView>>,
    pub reader_pane: adw::ToolbarView,
    article_actions: Vec<gio::SimpleAction>,
    state: RefCell<State>,
    reloading: Cell<bool>,
    textures: RefCell<HashMap<String, (SystemTime, gdk::Texture)>>,
    pub filter_dialog: RefCell<Weak<feed_dialogs::FilterDialog>>,
}

pub fn relative_time(timestamp: f64) -> String {
    let delta = (now() - timestamp).max(0.0) as i64;
    match delta {
        0..60 => gettext("now"),
        // Translators: abbreviated "minutes ago" in the article list
        60..3600 => fill(&gettext("{n} min"), &[("n", &(delta / 60))]),
        // Translators: abbreviated "hours ago" in the article list
        3600..86400 => fill(&gettext("{n} h"), &[("n", &(delta / 3600))]),
        // Translators: abbreviated "days ago" in the article list
        86400..604800 => fill(&gettext("{n} d"), &[("n", &(delta / 86400))]),
        // Translators: short date in the article list (strftime format)
        _ => html::format_date(timestamp, &gettext("%m/%d/%y")),
    }
}

impl Window {
    pub fn new(app: &Rc<App>, source: Source) -> Rc<Window> {
        let window = adw::ApplicationWindow::builder()
            .application(&app.application)
            .title("readrss")
            .default_width(1300)
            .default_height(820)
            .build();

        let sidebar_list = gtk::ListBox::new();
        sidebar_list.add_css_class("navigation-sidebar");
        let article_list = gtk::ListBox::new();
        article_list.set_show_separators(true);
        let list_title = adw::WindowTitle::new(&gettext("Unread"), "");
        let search_entry =
            gtk::SearchEntry::builder().placeholder_text(gettext("Search titles and summaries")).hexpand(true).build();
        let search_bar = gtk::SearchBar::builder().child(&search_entry).build();
        search_bar.connect_entry(&search_entry);
        let sync_stack = gtk::Stack::new();
        sync_stack.add_named(&gtk::Image::from_icon_name("view-refresh-symbolic"), Some("idle"));
        sync_stack.add_named(&adw::Spinner::new(), Some("busy"));
        let list_stack = gtk::Stack::new();
        let mode_group = adw::ToggleGroup::new();
        let star_button = gtk::Button::builder()
            .icon_name("non-starred-symbolic")
            .action_name("win.toggle-star")
            .tooltip_text(gettext("Star (S)"))
            .build();
        let reader = webkit6::WebView::new();
        reader.set_vexpand(true);
        reader.set_hexpand(true);
        if let Some(settings) = WebViewExt::settings(&reader) {
            settings.set_enable_javascript(false);
            settings.set_enable_developer_extras(false);
        }
        let content_stack = gtk::Stack::new();
        content_stack.add_named(&reader, Some("reader"));
        let progress =
            gtk::ProgressBar::builder().css_classes(["osd"]).valign(gtk::Align::Start).visible(false).build();
        let reader_pane = adw::ToolbarView::new();
        let toasts = adw::ToastOverlay::new();

        let this = Rc::new(Window {
            window,
            app: Rc::downgrade(app),
            db: app.db.clone(),
            toasts,
            sidebar_list,
            article_list,
            list_title,
            list_stack,
            search_entry,
            search_bar,
            sync_stack,
            mode_group,
            star_button,
            reader,
            content_stack,
            progress,
            site_view: RefCell::new(None),
            reader_pane,
            article_actions: Vec::new(),
            state: RefCell::new(State { source: Some(source), mode: Some(Mode::Summary), ..Default::default() }),
            reloading: Cell::new(false),
            textures: RefCell::new(HashMap::new()),
            filter_dialog: RefCell::new(Weak::new()),
        });
        let this = Window::with_actions(this);
        this.build_layout();
        this.connect_signals();
        this.install_shortcuts();
        this.reload_sidebar();
        this.reload_articles(false);
        this.show_article();
        this
    }

    fn with_actions(this: Rc<Window>) -> Rc<Window> {
        let mut inner = Rc::try_unwrap(this).ok().expect("janela recém-criada");
        let weak_cell: Rc<RefCell<Weak<Window>>> = Rc::new(RefCell::new(Weak::new()));
        let simple: [WindowAction; 8] = [
            ("mark-all-read", |w| w.mark_all_read()),
            ("open-browser", |w| {
                let link = w.state.borrow().current.as_ref().map(|a| a.link.clone());
                if let Some(link) = link {
                    w.open_external(&link);
                }
            }),
            ("toggle-star", |w| w.toggle_star()),
            ("toggle-read", |w| w.toggle_read()),
            ("next", |w| w.move_selection(1)),
            ("previous", |w| w.move_selection(-1)),
            ("toggle-full", |w| {
                let next = if w.mode() == Mode::Full { Mode::Summary } else { Mode::Full };
                w.mode_group.set_active_name(Some(next.name()));
            }),
            ("search", |w| w.search_bar.set_search_mode(true)),
        ];
        for (name, handler) in simple {
            let action = gio::SimpleAction::new(name, None);
            let cell = weak_cell.clone();
            action.connect_activate(move |_, _| {
                if let Some(w) = cell.borrow().upgrade() {
                    handler(&w);
                }
            });
            inner.window.add_action(&action);
            if matches!(name, "open-browser" | "toggle-star" | "toggle-read") {
                inner.article_actions.push(action);
            }
        }
        let with_id: [FeedAction; 4] = [
            ("remove-feed", |w, id| w.remove_feed(id)),
            ("mark-feed-read", |w, id| {
                w.db.mark_all_read(Some(id));
                w.reload_sidebar();
                w.reload_articles(true);
            }),
            ("edit-feed", |w, id| w.edit_feed(id)),
            ("filter-feed", |w, id| w.filter_feed(id)),
        ];
        for (name, handler) in with_id {
            let action = gio::SimpleAction::new(name, Some(glib::VariantTy::INT64));
            let cell = weak_cell.clone();
            action.connect_activate(move |_, param| {
                let (Some(w), Some(id)) = (cell.borrow().upgrade(), param.and_then(|p| p.get::<i64>())) else { return };
                handler(&w, id);
            });
            inner.window.add_action(&action);
        }
        let this = Rc::new(inner);
        *weak_cell.borrow_mut() = Rc::downgrade(&this);
        this
    }

    fn build_layout(self: &Rc<Self>) {
        let sidebar_header = adw::HeaderBar::builder().show_end_title_buttons(false).build();
        sidebar_header.set_title_widget(Some(&adw::WindowTitle::new("readrss", "")));
        sidebar_header.pack_start(
            &gtk::Button::builder()
                .icon_name("list-add-symbolic")
                .tooltip_text(gettext("Add feed (Ctrl+N)"))
                .action_name("app.add-feed")
                .build(),
        );
        let menu = gio::Menu::new();
        let section = gio::Menu::new();
        section.append(Some(&gettext("Import OPML…")), Some("app.import-opml"));
        section.append(Some(&gettext("Export OPML…")), Some("app.export-opml"));
        menu.append_section(None, &section);
        let section = gio::Menu::new();
        section.append(Some(&gettext("Preferences")), Some("app.preferences"));
        section.append(Some(&gettext("Changelog")), Some("app.changelog"));
        section.append(Some(&gettext("About readrss")), Some("app.about"));
        menu.append_section(None, &section);
        let section = gio::Menu::new();
        section.append(Some(&gettext("Quit")), Some("app.quit"));
        menu.append_section(None, &section);
        sidebar_header.pack_end(
            &gtk::MenuButton::builder()
                .icon_name("open-menu-symbolic")
                .menu_model(&menu)
                .tooltip_text(gettext("Main menu"))
                .build(),
        );
        let sidebar_view = adw::ToolbarView::new();
        sidebar_view.add_top_bar(&sidebar_header);
        sidebar_view.set_content(Some(&scrolled(&self.sidebar_list)));
        sidebar_view.set_size_request(220, -1);

        let list_header =
            adw::HeaderBar::builder().show_start_title_buttons(false).show_end_title_buttons(false).build();
        list_header.set_title_widget(Some(&self.list_title));
        list_header.pack_start(
            &gtk::Button::builder()
                .child(&self.sync_stack)
                .action_name("app.sync")
                .tooltip_text(gettext("Sync now (Ctrl+R)"))
                .build(),
        );
        let search_toggle = gtk::ToggleButton::builder()
            .icon_name("system-search-symbolic")
            .tooltip_text(gettext("Search (Ctrl+F)"))
            .build();
        list_header.pack_end(&search_toggle);
        list_header.pack_end(
            &gtk::Button::builder()
                .icon_name("checkbox-checked-symbolic")
                .action_name("win.mark-all-read")
                .tooltip_text(gettext("Mark all as read"))
                .build(),
        );
        self.search_bar
            .bind_property("search-mode-enabled", &search_toggle, "active")
            .bidirectional()
            .sync_create()
            .build();
        self.list_stack.add_named(&scrolled(&self.article_list), Some("list"));
        self.list_stack.add_named(
            &adw::StatusPage::builder()
                .icon_name("checkbox-checked-symbolic")
                .title(gettext("Nothing here"))
                .description(gettext("No articles in this view."))
                .build(),
            Some("empty"),
        );
        let list_view = adw::ToolbarView::new();
        list_view.add_top_bar(&list_header);
        list_view.add_top_bar(&self.search_bar);
        list_view.set_content(Some(&self.list_stack));
        list_view.set_size_request(300, -1);

        for (mode, label, tooltip) in [
            (Mode::Summary, gettext("Summary"), gettext("Summary from the feed")),
            (Mode::Full, gettext("Full"), gettext("Extract the full article from the site (F)")),
            (Mode::Web, gettext("Website"), gettext("Open the original page in the app")),
        ] {
            self.mode_group.add(adw::Toggle::builder().name(mode.name()).label(label).tooltip(tooltip).build());
        }
        self.mode_group.set_active_name(Some(Mode::Summary.name()));
        let reader_header = adw::HeaderBar::builder().show_start_title_buttons(false).show_title(false).build();
        reader_header.pack_start(&self.mode_group);
        reader_header.pack_end(
            &gtk::Button::builder()
                .icon_name("web-browser-symbolic")
                .action_name("win.open-browser")
                .tooltip_text(gettext("Open in browser (O)"))
                .build(),
        );
        reader_header.pack_end(&self.star_button);
        reader_header.pack_end(
            &gtk::Button::builder()
                .icon_name("mail-unread-symbolic")
                .action_name("win.toggle-read")
                .tooltip_text(gettext("Toggle read/unread (M)"))
                .build(),
        );
        let overlay = gtk::Overlay::new();
        overlay.set_child(Some(&self.content_stack));
        overlay.add_overlay(&self.progress);
        self.reader_pane.add_top_bar(&reader_header);
        self.reader_pane.set_content(Some(&overlay));

        let inner = gtk::Paned::new(gtk::Orientation::Horizontal);
        inner.set_start_child(Some(&list_view));
        inner.set_end_child(Some(&self.reader_pane));
        inner.set_position(400);
        let outer = gtk::Paned::new(gtk::Orientation::Horizontal);
        outer.set_start_child(Some(&sidebar_view));
        outer.set_end_child(Some(&inner));
        outer.set_position(260);
        for paned in [&inner, &outer] {
            paned.set_shrink_start_child(false);
            paned.set_resize_start_child(false);
            paned.set_shrink_end_child(false);
        }
        self.toasts.set_child(Some(&outer));
        self.window.set_content(Some(&self.toasts));

        let (_, natural, _, _) = reader_header.measure(gtk::Orientation::Horizontal, -1);
        self.reader_pane.set_size_request(natural.max(400), -1);
    }

    fn connect_signals(self: &Rc<Self>) {
        let weak = Rc::downgrade(self);
        self.sidebar_list.connect_row_selected(move |_, row| {
            let (Some(w), Some(row)) = (weak.upgrade(), row) else { return };
            if w.reloading.get() {
                return;
            }
            let source = w.state.borrow().sidebar.iter().find(|r| &r.row == row).and_then(|r| r.source);
            if let Some(source) = source {
                w.state.borrow_mut().source = Some(source);
                w.reload_articles(false);
            }
        });
        let weak = Rc::downgrade(self);
        self.article_list.connect_row_selected(move |_, row| {
            let (Some(w), Some(row)) = (weak.upgrade(), row) else { return };
            if !w.reloading.get() {
                w.article_selected(row);
            }
        });
        let weak = Rc::downgrade(self);
        self.search_entry.connect_search_changed(move |_| {
            if let Some(w) = weak.upgrade() {
                w.reload_articles(false);
            }
        });
        let weak = Rc::downgrade(self);
        self.mode_group.connect_active_name_notify(move |group| {
            let Some(w) = weak.upgrade() else { return };
            w.state.borrow_mut().mode = Some(Mode::from_name(group.active_name().as_deref()));
            w.show_article();
        });
        let weak = Rc::downgrade(self);
        self.reader.connect_decide_policy(move |_, decision, kind| {
            let Some(w) = weak.upgrade() else { return false };
            let Some(action) =
                decision.downcast_ref::<webkit6::NavigationPolicyDecision>().and_then(|nav| nav.navigation_action())
            else {
                return false;
            };
            let external = kind == webkit6::PolicyDecisionType::NewWindowAction
                || (kind == webkit6::PolicyDecisionType::NavigationAction
                    && action.navigation_type() == webkit6::NavigationType::LinkClicked);
            if external {
                if let Some(uri) = action.request().and_then(|r| r.uri()) {
                    w.open_external(&uri);
                }
                decision.ignore();
            }
            external
        });
    }

    fn install_shortcuts(&self) {
        let controller = gtk::ShortcutController::new();
        for (keys, action) in [
            ("j", "win.next"),
            ("n", "win.next"),
            ("k", "win.previous"),
            ("p", "win.previous"),
            ("s", "win.toggle-star"),
            ("m", "win.toggle-read"),
            ("o", "win.open-browser"),
            ("f", "win.toggle-full"),
            ("<Control>f", "win.search"),
        ] {
            controller.add_shortcut(gtk::Shortcut::new(
                gtk::ShortcutTrigger::parse_string(keys),
                Some(gtk::NamedAction::new(action)),
            ));
        }
        self.window.add_controller(controller);
    }

    pub fn source(&self) -> Source {
        self.state.borrow().source.unwrap_or(Source::Unread)
    }

    pub fn mode(&self) -> Mode {
        self.state.borrow().mode.unwrap_or(Mode::Summary)
    }

    pub fn current(&self) -> Option<Article> {
        self.state.borrow().current.clone()
    }

    pub fn articles(&self) -> Vec<Article> {
        self.state.borrow().rows.iter().map(|r| r.article.clone()).collect()
    }

    pub fn sidebar_rows(&self) -> Vec<(Option<Source>, gtk::ListBoxRow, Vec<String>)> {
        self.state.borrow().sidebar.iter().map(|r| (r.source, r.row.clone(), r.icons.clone())).collect()
    }

    pub fn select_source(&self, source: Source) {
        let row = self.state.borrow().sidebar.iter().find(|r| r.source == Some(source)).map(|r| r.row.clone());
        if let Some(row) = row {
            self.sidebar_list.select_row(Some(&row));
        }
    }

    pub fn select_article(&self, article_id: i64) {
        let row = self.state.borrow().rows.iter().find(|r| r.article.id == article_id).map(|r| r.row.clone());
        if let Some(row) = row {
            self.article_list.select_row(Some(&row));
        }
    }

    pub fn reload_sidebar(self: &Rc<Self>) {
        let feeds = self.db.list_feeds();
        let (unread, starred) = self.db.counts();
        self.reloading.set(true);
        self.sidebar_list.remove_all();
        let mut rows = Vec::new();
        let icon = |name: &str| gtk::Image::from_icon_name(name).upcast::<gtk::Widget>();
        rows.push(self.source_row(
            Some(Source::All),
            &gettext("All articles"),
            icon("view-list-symbolic"),
            0,
            None,
            false,
        ));
        rows.push(self.source_row(
            Some(Source::Unread),
            &gettext("Unread"),
            icon("mail-unread-symbolic"),
            unread,
            None,
            false,
        ));
        rows.push(self.source_row(
            Some(Source::Starred),
            &gettext("Starred"),
            icon("starred-symbolic"),
            starred,
            None,
            false,
        ));
        let header = gtk::ListBoxRow::builder().selectable(false).activatable(false).build();
        header.set_child(Some(
            &gtk::Label::builder()
                .label(gettext("Feeds"))
                .xalign(0.0)
                .css_classes(["heading", "dim-label"])
                .margin_top(12)
                .margin_start(6)
                .build(),
        ));
        rows.push(SidebarRow { source: None, row: header, icons: vec![] });
        for feed in &feeds {
            let row = self.source_row(
                Some(Source::Feed(feed.id)),
                feed.display_title(),
                self.feed_icon(Some(feed), 20),
                feed.unread,
                feed.last_error.as_deref(),
                feed.has_filter,
            );
            let click = gtk::GestureClick::builder().button(gdk::BUTTON_SECONDARY).build();
            let (target, feed_id) = (row.row.clone(), feed.id);
            click.connect_pressed(move |_, _, _, _| show_feed_menu(&target, feed_id));
            row.row.add_controller(click);
            rows.push(row);
        }
        for row in &rows {
            self.sidebar_list.append(&row.row);
        }
        let mut current = self.source();
        if !rows.iter().any(|r| r.source == Some(current)) {
            current = Source::Unread;
        }
        let selected = rows.iter().find(|r| r.source == Some(current)).map(|r| r.row.clone());
        {
            let mut state = self.state.borrow_mut();
            state.source = Some(current);
            state.feeds = feeds;
            state.sidebar = rows;
        }
        self.sidebar_list.select_row(selected.as_ref());
        self.reloading.set(false);
    }

    fn source_row(
        &self,
        source: Option<Source>,
        title: &str,
        icon: gtk::Widget,
        count: i64,
        error: Option<&str>,
        filtered: bool,
    ) -> SidebarRow {
        let row_box = gtk::Box::new(gtk::Orientation::Horizontal, 10);
        row_box.append(&icon);
        row_box.append(
            &gtk::Label::builder().label(title).xalign(0.0).hexpand(true).ellipsize(pango::EllipsizeMode::End).build(),
        );
        let mut icons = Vec::new();
        if filtered {
            row_box.append(
                &gtk::Image::builder()
                    .icon_name(FILTER_ICON)
                    .tooltip_text(gettext("Filter active: some articles are hidden"))
                    .css_classes(["dim-label"])
                    .build(),
            );
            icons.push(FILTER_ICON.to_owned());
        }
        if let Some(error) = error {
            row_box.append(
                &gtk::Image::builder()
                    .icon_name("dialog-warning-symbolic")
                    .tooltip_text(error)
                    .css_classes(["warning"])
                    .build(),
            );
            icons.push("dialog-warning-symbolic".to_owned());
        }
        if count > 0 {
            row_box.append(
                &gtk::Label::builder()
                    .label(count.to_string())
                    .css_classes(["dim-label", "caption", "numeric"])
                    .build(),
            );
        }
        SidebarRow { source, row: gtk::ListBoxRow::builder().child(&row_box).build(), icons }
    }

    /// Favicon cacheado; sem ícone válido, um avatar com as iniciais.
    fn feed_icon(&self, feed: Option<&Feed>, size: i32) -> gtk::Widget {
        if let Some(texture) = feed.and_then(|f| f.icon_path.as_deref()).and_then(|p| self.texture(p)) {
            return gtk::Image::builder().paintable(&texture).pixel_size(size).build().upcast();
        }
        adw::Avatar::new(size, Some(feed.map(Feed::display_title).unwrap_or("?")), true).upcast()
    }

    fn texture(&self, path: &str) -> Option<gdk::Texture> {
        let modified = std::fs::metadata(path).and_then(|m| m.modified()).ok()?;
        if let Some((cached_at, texture)) = self.textures.borrow().get(path)
            && *cached_at == modified
        {
            return Some(texture.clone());
        }
        let texture = gdk::Texture::from_filename(path).ok()?;
        self.textures.borrow_mut().insert(path.to_owned(), (modified, texture.clone()));
        Some(texture)
    }

    pub fn reload_articles(self: &Rc<Self>, keep_selection: bool) {
        let source = self.source();
        let feed_title = match source {
            Source::Feed(id) => {
                self.state.borrow().feeds.iter().find(|f| f.id == id).map(|f| f.display_title().to_owned())
            }
            _ => None,
        };
        self.list_title.set_title(&match source {
            Source::All => gettext("All articles"),
            Source::Unread => gettext("Unread"),
            Source::Starred => gettext("Starred"),
            Source::Feed(_) => feed_title.unwrap_or_default(),
        });
        let search = self.search_entry.text().trim().to_owned();
        let articles = self.db.list_articles(&ArticleQuery {
            feed_id: if let Source::Feed(id) = source { Some(id) } else { None },
            unread_only: source == Source::Unread,
            starred_only: source == Source::Starred,
            search: &search,
        });
        self.list_title.set_subtitle(&if articles.is_empty() {
            String::new()
        } else {
            ngettext_n("{n} article", "{n} articles", articles.len())
        });

        let selected_id = if keep_selection { self.state.borrow().current.as_ref().map(|a| a.id) } else { None };
        self.reloading.set(true);
        self.article_list.remove_all();
        let feeds: HashMap<i64, Feed> = self.state.borrow().feeds.iter().map(|f| (f.id, f.clone())).collect();
        let mut rows = Vec::with_capacity(articles.len());
        let mut selected = None;
        for article in articles {
            let row = self.article_row(article, &feeds);
            if Some(row.article.id) == selected_id {
                selected = Some(row.row.clone());
            }
            self.article_list.append(&row.row);
            rows.push(row);
        }
        self.list_stack.set_visible_child_name(if rows.is_empty() { "empty" } else { "list" });
        self.state.borrow_mut().rows = rows;
        if let Some(row) = selected {
            self.article_list.select_row(Some(&row));
        }
        self.reloading.set(false);
    }

    fn article_row(&self, article: Article, feeds: &HashMap<i64, Feed>) -> ArticleRow {
        let top = gtk::Box::new(gtk::Orientation::Horizontal, 6);
        top.append(&self.feed_icon(feeds.get(&article.feed_id), 16));
        top.append(
            &gtk::Label::builder()
                .label(&article.feed_title)
                .xalign(0.0)
                .hexpand(true)
                .ellipsize(pango::EllipsizeMode::End)
                .css_classes(["caption", "dim-label"])
                .build(),
        );
        let star = gtk::Image::builder().icon_name("starred-symbolic").css_classes(["accent"]).build();
        top.append(&star);
        top.append(
            &gtk::Label::builder()
                .label(relative_time(article.published))
                .css_classes(["caption", "dim-label"])
                .build(),
        );
        let title = gtk::Label::builder()
            .label(&article.title)
            .xalign(0.0)
            .wrap(true)
            .wrap_mode(pango::WrapMode::WordChar)
            .lines(3)
            .ellipsize(pango::EllipsizeMode::End)
            .build();
        let column = gtk::Box::builder()
            .orientation(gtk::Orientation::Vertical)
            .spacing(4)
            .margin_top(10)
            .margin_bottom(10)
            .margin_start(12)
            .margin_end(12)
            .build();
        column.append(&top);
        column.append(&title);
        let row = ArticleRow { row: gtk::ListBoxRow::builder().child(&column).build(), article, title, star };
        style_row(&row);
        row
    }

    fn article_selected(self: &Rc<Self>, row: &gtk::ListBoxRow) {
        let id = self.state.borrow().rows.iter().find(|r| &r.row == row).map(|r| r.article.id);
        let Some(id) = id else { return };
        if self.state.borrow().current.as_ref().map(|a| a.id) == Some(id) {
            return;
        }
        let Some(mut article) = self.db.get_article(id) else { return };
        let was_unread = !article.is_read;
        if was_unread {
            self.db.set_read(id, true);
            article.is_read = true;
        }
        {
            let mut state = self.state.borrow_mut();
            if let Some(r) = state.rows.iter_mut().find(|r| r.article.id == id) {
                r.article = article.clone();
                style_row(r);
            }
            state.current = Some(article);
        }
        if was_unread {
            self.reload_sidebar();
        }
        if self.mode() != Mode::Summary {
            self.mode_group.set_active_name(Some(Mode::Summary.name()));
        } else {
            self.show_article();
        }
    }

    fn move_selection(&self, step: i32) {
        let index = self.article_list.selected_row().map(|r| r.index() + step).unwrap_or(0).max(0);
        if let Some(target) = self.article_list.row_at_index(index) {
            self.article_list.select_row(Some(&target));
            target.grab_focus();
        }
    }

    fn toggle_star(self: &Rc<Self>) {
        let Some((id, starred)) = self.state.borrow_mut().current.as_mut().map(|a| {
            a.is_starred = !a.is_starred;
            (a.id, a.is_starred)
        }) else {
            return;
        };
        self.db.set_starred(id, starred);
        self.sync_current_row();
        self.reload_sidebar();
    }

    fn toggle_read(self: &Rc<Self>) {
        let Some((id, read)) = self.state.borrow_mut().current.as_mut().map(|a| {
            a.is_read = !a.is_read;
            (a.id, a.is_read)
        }) else {
            return;
        };
        self.db.set_read(id, read);
        self.sync_current_row();
        self.reload_sidebar();
    }

    fn sync_current_row(&self) {
        {
            let mut state = self.state.borrow_mut();
            if let Some(current) = state.current.clone()
                && let Some(row) = state.rows.iter_mut().find(|r| r.article.id == current.id)
            {
                row.article = current;
                style_row(row);
            }
        }
        self.update_reader_header();
    }

    fn mark_all_read(self: &Rc<Self>) {
        let feed = if let Source::Feed(id) = self.source() { Some(id) } else { None };
        self.db.mark_all_read(feed);
        self.reload_sidebar();
        self.reload_articles(true);
    }

    fn edit_feed(self: &Rc<Self>, feed_id: i64) {
        let (Some(feed), Some(app)) = (self.db.get_feed(feed_id), self.app.upgrade()) else { return };
        let original = feed.clone();
        feed_dialogs::show_edit_feed(&self.window, &feed, move |url, name| app.edit_feed(&original, &url, &name));
    }

    fn filter_feed(self: &Rc<Self>, feed_id: i64) {
        let Some(feed) = self.db.get_feed(feed_id) else { return };
        let weak = Rc::downgrade(self);
        let dialog = feed_dialogs::show_filter_feed(&self.window, self.db.clone(), &feed, move |hidden, active| {
            let Some(w) = weak.upgrade() else { return };
            w.reload_sidebar();
            w.reload_articles(true);
            w.toast(&if active {
                ngettext_n("Filter saved: {n} article hidden", "Filter saved: {n} articles hidden", hidden)
            } else {
                gettext("Filter removed")
            });
        });
        *self.filter_dialog.borrow_mut() = Rc::downgrade(&dialog);
    }

    fn remove_feed(self: &Rc<Self>, feed_id: i64) {
        let Some(feed) = self.db.get_feed(feed_id) else { return };
        let title = feed.display_title().to_owned();
        let dialog = adw::AlertDialog::builder()
            .heading(gettext("Remove feed?"))
            .body(fill(&gettext("“{title}” and all its articles will be deleted."), &[("title", &title)]))
            .build();
        dialog.add_response("cancel", &gettext("Cancel"));
        dialog.add_response("remove", &gettext("Remove"));
        dialog.set_response_appearance("remove", adw::ResponseAppearance::Destructive);
        let weak = Rc::downgrade(self);
        dialog.connect_response(None, move |_, response| {
            let Some(w) = weak.upgrade() else { return };
            if response != "remove" {
                return;
            }
            w.db.remove_feed(feed_id);
            let removed_current = w.state.borrow().current.as_ref().is_some_and(|a| a.feed_id == feed_id);
            if removed_current {
                w.state.borrow_mut().current = None;
                w.show_article();
            }
            w.reload_sidebar();
            w.reload_articles(false);
            w.toast(&fill(&gettext("“{title}” removed"), &[("title", &title)]));
        });
        dialog.present(Some(&self.window));
    }

    fn update_reader_header(&self) {
        let (has_article, starred) = {
            let state = self.state.borrow();
            (state.current.is_some(), state.current.as_ref().is_some_and(|a| a.is_starred))
        };
        for action in &self.article_actions {
            action.set_enabled(has_article);
        }
        self.mode_group.set_sensitive(has_article);
        self.star_button.set_icon_name(if starred { "starred-symbolic" } else { "non-starred-symbolic" });
        self.star_button.set_tooltip_text(Some(&if starred { gettext("Unstar (S)") } else { gettext("Star (S)") }));
    }

    fn show_article(self: &Rc<Self>) {
        self.update_reader_header();
        let (article, mode) = (self.current(), self.mode());
        if let (Some(article), Mode::Web) = (&article, mode)
            && !article.link.is_empty()
        {
            self.open_site(&article.link);
            return;
        }
        self.close_site();
        let page = match (&article, mode) {
            (None, _) => html::render_message(&gettext("Select an article to read")),
            (Some(_), Mode::Web) => html::render_message(&gettext("This article has no link")),
            (Some(a), Mode::Summary) => html::render_article(a, false),
            (Some(a), Mode::Full) if a.full_content.is_some() => html::render_article(a, true),
            (Some(a), Mode::Full) if a.link.is_empty() => html::render_message(&gettext("This article has no link")),
            (Some(a), Mode::Full) => {
                self.load_full_content(a.clone());
                html::render_message(&gettext("Loading full article…"))
            }
        };
        self.reader.load_html(&page, None);
    }

    /// Página original num WebView com contexto próprio, descartado ao sair: os processos do
    /// WebKit pertencem ao contexto, e os cookies ficam na sessão de rede padrão.
    fn open_site(self: &Rc<Self>, uri: &str) {
        self.close_site();
        let view = webkit6::WebView::builder().web_context(&webkit6::WebContext::new()).build();
        view.set_vexpand(true);
        view.set_hexpand(true);
        if let Some(settings) = WebViewExt::settings(&view) {
            settings.set_enable_developer_extras(false);
        }
        let weak = Rc::downgrade(self);
        view.connect_decide_policy(move |_, decision, kind| {
            if kind != webkit6::PolicyDecisionType::NewWindowAction {
                return false;
            }
            let uri = decision
                .downcast_ref::<webkit6::NavigationPolicyDecision>()
                .and_then(|nav| nav.navigation_action())
                .and_then(|action| action.request())
                .and_then(|request| request.uri());
            if let (Some(w), Some(uri)) = (weak.upgrade(), uri) {
                w.open_external(&uri);
            }
            decision.ignore();
            true
        });
        let progress = self.progress.clone();
        view.connect_estimated_load_progress_notify(move |view| {
            let value = view.estimated_load_progress();
            progress.set_fraction(value);
            progress.set_visible(value < 1.0);
        });
        self.content_stack.add_named(&view, Some("site"));
        self.content_stack.set_visible_child(&view);
        view.load_uri(uri);
        *self.site_view.borrow_mut() = Some(view);
    }

    fn close_site(&self) {
        self.progress.set_visible(false);
        let Some(view) = self.site_view.borrow_mut().take() else { return };
        self.content_stack.set_visible_child(&self.reader);
        self.content_stack.remove(&view);
        view.try_close();
    }

    fn load_full_content(self: &Rc<Self>, article: Article) {
        let weak = Rc::downgrade(self);
        glib::spawn_future_local(async move {
            let link = article.link.clone();
            let result = gio::spawn_blocking(move || net::extract_article(&net::Client::new(), &link))
                .await
                .unwrap_or(Err(net::NetError::ExtractFailed));
            let Some(w) = weak.upgrade() else { return };
            let wanted = w.mode() == Mode::Full && w.current().is_some_and(|a| a.id == article.id);
            match result {
                Ok(content) => {
                    w.db.set_full_content(article.id, &content);
                    if wanted {
                        if let Some(current) = w.state.borrow_mut().current.as_mut() {
                            current.full_content = Some(content);
                        }
                        w.show_article();
                    }
                }
                Err(error) if wanted => {
                    let text =
                        fill(&gettext("Could not extract the article: {error}"), &[("error", &error.describe())]);
                    w.reader.load_html(&html::render_message(&text), None);
                    let toast = adw::Toast::builder()
                        .title(gettext("Failed to extract the article"))
                        .button_label(gettext("View website"))
                        .build();
                    let weak = Rc::downgrade(&w);
                    toast.connect_button_clicked(move |_| {
                        if let Some(w) = weak.upgrade() {
                            w.mode_group.set_active_name(Some(Mode::Web.name()));
                        }
                    });
                    w.toasts.add_toast(toast);
                }
                Err(_) => {}
            }
        });
    }

    pub fn open_external(&self, uri: &str) {
        gtk::UriLauncher::new(uri).launch(Some(&self.window), gio::Cancellable::NONE, |_| {});
    }

    pub fn set_syncing(&self, busy: bool) {
        self.sync_stack.set_visible_child_name(if busy { "busy" } else { "idle" });
    }

    pub fn refresh_after_sync(self: &Rc<Self>) {
        self.reload_sidebar();
        self.reload_articles(true);
    }

    pub fn toast(&self, text: &str) {
        self.toasts.add_toast(adw::Toast::new(text));
    }

    pub fn present(&self) {
        self.window.present();
    }
}

fn style_row(row: &ArticleRow) {
    if row.article.is_read {
        row.title.remove_css_class("heading");
        row.title.add_css_class("dim-label");
    } else {
        row.title.remove_css_class("dim-label");
        row.title.add_css_class("heading");
    }
    row.star.set_visible(row.article.is_starred);
}

fn scrolled(child: &impl IsA<gtk::Widget>) -> gtk::ScrolledWindow {
    gtk::ScrolledWindow::builder().child(child).hscrollbar_policy(gtk::PolicyType::Never).vexpand(true).build()
}

fn show_feed_menu(row: &gtk::ListBoxRow, feed_id: i64) {
    let menu = gio::Menu::new();
    let section = gio::Menu::new();
    section.append(Some(&gettext("Edit…")), Some(&format!("win.edit-feed(int64 {feed_id})")));
    section.append(Some(&gettext("Filter…")), Some(&format!("win.filter-feed(int64 {feed_id})")));
    section.append(Some(&gettext("Mark all as read")), Some(&format!("win.mark-feed-read(int64 {feed_id})")));
    menu.append_section(None, &section);
    let section = gio::Menu::new();
    section.append(Some(&gettext("Remove feed…")), Some(&format!("win.remove-feed(int64 {feed_id})")));
    menu.append_section(None, &section);
    let popover = gtk::PopoverMenu::builder().menu_model(&menu).has_arrow(false).halign(gtk::Align::Start).build();
    popover.set_parent(row);
    popover.connect_closed(|popover| {
        let popover = popover.clone();
        glib::idle_add_local_once(move || popover.unparent());
    });
    popover.popup();
}
