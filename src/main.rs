use gtk::glib;
use readrss::app::{App, Config};

fn main() -> glib::ExitCode {
    unsafe {
        if std::env::var_os("GSK_RENDERER").is_none() {
            std::env::set_var("GSK_RENDERER", "gl");
        }
    }
    let args: Vec<String> = std::env::args().collect();
    App::new(Config::from_env()).run(&args)
}
