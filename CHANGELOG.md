# Changelog

## [0.3.0] - 2026-09-27

### Added

- Capture WebAssembly panics as error-level log entries, including the source
  location and browser JavaScript stack trace when available.
- Add `panic` and `civilization` examples demonstrating panic reporting and
  logging in a resource-management simulation.

### Changed

- Refresh the README with a centered project heading, build and crates.io
  badges, and a console showcase image.

## [0.2.0] - 2026-09-27

### Added

- `init()` now returns a chainable `Init` handle, so the logger level can be
  configured with `examplify::init().with_log_level(...)`. On native targets
  this sets the maximum forwarded level; on WebAssembly it sets the default
  level shown in Examplify's DOM console.

### Changed

- Native logging now defaults to `Debug` and uses the configured level instead
  of reading `env_logger` configuration from the environment. Set the desired
  level explicitly with `with_log_level`.

## [0.1.0] - 2026-09-27

### Added

- Initial release of Examplify, with `init()` and fallible `try_init()` for
  installing the `log` facade logger.
- Forward native log records to `env_logger`; on WebAssembly, send records to
  the browser developer console and an interactive in-page console.
- Add searchable, level-filterable browser logs with selection/copy, clear,
  and minimize/reopen controls.
- Capture up to 1,000 recent log entries, accessible through `entries()` and
  removable with `clear()`.
