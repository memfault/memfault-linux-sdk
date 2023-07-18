//
// Copyright (c) Memfault, Inc.
// See License.txt for details
use std::collections::{BTreeMap, HashMap};

use serde::{Serialize, Serializer};

pub fn sorted_map<S: Serializer, K: Serialize + Ord, V: Serialize>(
    value: &HashMap<K, V>,
    serializer: S,
) -> Result<S::Ok, S::Error> {
    let mut items: Vec<(_, _)> = value.iter().collect();
    items.sort_by(|a, b| a.0.cmp(b.0));
    BTreeMap::from_iter(items).serialize(serializer)
}
