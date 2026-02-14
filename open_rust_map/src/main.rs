use std::fs::File;
use std::collections::HashMap;
use std::path::PathBuf;
use geo::prelude::*;
use geo_types::{Point, LineString};
use hashbrown::HashSet;
use osmpbfreader::{OsmPbfReader, OsmObj, NodeId, WayId, Tags};
use petgraph::graph::{NodeIndex, UnGraph};
use petgraph::algo::astar;
use clap::Parser;
use indicatif::{ProgressBar, ProgressStyle};
use tracing::{info, debug, warn, error, instrument};
use tracing_subscriber::EnvFilter;

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
    #[allow(dead_code)]
    tags: Tags,
}

#[derive(Debug, Clone)]
struct Edge {
    source: NodeId,
    target: NodeId,
    distance: f64,
    way_id: WayId,
    #[allow(dead_code)]
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

    #[instrument(skip(self))]
    fn add_node(&mut self, node: Node) -> NodeIndex {
        let node_idx = self.graph.add_node(node.clone());
        self.node_indices.insert(node.id, node_idx);
        debug!("Added node {:?} at ({:.6}, {:.6})", 
               node.id, node.point.y(), node.point.x());
        node_idx
    }

    #[instrument(skip(self))]
    fn add_edge(&mut self, edge: Edge) {
        if let (Some(&source_idx), Some(&target_idx)) = (
            self.node_indices.get(&edge.source),
            self.node_indices.get(&edge.target),
        ) {
            self.graph.add_edge(source_idx, target_idx, edge.clone());
            debug!("Added edge from {:?} to {:?} with distance {:.2}m, way_id: {:?}", 
                   edge.source, edge.target, edge.distance, edge.way_id);
        } else {
            warn!("Could not add edge: source {:?} or target {:?} not found in graph", 
                  edge.source, edge.target);
        }
    }

    #[instrument(skip(self))]
    fn get_nearest_node(&self, lat: f64, lon: f64) -> Option<NodeIndex> {
        let query_point = Point::new(lon, lat);
        debug!("Finding nearest node to ({}, {})", lat, lon);
        
        let nearest = self.graph
            .node_indices()
            .min_by_key(|&idx| {
                let node = &self.graph[idx];
                // Using an approximate distance metric for performance
                let dist = node.point.geodesic_distance(&query_point);
                (dist * 1000.0) as i32  // Convert to mm for integer comparison
            });
            
        if let Some(idx) = nearest {
            let node = &self.graph[idx];
            debug!("Found nearest node {:?} at ({:.6}, {:.6}), distance: {:.2}m", 
                   node.id, node.point.y(), node.point.x(),
                   node.point.geodesic_distance(&query_point));
        } else {
            warn!("No nodes found in graph to calculate nearest");
        }
        
        nearest
    }

    #[instrument(skip(self))]
    fn find_shortest_path(&self, start: NodeIndex, end: NodeIndex) -> Option<(Vec<NodeIndex>, f64)> {
        debug!("Finding shortest path from node index {:?} to {:?}", start, end);
        
        let result = astar(
            &self.graph,
            start,
            |finish| finish == end,
            |e| {
                let edge = e.weight();
                edge.distance
            },
            |idx| {
                let node = &self.graph[idx];
                let target = &self.graph[end];
                node.point.geodesic_distance(&target.point)
            },
        );
        
        match &result {
            Some((cost, path)) => {
                debug!("Path found with {} nodes and cost {:.2}", path.len(), cost);
            }
            None => {
                warn!("No path found between nodes");
            }
        }

        result.map(|(cost, path)| (path, cost))
    }
}


