//! Running one command against the engine. Plain synchronous code: the server calls it on a blocking
//! thread, because engine writes wait for the disk (ADR 0003).

use emberkv_storage::engine::{Engine, EngineError};

use crate::reply::Reply;

/// Commands this server understands, for "wrong number of arguments" replies.
const KNOWN: [&[u8]; 5] = [b"PING", b"GET", b"SET", b"DEL", b"EXISTS"];

/// Runs the command in `args`, where `args[0]` is its name in any letter case.
#[must_use]
pub fn execute(engine: &Engine, args: &[Vec<u8>]) -> Reply {
    let Some((name, rest)) = args.split_first() else {
        return Reply::Error("ERR empty command".into());
    };
    let name = name.to_ascii_uppercase();

    match (name.as_slice(), rest) {
        (b"PING", []) => Reply::Simple("PONG"),
        (b"PING", [message]) => Reply::Bulk(message.clone()),
        (b"GET", [key]) => match engine.get(key) {
            Ok(Some(value)) => Reply::Bulk(value),
            Ok(None) => Reply::Null,
            Err(error) => engine_error(&error),
        },
        (b"SET", [key, value]) => match engine.set(key.clone(), value.clone()) {
            Ok(()) => Reply::Simple("OK"),
            Err(error) => engine_error(&error),
        },
        (b"DEL", keys) if !keys.is_empty() => count(keys, |key| engine.delete(key)),
        (b"EXISTS", keys) if !keys.is_empty() => count(keys, |key| engine.exists(key)),
        // Clients such as redis-cli first try `HELLO 3` to switch to RESP3. This exact error makes
        // them fall back to RESP2.
        (b"HELLO", _) => Reply::Error("NOPROTO this server only supports RESP2".into()),
        (known, _) if KNOWN.contains(&known) => Reply::Error(format!(
            "ERR wrong number of arguments for '{}' command",
            String::from_utf8_lossy(known).to_lowercase()
        )),
        (unknown, _) => Reply::Error(format!(
            "ERR unknown command '{}'",
            String::from_utf8_lossy(&unknown[..unknown.len().min(64)])
        )),
    }
}

/// Runs `action` on each key and replies with how many returned `true`, as Redis's `DEL` and
/// `EXISTS` do. Stops at the first error; earlier keys may already be deleted.
fn count(keys: &[Vec<u8>], action: impl Fn(&[u8]) -> Result<bool, EngineError>) -> Reply {
    let mut total = 0;
    for key in keys {
        match action(key) {
            Ok(true) => total += 1,
            Ok(false) => {}
            Err(error) => return engine_error(&error),
        }
    }
    Reply::Integer(total)
}

fn engine_error(error: &EngineError) -> Reply {
    Reply::Error(format!("ERR {error}"))
}

#[cfg(test)]
mod tests {
    use emberkv_storage::log::LogConfig;

    use super::*;

    fn run(engine: &Engine, words: &[&str]) -> Reply {
        let args: Vec<Vec<u8>> = words.iter().map(|word| word.as_bytes().to_vec()).collect();
        execute(engine, &args)
    }

    #[test]
    fn commands_behave_like_redis() {
        let dir = tempfile::tempdir().unwrap();
        let engine = Engine::open(dir.path(), LogConfig::default()).unwrap().0;

        assert_eq!(run(&engine, &["ping"]), Reply::Simple("PONG"));
        assert_eq!(run(&engine, &["PING", "hi"]), Reply::Bulk(b"hi".to_vec()));
        assert_eq!(run(&engine, &["GET", "k"]), Reply::Null);
        assert_eq!(run(&engine, &["set", "k", "v"]), Reply::Simple("OK"));
        assert_eq!(run(&engine, &["GET", "k"]), Reply::Bulk(b"v".to_vec()));
        assert_eq!(
            run(&engine, &["EXISTS", "k", "k", "other"]),
            Reply::Integer(2)
        );
        assert_eq!(run(&engine, &["DEL", "k", "other"]), Reply::Integer(1));
        assert_eq!(run(&engine, &["GET", "k"]), Reply::Null);
    }

    #[test]
    fn mistakes_get_redis_style_errors() {
        let dir = tempfile::tempdir().unwrap();
        let engine = Engine::open(dir.path(), LogConfig::default()).unwrap().0;

        assert_eq!(
            run(&engine, &["GET"]),
            Reply::Error("ERR wrong number of arguments for 'get' command".into())
        );
        assert_eq!(
            run(&engine, &["FLY", "away"]),
            Reply::Error("ERR unknown command 'FLY'".into())
        );
        assert!(
            matches!(run(&engine, &["HELLO", "3"]), Reply::Error(text) if text.starts_with("NOPROTO"))
        );
        assert!(
            matches!(run(&engine, &["SET", "", "v"]), Reply::Error(text) if text.contains("key is empty"))
        );
    }
}
