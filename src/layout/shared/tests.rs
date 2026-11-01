use super::*;
use std::collections::HashMap;

#[test]
fn copies_of_a_log_keep_their_own_items_across_chunk_boundaries() {
    const SHARED: usize = 2 * LOG_CHUNK + 3;
    let mut log = SharedLog::default();
    for item in 0..SHARED {
        log.push(item);
    }
    let mut copy = log.clone();
    for item in 0..LOG_CHUNK {
        log.push(1000 + item);
        copy.push(2000 + item);
    }
    let expected = |offset: usize| {
        (0..SHARED)
            .chain((0..LOG_CHUNK).map(|item| offset + item))
            .collect::<Vec<_>>()
    };
    assert_eq!(
        [log.len(), copy.len()],
        [SHARED + LOG_CHUNK; 2],
        "each copy counts the items it holds"
    );
    let shared = log.clone();
    assert_eq!(
        log.into_vec(),
        expected(1000),
        "a log sharing its chunks copies them"
    );
    assert_eq!(
        shared.into_vec(),
        expected(1000),
        "the last log holding its chunks moves them"
    );
    assert_eq!(copy.into_vec(), expected(2000));
}

#[test]
fn dropping_a_long_log_does_not_nest_a_drop_per_chunk() {
    const CHUNKS: usize = 1 << 14;
    // A thread stack far smaller than the chunks would need if each dropped the one before.
    const STACK: usize = 64 * 1024;
    std::thread::Builder::new()
        .stack_size(STACK)
        .spawn(|| {
            let mut log = SharedLog::default();
            for _ in 0..CHUNKS * LOG_CHUNK {
                log.push(());
            }
            let copy = log.clone();
            drop(log);
            assert_eq!(copy.len(), CHUNKS * LOG_CHUNK, "the copy outlives the log");
        })
        .unwrap()
        .join()
        .expect("dropping the chunks one by one fits the small stack");
}

/// A key whose hash is the same for every value, so all keys share one path to the deepest
/// leaf.
#[derive(Clone, PartialEq, Eq, Debug)]
struct Colliding(usize);
impl Hash for Colliding {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        0.hash(state);
    }
}

fn entries<K: Hash + Eq + Clone, V: Clone>(map: &SharedMap<K, V>) -> HashMap<K, V> {
    let entries: HashMap<_, _> = map
        .iter()
        .map(|(key, value)| (key.clone(), value.clone()))
        .collect();
    assert_eq!(
        entries.len(),
        map.iter().count(),
        "iteration visits each entry once"
    );
    entries
}

#[test]
fn map_copies_keep_the_first_value_under_each_key_independently() {
    const KEYS: usize = 4096;
    let mut map = SharedMap::default();
    for key in 0..KEYS {
        assert!(map.insert_new(key, key), "key {key} is new");
        assert!(
            !map.insert_new(key, usize::MAX),
            "key {key} keeps its first value"
        );
    }
    let mut copy = map.clone();
    for key in KEYS..2 * KEYS {
        map.insert_new(key, 1);
        copy.insert_new(key, 2);
    }
    copy.insert_new(0, usize::MAX);
    let [map, copy] = [&map, &copy].map(entries);
    for key in 0..KEYS {
        assert_eq!(map.get(&key), Some(&key), "map lost or replaced key {key}");
        assert_eq!(
            copy.get(&key),
            Some(&key),
            "copy lost or replaced key {key}"
        );
    }
    for key in KEYS..2 * KEYS {
        assert_eq!(
            [map.get(&key), copy.get(&key)],
            [Some(&1), Some(&2)],
            "key {key} inserted after the copy belongs to each map apart"
        );
    }
    assert_eq!([map.len(), copy.len()], [2 * KEYS; 2]);
}

#[test]
fn keys_with_equal_hashes_share_the_deepest_leaf() {
    const KEYS: usize = 4 * MAP_LEAF;
    let mut map = SharedMap::default();
    for key in 0..KEYS {
        assert!(map.insert_new(Colliding(key), key), "key {key} is new");
    }
    assert!(
        !map.insert_new(Colliding(0), KEYS),
        "a colliding key is found past every split"
    );
    let entries = entries(&map);
    for key in 0..KEYS {
        assert_eq!(entries.get(&Colliding(key)), Some(&key));
    }
}
