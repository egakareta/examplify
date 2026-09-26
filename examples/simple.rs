//! Run this in
//! - native `cargo run --example simple`
//! - web `trunk serve --example simple`

fn main() {
    examplify::init().with_log_level(examplify::log::LevelFilter::Info);
    log::info!("Application started");
}
