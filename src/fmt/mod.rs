//! Format compilation, resolution and substitution.
mod context;
mod expand;
mod plugins;
mod resolve;
mod template;

pub use context::Ctx;
pub use expand::{enumerate, expand_strict, expansion_total, has_token, tokenize};
pub use plugins::{custom_plugin, mark_ascii_duplicates, static_plugins};
pub use resolve::{
    field_props, resolve_sequential, resolve_string, resolve_string_lower, resolve_string_lower_raw,
};
pub use template::{ExpKind, Plugin, PluginKind, Seg};

#[cfg(test)]
mod tests;
