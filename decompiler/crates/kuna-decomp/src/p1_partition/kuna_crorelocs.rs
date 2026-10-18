//! (kuna `crorelocs`) Apply a Nintendo 3DS CRO module's relocations at load.
//!
//! The loader (`kuna_analysis::loadimage_cro`) reads [`CRORELOCS_ENV`] while it
//! maps the module, upstream of every `option` command, so the option is a
//! load-time gate bridged through the environment like `typedepth`: the CLI sets
//! the variable on the process it loads in, and the console command mirrors it for
//! a later `load file` in the same process. Default on; `off` maps the module as
//! it is on disk, with the patched words still zero.

/// The environment variable the loader reads.
pub const CRORELOCS_ENV: &str = "KUNA_CRORELOCS";

/// Whether relocations are applied in this process.
pub fn crorelocs_enabled() -> bool {
    crate::options::env_toggle(CRORELOCS_ENV, true)
}

/// Bridge a `set_kuna_option("crorelocs", val)` toggle to [`CRORELOCS_ENV`].
pub fn set_crorelocs_env(on: bool) {
    std::env::set_var(CRORELOCS_ENV, if on { "on" } else { "off" });
}
