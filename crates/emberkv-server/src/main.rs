//! The EmberKV server. Networking arrives in slice 5.

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
