//! Testes da interface: sobem o app real, com servidor HTTP falso e dados temporários.
//!
//! Abrem janelas na tela, então exigem display gráfico. Rode com `cargo test --test gui`.
//! Todos os cenários rodam em sequência na thread principal (exigência do GTK).

mod common;

use std::cell::{Cell, RefCell};
use std::process::{Command, Stdio};
use std::rc::Rc;
use std::time::{Duration, Instant};

use adw::prelude::*;
use common::{FakeWeb, Response, TempDir, item, rss, simple_item};
use gtk::glib;
use readrss::app::{App, Config};
use readrss::constants::{APP_ID, FILTER_ICON};
use readrss::db::{ArticleQuery, Db};
use readrss::metrics::web_processes;
use readrss::window::{Mode, Source};
use webkit6::prelude::*;

type Step = Box<dyn FnMut(&Rc<App>) -> bool>;

fn step(f: impl FnMut(&Rc<App>) + 'static) -> Step {
    let mut f = f;
    Box::new(move |app| {
        f(app);
        true
    })
}

fn wait(f: impl FnMut(&Rc<App>) -> bool + 'static) -> Step {
    Box::new(f)
}

fn pause(ticks: u32) -> Step {
    let left = Cell::new(ticks);
    Box::new(move |_| {
        left.set(left.get().saturating_sub(1));
        left.get() == 0
    })
}

fn unique_id() -> String {
    format!("io.github.readrss.Test{}{}", std::process::id(), glib::monotonic_time())
}

fn config(dir: &TempDir) -> Config {
    Config { data_dir: dir.path().join("data"), cache_dir: dir.path().join("cache"), app_id: unique_id() }
}

/// Executa os passos no laço do app; um passo que devolve `false` é repetido no próximo tique.
fn run_steps(app: &Rc<App>, steps: Vec<Step>, args: &[&str]) {
    let steps = Rc::new(RefCell::new(steps.into_iter().collect::<std::collections::VecDeque<_>>()));
    let deadline = Instant::now() + Duration::from_secs(90);
    let runner = app.clone();
    glib::timeout_add_local(Duration::from_millis(250), move || {
        assert!(Instant::now() < deadline, "cenário excedeu o tempo limite");
        let mut queue = steps.borrow_mut();
        if let Some(mut current) = queue.pop_front() {
            if !current(&runner) {
                queue.push_front(current);
            }
        }
        if queue.is_empty() {
            runner.application.quit();
            return glib::ControlFlow::Break;
        }
        glib::ControlFlow::Continue
    });
    let args: Vec<String> = args.iter().map(|a| a.to_string()).collect();
    app.run(&args);
}

fn win(app: &Rc<App>) -> Rc<readrss::window::Window> {
    app.window().expect("janela aberta")
}

fn act(app: &Rc<App>, name: &str, param: Option<&glib::Variant>) {
    WidgetExt::activate_action(&win(app).window, name, param).unwrap();
}

fn titles(app: &Rc<App>) -> Vec<String> {
    let mut titles: Vec<String> = win(app).articles().into_iter().map(|a| a.title).collect();
    titles.sort();
    titles
}

fn synced(app: &Rc<App>) -> bool {
    !app.sync.busy() && app.window().is_some_and(|w| !w.articles().is_empty() || app.db.counts() == (0, 0))
}

