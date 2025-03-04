use super::error::TomlHelper;
use log::error;
use regex::Regex;
use std::fmt;
use toml::Value;

#[derive(Clone, Debug)]
pub enum Ident {
    Name(String),
    Pattern(Regex),
}

impl fmt::Display for Ident {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Ident::Name(name) => f.write_str(name),
            // TODO: maybe store the regex string to display it here?
            Ident::Pattern(_) => f.write_str("Regex"),
        }
    }
}

impl PartialEq for Ident {
    fn eq(&self, other: &Ident) -> bool {
        pub use self::Ident::*;
        match (self, other) {
            (&Name(ref s1), &Name(ref s2)) => s1 == s2,
            (&Pattern(ref r1), &Pattern(ref r2)) => r1.as_str() == r2.as_str(),
            _ => false,
        }
    }
}

impl Eq for Ident {}

impl Ident {
    pub fn parse(toml: &Value, object_name: &str, what: &str) -> Option<Ident> {
        match toml.lookup("pattern").and_then(Value::as_str) {
            Some(s) => Regex::new(&format!("^{}$", s))
                .map(Ident::Pattern)
                .map_err(|e| {
                    error!(
                        "Bad pattern `{}` in {} for `{}`: {}",
                        s, what, object_name, e
                    );
                    e
                })
                .ok(),
            None => match toml.lookup("name").and_then(Value::as_str) {
                Some(name) => {
                    if name.contains(['.', '+', '*'].as_ref()) {
                        error!(
                            "Should be `pattern` instead of `name` in {} for `{}`",
                            what, object_name
                        );
                        None
                    } else {
                        Some(Ident::Name(name.into()))
                    }
                }
                None => None,
            },
        }
    }

    pub fn is_match(&self, name: &str) -> bool {
        use self::Ident::*;
        match *self {
            Name(ref n) => name == n,
            Pattern(ref regex) => regex.is_match(name),
        }
    }
}
