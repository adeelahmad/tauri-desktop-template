//! {{Title}}: say in one line what this module does.
//!
//! ```
//! assert_eq!(desktop_core::{{name}}::greet("world"), "Hello, world!");
//! ```

/// An example to replace.
pub fn greet(who: &str) -> String {
    format!("Hello, {who}!")
}

#[cfg(test)]
mod tests {
    #[test]
    fn greets() {
        assert_eq!(super::greet("x"), "Hello, x!");
    }
}
