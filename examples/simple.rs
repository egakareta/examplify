#[cfg(target_arch = "wasm32")]
use wasm_bindgen::prelude::*;

fn log_messages() {
    log::warn!("This is a warn.");
    log::error!("This is an error.");
    log::trace!("This is a trace.");
    log::debug!("This is a debug message.");
}

#[cfg(target_arch = "wasm32")]
#[wasm_bindgen]
pub fn start_loop() {
    use wasm_bindgen::JsCast;
    use web_sys::window;

    let closure = Closure::wrap(Box::new(move || {
        log_messages();
    }) as Box<dyn FnMut()>);

    window()
        .unwrap()
        .set_interval_with_callback_and_timeout_and_arguments_0(
            closure.as_ref().unchecked_ref(),
            1000,
        )
        .unwrap();

    closure.forget();
}

#[cfg(not(target_arch = "wasm32"))]
fn start_loop() {
    std::thread::spawn(|| {
        loop {
            log_messages();
            std::thread::sleep(std::time::Duration::from_secs(1));
        }
    });
}

fn main() {
    examplify::init();
    log::info!("Application started");

    start_loop();
}