#[instrument]
fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Initialize tracing subscriber
    tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::from_default_env())
        .with_target(false)
        .init();
    
    let args = Args::parse();
    
    info!("Reading OSM PBF file: {}", args.input.display());
    let file = File::open(&args.input)?;
    let mut pbf = OsmPbfReader::new(file);
    
    // First pass: collect ways and identify needed node IDs
    info!("First pass: Collecting highway ways...");
    let progress_style = ProgressStyle::default_bar()
        .template("[{elapsed_precise}] {bar:40.cyan/blue} {pos:>7}/{len:7} {msg}")
        .unwrap();

    let mut ways = Vec::new();
    let mut needed_node_ids = HashSet::new();

    let progress = ProgressBar::new_spinner();
    progress.set_style(progress_style.clone());

    for (i, obj) in pbf.iter().enumerate() {
        if i % 100000 == 0 {
            progress.set_message(format!("Pass 1: Processed {} objects", i));
            progress.inc(1);
        }

        if let Ok(OsmObj::Way(way)) = obj {
            // Only keep ways that are roads/paths
            if way.tags.contains_key("highway") {
                // Collect node IDs needed for this way
                for &node_id in &way.nodes {
                    needed_node_ids.insert(node_id);
                }
                ways.push(way);
            }
        }
    }
    progress.finish_with_message(format!("Found {} highway ways using {} nodes", ways.len(), needed_node_ids.len()));
    info!("Found {} highway ways using {} unique nodes", ways.len(), needed_node_ids.len());

    // Second pass: collect only needed nodes
    info!("Second pass: Loading required nodes...");
    let file = File::open(&args.input)?;
    let mut pbf = OsmPbfReader::new(file);

    let mut nodes = HashMap::new();
    let progress = ProgressBar::new(needed_node_ids.len() as u64);
    progress.set_style(progress_style.clone());

    for (i, obj) in pbf.iter().enumerate() {
        if i % 100000 == 0 {
            progress.set_message(format!("Pass 2: Processed {} objects", i));
        }

        if let Ok(OsmObj::Node(node)) = obj {
            // Only store nodes that are needed for highways
            if needed_node_ids.contains(&node.id) {
                nodes.insert(
                    node.id,
                    Node {
                        id: node.id,
                        point: Point::new(node.lon(), node.lat()),
                        tags: node.tags,
                    },
                );
                progress.inc(1);
            }
        }
    }
    progress.finish_with_message(format!("Loaded {} nodes", nodes.len()));
    info!("Loaded {} nodes for graph building", nodes.len());
    
    // Build graph
    info!("Building graph...");
    let mut graph = Graph::new();

    // Add nodes to graph
    let progress = ProgressBar::new(nodes.len() as u64);
    progress.set_style(progress_style.clone());

    for node in nodes.values() {
        graph.add_node(node.clone());
        progress.inc(1);
    }
    progress.finish_with_message("Added nodes to graph");
    info!("Added {} nodes to graph", nodes.len());
    
    // Add edges
    info!("Adding edges...");
    let progress = ProgressBar::new(ways.len() as u64);
    progress.set_style(progress_style);
    
    for way in &ways {
        let highway_type = way.tags.get("highway").map(|s| s.to_string());
        
        // Create edges between consecutive nodes
        for window in way.nodes.windows(2) {
            if let [source, target] = *window {
                if let (Some(source_node), Some(target_node)) = (nodes.get(&source), nodes.get(&target)) {
                    let distance = source_node.point.geodesic_distance(&target_node.point);
                    
                    graph.add_edge(Edge {
                        source,
                        target,
                        distance,
                        way_id: way.id,
                        highway_type: highway_type.clone(),
                    });
                }
            }
        }
        progress.inc(1);
    }
    progress.finish_with_message("Built graph");
    
    info!("Graph built with {} nodes and {} edges", 
          graph.graph.node_count(), 
          graph.graph.edge_count());
    
    // If coordinates are provided, find path
    if let (Some(start_lat), Some(start_lon), Some(end_lat), Some(end_lon)) = 
       (args.start_lat, args.start_lon, args.end_lat, args.end_lon) {
        
        info!("Finding shortest path from ({}, {}) to ({}, {})", 
              start_lat, start_lon, end_lat, end_lon);
        
        if let (Some(start_idx), Some(end_idx)) = (
            graph.get_nearest_node(start_lat, start_lon),
            graph.get_nearest_node(end_lat, end_lon),
        ) {
            let start_node = &graph.graph[start_idx];
            let end_node = &graph.graph[end_idx];
            
            info!("Nearest start node: {:?} at ({}, {})", 
                  start_node.id, 
                  start_node.point.y(), 
                  start_node.point.x());
            
            info!("Nearest end node: {:?} at ({}, {})", 
                  end_node.id, 
                  end_node.point.y(), 
                  end_node.point.x());
            
            if let Some((path, cost)) = graph.find_shortest_path(start_idx, end_idx) {
                info!("Found path with {} nodes and total distance of {:.2} km", 
                      path.len(), cost / 1000.0);
                
                // Print detailed path info
                debug!("Path details:");
                let path_line = LineString(path.iter()
                    .map(|&idx| {
                        let node = &graph.graph[idx];
                        (node.point.x(), node.point.y()).into()
                    })
                    .collect());
                
                for (i, &idx) in path.iter().enumerate() {
                    if i % 10 == 0 || i == path.len() - 1 {  // print every 10th node or the last one
                        let node = &graph.graph[idx];
                        debug!("  Node {}: ({:.6}, {:.6})", 
                               i, node.point.y(), node.point.x());
                    }
                }
                
                // Output GeoJSON path
                info!("Path GeoJSON:");
                info!("{{");
                info!("  \"type\": \"LineString\",");
                info!("  \"coordinates\": [");
                
                let mut geojson = String::new();
                for (i, point) in path_line.0.iter().enumerate() {
                    let line = format!("    [{}, {}]{}", 
                        point.x, point.y, 
                        if i < path_line.0.len() - 1 { "," } else { "" }
                    );
                    geojson.push_str(&line);
                    geojson.push('\n');
                }
                
                info!("  {}]", geojson);
                info!("}}");
            } else {
                warn!("No path found between the given points");
            }
        } else {
            error!("Could not find nearest nodes to the given coordinates");
        }
    }
    
    // Export graph if requested
    if let Some(export_path) = args.export_graph {
        info!("Exporting graph to {}", export_path.display());
        // Implementation for graph export...
    }
    
    Ok(())
}