fn main_flow() {
    let web = FakeWeb::start();
    let story = web.url("/story-1");
    let feed_body = rss(&(simple_item("1", "First story", &story) + &simple_item("2", "Second story", "https://site.test/2")),
                        "Test Site", &web.url("/"), "");
    web.add("/feed", Response::new(200, "application/rss+xml", feed_body));
    web.add("/story-1", Response::new(200, "text/html", format!(
        "<html><body><article><h1>Story</h1><p>{}</p></article></body></html>",
        "Full story paragraph with plenty of words to be extracted by the reader. ".repeat(8))));
    let dir = TempDir::new();
    let app = App::new(config(&dir));
    app.db.add_feed(&web.url("/feed"), "");
    let first_id = Rc::new(Cell::new(0));
    let old_window: Rc<RefCell<Option<adw::ApplicationWindow>>> = Rc::default();

    let id = first_id.clone();
    let old = old_window.clone();
    let id2 = first_id.clone();
    let id3 = first_id.clone();
    run_steps(&app, vec![
        wait(synced),
        step(|app| {
            let theme = gtk::IconTheme::for_display(&WidgetExt::display(&win(app).window));
            assert!(theme.has_icon(APP_ID) && theme.has_icon(&format!("{APP_ID}-symbolic")), "ícone embutido");
            win(app).select_source(Source::All);
        }),
        step(move |app| {
            assert_eq!(titles(app), vec!["First story", "Second story"]);
            let first = win(app).articles().into_iter().find(|a| a.title == "First story").unwrap();
            id.set(first.id);
            win(app).select_article(first.id);
        }),
        step(move |app| {
            let current = win(app).current().unwrap();
            assert!(current.title == "First story" && current.is_read);
            assert_eq!(app.db.counts().0, 1);
            act(app, "win.toggle-star", None);
            assert!(app.db.get_article(id2.get()).unwrap().is_starred);
            win(app).mode_group.set_active_name(Some(Mode::Full.name()));
        }),
        wait(move |app| app.db.get_article(id3.get()).and_then(|a| a.full_content).is_some_and(|c| c.contains("Full story paragraph"))),
        step(|app| {
            let w = win(app);
            w.mode_group.set_active_name(Some(Mode::Web.name()));
            let site = w.site_view.borrow().clone().expect("WebView do modo Site");
            assert!(WebViewExt::settings(&site).unwrap().enables_javascript());
            assert!(site.web_context() != w.reader.web_context(), "contexto próprio e descartável");
            assert!(!WebViewExt::settings(&w.reader).unwrap().enables_javascript());
        }),
        step(|app| {
            win(app).mode_group.set_active_name(Some(Mode::Summary.name()));
            assert!(win(app).site_view.borrow().is_none());
        }),
        step(move |app| {
            *old.borrow_mut() = Some(win(app).window.clone());
            let dialog = app.show_preferences().unwrap();
            app.change_language("es", &dialog);
        }),
        pause(2),
        step(move |app| {
            assert!(Some(win(app).window.clone()) != *old_window.borrow(), "janela recriada");
            assert_eq!(win(app).list_title.title(), "Todos los artículos");
            assert_eq!(app.db.get_setting("language"), "es");
            if let Some(dialog) = win(app).window.visible_dialog() {
                dialog.close();
            }
        }),
        step(|app| {
            act(app, "win.mark-all-read", None);
            assert_eq!(app.db.counts().0, 0);
        }),
        step(|app| {
            let w = win(app);
            let inner = w.reader_pane.parent().and_then(|p| p.downcast::<gtk::Paned>().ok()).unwrap();
            let outer = inner.parent().and_then(|p| p.downcast::<gtk::Paned>().ok()).unwrap();
            for paned in [inner, outer] {
                let end_min = paned.end_child().unwrap().measure(gtk::Orientation::Horizontal, -1).0;
                assert!(end_min >= 400);
                assert!(paned.max_position() <= paned.width() - end_min, "leitor não pode ser espremido");
            }
            app.application.activate_action("about", None);
        }),
        step(|app| {
            let dialog = win(app).window.visible_dialog().unwrap();
            let about = dialog.downcast::<adw::AboutDialog>().unwrap();
            assert_eq!(about.version(), env!("CARGO_PKG_VERSION"));
            assert_eq!(about.release_notes_version(), env!("CARGO_PKG_VERSION"));
            about.close();
        }),
        step(|app| app.application.activate_action("changelog", None)),
        step(|app| {
            let dialog = win(app).window.visible_dialog().unwrap();
            assert_eq!(dialog.title(), "Registro de cambios");
            dialog.close();
        }),
    ], &["readrss"]);
    readrss::i18n::set_language("system");
}

