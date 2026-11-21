//! The `emberkv` program. The server itself lives in the `emberkv_server` library (`lib.rs`).

fn version() -> &'static str {
    env!("CARGO_PKG_VERSION")
}

fn main() {
    println!("emberkv {} (server not implemented yet)", version());
}

#[cfg(test)]
mod tests {
    use super::version;

    #[test]
    fn version_has_three_parts() {
        assert_eq!(version().split('.').count(), 3);
    }
}
