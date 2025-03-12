#[derive(Parser, Debug)]
#[command(version, about, long_about = None)]
struct Args {
    /// Path to the OSM PBF file
    #[arg(short, long)]
    input: PathBuf,

    /// Start latitude
    #[arg(long)]
    start_lat: Option<f64>,

    /// Start longitude
    #[arg(long)]
    start_lon: Option<f64>,

    /// End latitude
    #[arg(long)]
    end_lat: Option<f64>,

    /// End longitude
    #[arg(long)]
    end_lon: Option<f64>,

    /// Export graph to JSON (optional)
    #[arg(long)]
    export_graph: Option<PathBuf>,
}

#[derive(Debug, Clone)]
struct Node {
    id: NodeId,
    point: Point<f64>,
    tags: Tags,
}

#[derive(Debug)]
struct Edge {
    source: NodeId,
    target: NodeId,
    distance: f64,
    way_id: WayId,
    highway_type: Option<String>,
}

#[derive(Debug)]
struct Graph {
    graph: UnGraph<Node, Edge>,
    node_indices: HashMap<NodeId, NodeIndex>,
}

impl Graph {
    fn new() -> Self {
        Graph {
            graph: UnGraph::new_undirected(),
            node_indices: HashMap::new(),
        }
    }

    fn add_node(&mut self, node: Node) -> NodeIndex {
        let node_idx = self.graph.add_node(node.clone());
        self.node_indices.insert(node.id, node_idx);
        node_idx
    }

    fn add_edge(&mut self, edge: Edge) {
        if let (Some(&source_idx), Some(&target_idx)) = (
            self.node_indices.get(&edge.source),
            self.node_indices.get(&edge.target),
        ) {
            self.graph.add_edge(source_idx, target_idx, edge);
        }
    }

    fn get_nearest_node(&self, lat: f64, lon: f64) -> Option<NodeIndex> {
        let query_point = Point::new(lon, lat);
        
        self.graph
            .node_indices()
            .min_by_key(|&idx| {
                let node = &self.graph[idx];
                // Using an approximate distance metric for performance
                let dist = node.point.geodesic_distance(&query_point);
                (dist * 1000.0) as i32  // Convert to mm for integer comparison
            })
    }

    fn find_shortest_path(&self, start: NodeIndex, end: NodeIndex) -> Option<(Vec<NodeIndex>, f64)> {
        astar(
            &self.graph,
            start,
            |finish| finish == end,
            |e| {
                let edge = e.weight();
                edge.distance as f64
            },
            |idx| {
                let node = &self.graph[idx];
                let target = &self.graph[end];
                node.point.geodesic_distance(&target.point)
            },
        )
    }
}