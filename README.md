# Examplify

Examplify is a lightweight logging UI for Rust web applications. It uses the
[`log`](https://crates.io/crates/log) facade and adds a searchable, filterable
console overlay to WebAssembly apps. On native targets, it forwards log records
to `env_logger`.

Its primary motivation is to provide an ultra-simple way for libraries to show
`examples/` in the web without having to stitch together `js-sys` or possibly a
much heavier crate like `eframe`.

## Usage

```rust
use examplify::log;

fn main() {
    examplify::init().with_log_level(log::LevelFilter::Info);
    log::info!("Application started");
    log::warn!("Something needs attention");
}
```

`examplify::init()` installs the logger for the current target. If another
logger might already be installed, use `examplify::try_init()` to handle that
case:

```rust
if let Err(error) = examplify::try_init() {
    eprintln!("Could not install Examplify logger: {error}");
}
```

## License

Licensed under the [MIT License](https://github.com/egakareta/examplify/blob/master/LICENSE).
