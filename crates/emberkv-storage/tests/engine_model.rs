//! Model-based test: random sequences of operations, including closing and reopening the engine, are
//! applied both to the real engine and to a plain `BTreeMap`, the "model" that is obviously correct.
//! After every step the two must agree. Any difference is a bug in the engine.

#![allow(clippy::unwrap_used)]

use std::collections::BTreeMap;
use std::path::Path;

use emberkv_storage::engine::Engine;
use emberkv_storage::log::LogConfig;
use proptest::prelude::*;

/// One step of a test run.
#[derive(Debug, Clone)]
enum Op {
    Set(u8, Vec<u8>),
    Delete(u8),
    Get(u8),
    Reopen,
}

/// A handful of keys, so the same keys are overwritten, deleted and set again often.
const KEYS: u8 = 8;

fn any_op() -> impl Strategy<Value = Op> {
    prop_oneof![
        4 => (0..KEYS, prop::collection::vec(any::<u8>(), 0..32)).prop_map(|(k, v)| Op::Set(k, v)),
        2 => (0..KEYS).prop_map(Op::Delete),
        3 => (0..KEYS).prop_map(Op::Get),
        1 => Just(Op::Reopen),
    ]
}

fn key(k: u8) -> Vec<u8> {
    vec![b'k', k]
}

/// Small segments, so runs cross segment boundaries and reopen in the middle of them.
fn open(dir: &Path) -> Engine {
    let config = LogConfig {
        max_segment_bytes: 256,
        ..LogConfig::default()
    };
    Engine::open(dir, config).unwrap().0
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(64))]

    #[test]
    fn the_engine_behaves_like_a_map(ops in prop::collection::vec(any_op(), 1..120)) {
        let dir = tempfile::tempdir().unwrap();
        let mut engine = open(dir.path());
        let mut model: BTreeMap<Vec<u8>, Vec<u8>> = BTreeMap::new();

        for op in ops {
            match op {
                Op::Set(k, value) => {
                    engine.set(key(k), value.clone()).unwrap();
                    model.insert(key(k), value);
                }
                Op::Delete(k) => {
                    let existed = engine.delete(&key(k)).unwrap();
                    prop_assert_eq!(existed, model.remove(&key(k)).is_some());
                }
                Op::Get(k) => {
                    prop_assert_eq!(engine.get(&key(k)).unwrap(), model.get(&key(k)).cloned());
                }
                Op::Reopen => {
                    drop(engine); // release the lock, as a restart would
                    engine = open(dir.path());
                }
            }

            // After every step, every key must agree, not just the one touched.
            for k in 0..KEYS {
                prop_assert_eq!(engine.exists(&key(k)).unwrap(), model.contains_key(&key(k)));
            }
        }

        // And one final restart: everything must come back exactly.
        drop(engine);
        let engine = open(dir.path());
        for k in 0..KEYS {
            prop_assert_eq!(engine.get(&key(k)).unwrap(), model.get(&key(k)).cloned());
        }
    }
}
