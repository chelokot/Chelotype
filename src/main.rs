use chelotype::app;
use chelotype::diagnostics;
use gtk::glib;

fn main() -> glib::ExitCode {
    if std::env::var("CHELOTYPE_HEADLESS").ok().as_deref() == Some("1") {
        return diagnostics::run_headless();
    }
    app::run_app()
}