fn filter_and_edit() {
    let web = FakeWeb::start();
    web.add("/feed", Response::new(200, "application/rss+xml",
        rss(&(simple_item("1", "Paredão do BBB", "https://s.test/1") + &simple_item("2", "Ubuntu lançado", "https://s.test/2")),
            "Test Site", &web.url("/"), "")));
    web.add("/new-feed", Response::new(200, "application/rss+xml",
        rss(&simple_item("3", "Outro", "https://s.test/3"), "Test Site", &web.url("/"), "")));
    let dir = TempDir::new();
    let app = App::new(config(&dir));
    let feed_id = app.db.add_feed(&web.url("/feed"), "").unwrap();
    let new_url = web.url("/new-feed");
    let paths = Rc::new(move || web.paths());

    let p = paths.clone();
    run_steps(&app, vec![
        wait(synced),
        step(|app| win(app).select_source(Source::All)),
        step(move |app| {
            assert_eq!(titles(app), vec!["Paredão do BBB", "Ubuntu lançado"]);
            act(app, "win.filter-feed", Some(&feed_id.to_variant()));
        }),
        step(|app| {
            let dialog = win(app).filter_dialog.borrow().upgrade().expect("diálogo de filtro");
            dialog.mode_row.set_selected(1);
            dialog.text_view.buffer().set_text("bbb");
        }),
        pause(2),
        step(|app| {
            let dialog = win(app).filter_dialog.borrow().upgrade().unwrap();
            let preview = dialog.preview_label.label();
            assert!(preview.contains('1') && preview.contains('2'), "{preview}");
            dialog.regex_row.set_active(true);
            dialog.text_view.buffer().set_text("(bbb");
        }),
        pause(2),
        step(|app| {
            let dialog = win(app).filter_dialog.borrow().upgrade().unwrap();
            assert!(dialog.error_label.is_visible() && !dialog.save_button.is_sensitive());
            dialog.text_view.buffer().set_text("b{3}");
        }),
        pause(2),
        step(|app| {
            let dialog = win(app).filter_dialog.borrow().upgrade().unwrap();
            assert!(dialog.save_button.is_sensitive());
            dialog.save_button.emit_clicked();
        }),
        step(move |app| {
            assert_eq!(titles(app), vec!["Ubuntu lançado"]);
            assert_eq!(app.db.counts().0, 1);
            let icons = win(app).sidebar_rows().into_iter().find(|(s, _, _)| *s == Some(Source::Feed(feed_id))).unwrap().2;
            assert!(icons.contains(&FILTER_ICON.to_owned()));
            act(app, "win.edit-feed", Some(&feed_id.to_variant()));
        }),
        step(move |app| {
            let dialog = win(app).window.visible_dialog().unwrap();
            assert!(dialog.is::<adw::AlertDialog>());
            dialog.close();
            app.edit_feed(&app.db.get_feed(feed_id).unwrap(), &new_url, "Meu Feed");
        }),
        wait(move |app| app.db.get_feed(feed_id).is_some_and(|f| f.url.ends_with("/new-feed")) && !app.sync.busy()),
        pause(2),
        step(move |app| {
            assert_eq!(app.db.get_feed(feed_id).unwrap().display_title(), "Meu Feed");
            assert!(p().contains(&"/new-feed".to_owned()));
            assert!(titles(app).contains(&"Outro".to_owned()));
        }),
    ], &["readrss"]);
}

