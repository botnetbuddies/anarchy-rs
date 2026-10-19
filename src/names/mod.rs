//! Name records and input data.
mod input;
mod model;
mod wordlist;

pub use input::load_names_list;
pub use model::{
    person_of, Name, NameFields, NameRef, Person, FIRSTINITIAL, FIRSTNAME, LASTINITIAL, LASTNAME,
    MIDDLEINITIAL, MIDDLENAME,
};
pub use wordlist::{load_given_list, WordList};

#[cfg(test)]
mod tests;
