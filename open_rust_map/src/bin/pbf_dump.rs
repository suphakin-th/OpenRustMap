use std::path::PathBuf;

use clap::Parser;
use osmpbfreader::{OsmObj, OsmPbfReader};

#[derive(Parser)]
#[command(about = "Dump OSM PBF file contents in a human-readable format")]
struct Args {
    /// PBF file path
    input: PathBuf,

    /// Show only: nodes, ways, relations, or all (default)
    #[arg(short, long, default_value = "all")]
    filter: String,

    /// Max number of objects to print (0 = unlimited)
    #[arg(short, long, default_value = "50")]
    limit: usize,

    /// Only show objects that have tags (skip untagged nodes etc.)
    #[arg(long)]
    tagged_only: bool,

    /// Filter by tag key (e.g. "highway", "name", "amenity")
    #[arg(short, long)]
    tag: Option<String>,
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args = Args::parse();

    let file = std::fs::File::open(&args.input)?;
    let mut pbf = OsmPbfReader::new(file);

    let limit = if args.limit == 0 { usize::MAX } else { args.limit };
    let mut count = 0u64;
    let mut printed = 0usize;

    for obj in pbf.iter() {
        let obj = obj?;
        count += 1;

        if printed >= limit {
            continue; // keep counting total
        }

        // Type filter
        match obj {
            OsmObj::Node(_) if args.filter != "all" && args.filter != "nodes" => continue,
            OsmObj::Way(_) if args.filter != "all" && args.filter != "ways" => continue,
            OsmObj::Relation(_) if args.filter != "all" && args.filter != "relations" => continue,
            _ => {}
        }

        // Tagged-only filter
        if args.tagged_only && obj.tags().is_empty() {
            continue;
        }

        // Tag key filter
        if let Some(ref key) = args.tag {
            if !obj.tags().contains_key(key.as_str()) {
                continue;
            }
        }

        print_obj(&obj);
        printed += 1;

        if printed >= limit {
            println!("--- reached limit of {} objects ---", limit);
        }
    }

    println!("\n=== Total objects scanned: {} | Printed: {} ===", count, printed);
    Ok(())
}

fn print_obj(obj: &OsmObj) {
    match obj {
        OsmObj::Node(node) => {
            println!("NODE id={} lat={:.6} lon={:.6}", node.id.0, node.lat(), node.lon());
            print_tags(&node.tags);
        }
        OsmObj::Way(way) => {
            println!("WAY id={} ({} nodes)", way.id.0, way.nodes.len());
            print_tags(&way.tags);
            if !way.nodes.is_empty() {
                let refs: Vec<String> = way.nodes.iter().take(10).map(|n| n.0.to_string()).collect();
                let suffix = if way.nodes.len() > 10 { " ..." } else { "" };
                println!("  refs: [{}{}]", refs.join(", "), suffix);
            }
        }
        OsmObj::Relation(rel) => {
            println!("RELATION id={} ({} members)", rel.id.0, rel.refs.len());
            print_tags(&rel.tags);
            for (i, member) in rel.refs.iter().take(10).enumerate() {
                println!("  member[{}]: {:?} role=\"{}\"", i, member.member, member.role);
            }
            if rel.refs.len() > 10 {
                println!("  ... and {} more members", rel.refs.len() - 10);
            }
        }
    }
    println!();
}

fn print_tags(tags: &osmpbfreader::objects::Tags) {
    for (k, v) in tags.iter() {
        println!("  {}={}", k, v);
    }
}