fn webkit_processes_do_not_accumulate() {
    let web = FakeWeb::start();
    let items: String = (0..10)
        .map(|i| item(&i.to_string(), &format!("Artigo {i}"), &format!("https://site{i}.example{i}.test/a/{i}"),
                      r#"<p>texto</p><img src="img.png">"#, "", "Mon, 19 Oct 2026 10:00:00 GMT"))
        .collect();
    web.add("/feed", Response::new(200, "application/rss+xml", rss(&items, "Many", &web.url("/"), "")));
    let dir = TempDir::new();
    let app = App::new(config(&dir));
    app.db.add_feed(&web.url("/feed"), "");
    let me = std::process::id();
    let baseline = Rc::new(Cell::new(0));
    let index = Rc::new(Cell::new(0usize));
    let site_mode = Rc::new(Cell::new(false));

    let mut steps: Vec<Step> = vec![wait(synced)];
    let b = baseline.clone();
    steps.push(step(move |app| {
        b.set(web_processes(me));
        win(app).select_source(Source::All);
    }));
    for round in 0..16 {
        let (i, web_mode) = (index.clone(), site_mode.clone());
        if round == 10 {
            let b = baseline.clone();
            steps.push(pause(6));
            steps.push(step(move |_| assert!(web_processes(me) <= b.get() + 2, "leitor: {} > {}", web_processes(me), b.get() + 2)));
            let m = web_mode.clone();
            steps.push(step(move |_| m.set(true)));
        }
        steps.push(Box::new(move |app| {
            let articles = win(app).articles();
            win(app).select_article(articles[i.get() % articles.len()].id);
            if web_mode.get() {
                win(app).mode_group.set_active_name(Some(Mode::Web.name()));
            }
            i.set(i.get() + 1);
            true
        }));
        steps.push(pause(2));
    }
    steps.push(step(|app| win(app).mode_group.set_active_name(Some(Mode::Summary.name()))));
    steps.push(pause(12));
    let b = baseline.clone();
    steps.push(step(move |app| {
        assert!(win(app).site_view.borrow().is_none());
        assert!(web_processes(me) <= b.get() + 2, "após modo Site: {} > {}", web_processes(me), b.get() + 2);
    }));
    run_steps(&app, steps, &["readrss"]);
}

fn background_mode() {
    let web = FakeWeb::start();
    web.add("/feed", Response::new(200, "application/rss+xml", rss(&simple_item("1", "First story", "https://s.test/1"), "T", &web.url("/"), "")));
    let dir = TempDir::new();
    let app = App::new(config(&dir));
    app.db.add_feed(&web.url("/feed"), "");
    run_steps(&app, vec![
        wait(|app| !app.sync.busy() && app.db.counts().0 == 1),
        step(|app| {
            assert!(app.window().is_none() && app.application.windows().is_empty(), "serviço não abre janela");
            app.application.activate();
        }),
        pause(2),
        step(|app| {
            assert!(win(app).window.is_visible());
            assert_eq!(app.db.list_articles(&ArticleQuery::default())[0].title, "First story");
        }),
    ], &["readrss", "--background"]);
}

fn name_owned(name: &str) -> bool {
    Command::new("gdbus")
        .args(["call", "--session", "--dest", "org.freedesktop.DBus", "--object-path", "/org/freedesktop/DBus",
               "--method", "org.freedesktop.DBus.NameHasOwner", name])
        .output()
        .is_ok_and(|o| String::from_utf8_lossy(&o.stdout).contains("true"))
}

fn spawn_binary(dir: &TempDir, app_id: &str, args: &[&str]) -> std::process::Child {
    Command::new(env!("CARGO_BIN_EXE_readrss"))
        .args(args)
        .env("READRSS_DATA_DIR", dir.path().join("data"))
        .env("READRSS_CACHE_DIR", dir.path().join("cache"))
        .env("READRSS_APP_ID", app_id)
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap()
}

fn wait_for(condition: impl Fn() -> bool, seconds: u64) -> bool {
    let deadline = Instant::now() + Duration::from_secs(seconds);
    while Instant::now() < deadline {
        if condition() {
            return true;
        }
        std::thread::sleep(Duration::from_millis(100));
    }
    false
}

fn exit_within(child: &mut std::process::Child, seconds: u64) -> Option<std::process::ExitStatus> {
    let deadline = Instant::now() + Duration::from_secs(seconds);
    while Instant::now() < deadline {
        if let Ok(Some(status)) = child.try_wait() {
            return Some(status);
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    let _ = child.kill();
    None
}

fn signals_quit_cleanly_during_sync() {
    for signal in ["INT", "TERM"] {
        let web = FakeWeb::start();
        web.route("/slow", |_| {
            std::thread::sleep(Duration::from_secs(30));
            Response::new(200, "text/plain", "")
        });
        let dir = TempDir::new();
        let db = Db::open(&dir.path().join("data").join("readrss.db")).unwrap();
        for i in 0..10 {
            db.add_feed(&format!("{}?{i}", web.url("/slow")), "");
        }
        drop(db);
        let app_id = unique_id();
        let mut child = spawn_binary(&dir, &app_id, &["--background"]);
        assert!(wait_for(|| name_owned(&app_id), 15), "app não registrou o nome no D-Bus");
        assert!(wait_for(|| !web.requests().is_empty(), 15), "sync deveria estar em andamento");
        let started = Instant::now();
        Command::new("kill").args([&format!("-{signal}"), &child.id().to_string()]).status().unwrap();
        let status = exit_within(&mut child, 10).expect("app não encerrou");
        let mut stderr = String::new();
        std::io::Read::read_to_string(child.stderr.as_mut().unwrap(), &mut stderr).unwrap();
        assert!(status.success(), "SIG{signal}: {status:?} {stderr}");
        assert!(!stderr.contains("panicked"), "{stderr}");
        assert!(started.elapsed() < Duration::from_secs(3), "SIG{signal}: saída esperou os downloads");
        assert!(!dir.path().join("data").join("readrss.db-wal").exists(), "banco deveria ser fechado (WAL)");
        println!("    SIG{signal}: saiu em {:?}", started.elapsed());
    }
}

fn second_background_start_is_a_no_op() {
    let dir = TempDir::new();
    let app_id = unique_id();
    let mut first = spawn_binary(&dir, &app_id, &["--background"]);
    assert!(wait_for(|| name_owned(&app_id), 15));
    let mut second = spawn_binary(&dir, &app_id, &["--background"]);
    let status = exit_within(&mut second, 10).expect("segunda instância não terminou");
    assert!(status.success());
    assert!(first.try_wait().unwrap().is_none(), "primeira instância deve continuar rodando");
    Command::new("kill").args(["-TERM", &first.id().to_string()]).status().unwrap();
    assert!(exit_within(&mut first, 10).is_some_and(|s| s.success()));
}

fn main() {
    if std::env::var_os("WAYLAND_DISPLAY").is_none() && std::env::var_os("DISPLAY").is_none() {
        println!("sem display gráfico: testes de GUI ignorados");
        return;
    }
    unsafe {
        std::env::set_var("GSK_RENDERER", "gl");
    }
    let scenarios: [(&str, fn()); 6] = [
        ("main_flow", main_flow),
        ("filter_and_edit", filter_and_edit),
        ("webkit_processes_do_not_accumulate", webkit_processes_do_not_accumulate),
        ("background_mode", background_mode),
        ("signals_quit_cleanly_during_sync", signals_quit_cleanly_during_sync),
        ("second_background_start_is_a_no_op", second_background_start_is_a_no_op),
    ];
    let filter = std::env::args().nth(1).filter(|a| !a.starts_with('-'));
    for (name, scenario) in scenarios {
        if filter.as_deref().is_some_and(|f| !name.contains(f)) {
            continue;
        }
        println!("test {name} ...");
        scenario();
        println!("test {name} ... ok");
    }
}
