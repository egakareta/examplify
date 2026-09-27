fn main() {
    examplify::init();
    log::info!("Triggering the example panic");
    panic!("This is an intentional example panic");
}
