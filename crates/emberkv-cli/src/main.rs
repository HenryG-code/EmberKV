//! The EmberKV command-line client. Commands arrive in slice 5.

fn usage() -> &'static str {
    "usage: emberkv-cli <get|set|del> ... (not implemented yet)"
}

fn main() {
    println!("{}", usage());
}

#[cfg(test)]
mod tests {
    use super::usage;

    #[test]
    fn usage_names_the_program() {
        assert!(usage().starts_with("usage: emberkv-cli"));
    }
}
