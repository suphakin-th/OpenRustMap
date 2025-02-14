use std::collections::HashMap;

use osmpbfreader::{Node, Relation, Way};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct OsmData {
	pub nodes: HashMap<i64, Node>,
	pub ways: HashMap<i64, Way>,
	pub relations: HashMap<i64, Relation>,
}
