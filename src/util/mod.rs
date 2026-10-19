//! Byte operations and Ruby-compatible text handling.
mod bytes;
mod case;
mod split;

pub use bytes::{bytes_equal, push_bytes};
pub use case::{
    ascii_lowercase_in_place, downcase, downcase_into, nth_char_boundary,
    strip_spaces_and_apostrophes, upcase_it_into, upcase_then_downcase_into,
};
pub use split::{ruby_split, ruby_split_buf, strip, Delim};
