use nonempty::NonEmpty;
use num_traits::ToPrimitive;
use osmpbfreader::{Node, NodeId, Relation, RelationId, Way, WayId};

use crate::model::osm_data_model::OsmData;

impl OsmData {
	pub fn add_node(&mut self, node: Node) {
		self.nodes.insert(node.id.0, node.clone());
	}

	pub fn add_way(&mut self, way: Way) {
		self.ways.insert(way.id.0, way.clone());
	}

	pub fn add_relation(&mut self, relation: Relation) {
		self.relations.insert(relation.id.0, relation.clone());
	}

	pub fn get_node_by_id(&self, id: i64) -> Option<&Node> {
		self.nodes.get(&id)
	}

	pub fn get_way_by_id(&self, id: i64) -> Option<&Way> {
		self.ways.get(&id)
	}

	pub fn get_relation_by_id(&self, id: i64) -> Option<&Relation> {
		self.relations.get(&id)
	}

	pub fn from_osm_pbf_file(mut pbf: osmpbfreader::OsmPbfReader<std::fs::File>) -> OsmData {
		let mut osm_data = OsmData::default();
		for obj in pbf.iter().map(Result::unwrap) {
			match obj {
				osmpbfreader::OsmObj::Node(node) => {
					osm_data.add_node(node);
				}
				osmpbfreader::OsmObj::Way(way) => {
					osm_data.add_way(way);
				}
				osmpbfreader::OsmObj::Relation(relation) => {
					osm_data.add_relation(relation);
				}
			}
		}
		osm_data
	}

	pub fn get_coordinate_by_node(&self, node: &Node) -> Result<(f64, f64), String> {
		let micro = 10_000_000.0;
		let lat = node
			.decimicro_lat
			.to_f64()
			.map(|f| f / micro)
			.ok_or("Could not parse to f64")?;
		let lon = node
			.decimicro_lon
			.to_f64()
			.map(|f| f / micro)
			.ok_or("Could not parse to f64")?;
		Ok((lon, lat))
	}

	pub fn get_coordinate_by_node_id(&self, id: i64) -> Option<(f64, f64)> {
		self.get_node_by_id(id)
			.and_then(|node| self.get_coordinate_by_node(node).ok())
	}

	pub fn get_coordinates_by_way(&self, way: &Way) -> Vec<(f64, f64)> {
		way.nodes
			.iter()
			.filter_map(|id| self.get_coordinate_by_node_id(id.0))
			.collect::<Vec<_>>()
	}

	pub fn get_coordinates_by_way_id(&self, id: i64) -> Option<Vec<(f64, f64)>> {
		self.get_way_by_id(id)
			.map(|way| self.get_coordinates_by_way(way))
	}

	pub fn get_outer_coordinates_by_relation(
		&self,
		relation: &Relation,
	) -> Option<Vec<(f64, f64)>> {
		let x = self.get_outer_coordinates_by_relation_id(relation.id.0)?;
		NonEmpty::from_vec(x).map(|c| c.into())
	}

	pub fn get_outer_coordinates_by_relation_id(&self, id: i64) -> Option<Vec<(f64, f64)>> {
		let relation = self.get_relation_by_id(id)?;
		let RelationId(cur_id) = relation.id;
		let res = relation
			.refs
			.iter()
			.filter_map(|member| {
				let role = member.role.clone().to_lowercase();
				if role != "outer" {
					return None;
				}
				match member.member {
					osmpbfreader::OsmId::Node(NodeId(id)) => {
						self.get_coordinate_by_node_id(id).map(|c| vec![c])
					}
					osmpbfreader::OsmId::Way(WayId(id)) => self.get_coordinates_by_way_id(id),
					osmpbfreader::OsmId::Relation(RelationId(id)) => {
						if cur_id == id {
							return None;
						}
						self.get_outer_coordinates_by_relation_id(id)
					}
				}
			})
			.flatten()
			.collect::<Vec<_>>();
		Some(res)
	}

	pub fn get_coordinates_by_relation(&self, relation: &Relation) -> Option<Vec<(f64, f64)>> {
		self.get_coordinates_by_relation_id(relation.id.0)
	}

	pub fn get_coordinates_by_relation_id(&self, relation_id: i64) -> Option<Vec<(f64, f64)>> {
		let relation = self.get_relation_by_id(relation_id)?;
		let res = relation
			.refs
			.iter()
			.filter_map(|member| match member.member {
				osmpbfreader::OsmId::Node(NodeId(id)) => {
					self.get_coordinate_by_node_id(id).map(|c| vec![c])
				}
				osmpbfreader::OsmId::Way(WayId(id)) => self.get_coordinates_by_way_id(id),
				osmpbfreader::OsmId::Relation(RelationId(id)) => {
					if relation_id == id {
						return None;
					}
					self.get_coordinates_by_relation_id(id)
				}
			})
			.flatten()
			.collect::<Vec<_>>();
		Some(res)
	}

	pub fn get_not_outer_coordinates_by_relation_id(
		&self,
		id: i64,
	) -> Option<Vec<serde_json::Value>> {
		let relation = self.get_relation_by_id(id)?;
		let RelationId(cur_id) = relation.id;
		let res = relation
			.refs
			.iter()
			.filter_map(|member| {
				let role = member.role.clone().to_lowercase();
				if role == "outer" {
					return None;
				}
				match member.member {
					osmpbfreader::OsmId::Node(NodeId(id)) => {
						let node = self.get_node_by_id(id)?;
						let micro = 10_000_000.0;
						let lat = node.decimicro_lat.to_f64().map(|f| f / micro)?;
						let lon = node.decimicro_lon.to_f64().map(|f| f / micro)?;
						Some(serde_json::json!({
							"id": id,
							"tags": node.tags.clone(),
							"coordinate": {
								"lon": lon,
								"lat": lat,
							},
						}))
					}
					osmpbfreader::OsmId::Way(WayId(id)) => {
						let way = self.get_way_by_id(id)?;
						let x = self.get_coordinates_by_way_id(id)?;
						let coordinates = x
							.iter()
							.map(|(lon, lat)| {
								serde_json::json!({
									"lon": lon,
									"lat": lat,
								})
							})
							.collect::<Vec<_>>();
						Some(serde_json::json!({
							"id": id,
							"tags": way.tags.clone(),
							"coordinates": coordinates,
						}))
					}
					osmpbfreader::OsmId::Relation(RelationId(id)) => {
						if cur_id == id {
							return None;
						}
						let rel = self.get_relation_by_id(id)?;
						let v = self.get_outer_coordinates_by_relation_id(id)?;
						let coordinates = v
							.iter()
							.map(|(lon, lat)| {
								serde_json::json!({
									"lon": lon,
									"lat": lat,
								})
							})
							.collect::<Vec<_>>();
						Some(serde_json::json!({
							"id": id,
							"role": member.role.clone(),
							"tags": rel.tags.clone(),
							"coordinates": coordinates,
						}))
					}
				}
			})
			.collect::<Vec<_>>();
		Some(res)
	}
}
