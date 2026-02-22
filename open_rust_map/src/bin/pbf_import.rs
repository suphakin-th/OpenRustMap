use clap::Parser;
use indicatif::{ProgressBar, ProgressStyle};
use osmpbfreader::{OsmObj, OsmPbfReader};
use serde_json::json;
use sqlx::{postgres::PgPoolOptions, PgPool};
use std::fs::File;
use std::path::PathBuf;
use tracing::info;

/// Allowed highway types: free public roads/paths for walking, cycling, driving
const ALLOWED_HIGHWAY_TYPES: &[&str] = &[
    // Vehicle roads
    "primary",
    "secondary",
    "tertiary",
    "primary_link",
    "secondary_link",
    "tertiary_link",
    "residential",
    "living_street",
    "unclassified",
    "service",
    "road",
    // Walking / cycling / mixed paths
    "footway",
    "path",
    "cycleway",
    "pedestrian",
    "steps",
    "bridleway",
    // Unpaved / rural
    "track",
];

/// Check if a road should be imported (only free, public roads)
fn is_public_road(tags: &osmpbfreader::Tags) -> bool {
    let highway = match tags.get("highway") {
        Some(h) => h.as_str(),
        None => return false,
    };

    if !ALLOWED_HIGHWAY_TYPES.contains(&highway) {
        return false;
    }

    if tags.get("toll").map(|v| v.as_str()) == Some("yes") {
        return false;
    }

    if let Some(access) = tags.get("access") {
        if matches!(access.as_str(), "private" | "no") {
            return false;
        }
    }

    true
}

#[derive(Parser, Debug)]
#[command(version, about = "Import OSM PBF data into PostgreSQL database")]
struct Args {
    /// Path to the OSM PBF file (required unless --pass 3)
    #[arg(short, long)]
    input: Option<PathBuf>,

    /// Database URL (defaults to DATABASE_URL env var)
    #[arg(long, env = "DATABASE_URL")]
    database_url: String,

    /// Batch size for inserts (default: 50000)
    #[arg(long, default_value = "50000")]
    batch_size: usize,

    /// Number of ways to build geometry for per chunk (default: 200000)
    #[arg(long, default_value = "200000")]
    geometry_chunk_size: i64,

    /// Run only geometry building (skip PBF scan)
    #[arg(long)]
    geometry_only: bool,
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
        .with_target(false)
        .init();

    let args = Args::parse();

    info!("Connecting to database...");
    let pool = PgPoolOptions::new()
        .max_connections(5)
        .connect(&args.database_url)
        .await?;
    info!("Database connected");

    // --geometry-only: just build geometries, no PBF scan
    if args.geometry_only {
        info!(
            "Geometry-only mode: building way geometries in chunks of {} …",
            args.geometry_chunk_size
        );
        update_way_geometries_sql(&pool, args.geometry_chunk_size).await?;
        return Ok(());
    }

    // Full import requires input file
    let input = args
        .input
        .as_ref()
        .ok_or("--input is required (use --geometry-only to skip PBF scan)")?;

    // Create data source record
    let filename = input.file_name().unwrap().to_string_lossy().to_string();
    let file_path = input
        .canonicalize()
        .ok()
        .and_then(|p| p.to_str().map(|s| s.to_string()));

    let source_id: uuid::Uuid = sqlx::query_scalar(
        "INSERT INTO data_sources (source_type, file_name, file_path, file_format, metadata, status)
         VALUES ($1, $2, $3, $4, $5, $6) RETURNING id"
    )
    .bind("osm")
    .bind(&filename)
    .bind(file_path)
    .bind("pbf")
    .bind(json!({ "import_started": chrono::Utc::now() }))
    .bind("processing")
    .fetch_one(&pool)
    .await?;

    info!("Source ID: {}", source_id);

    // ---------------------------------------------------------------
    // Single PBF scan: ways + nodes in one pass
    // ---------------------------------------------------------------
    info!(
        "Scanning PBF: ways + nodes in single pass (batch size: {}) …",
        args.batch_size
    );

    let file = File::open(input)?;
    let mut pbf = OsmPbfReader::new(file);

    let progress = ProgressBar::new_spinner();
    progress.set_style(
        ProgressStyle::default_bar()
            .template("[{elapsed_precise}] {bar:40.cyan/blue} {pos:>7}/{len:7} {msg}")
            .unwrap(),
    );

    let mut way_batch: Vec<(i64, Option<String>, serde_json::Value, Vec<i64>)> = Vec::new();
    let mut node_batch: Vec<(i64, f64, f64, serde_json::Value)> = Vec::new();
    let mut total_ways: usize = 0;
    let mut total_nodes: usize = 0;
    let mut skipped: usize = 0;
    let mut scanned: usize = 0;

    for obj in pbf.iter() {
        scanned += 1;
        if scanned.is_multiple_of(100_000) {
            progress.set_message(format!(
                "scanned: {}  ways: {}  nodes: {}  skip: {}",
                scanned, total_ways, total_nodes, skipped
            ));
            progress.inc(1);
        }

        match obj {
            Ok(OsmObj::Way(way)) => {
                if !is_public_road(&way.tags) {
                    skipped += 1;
                    continue;
                }

                let highway_type = way.tags.get("highway").map(|s| s.to_string());
                way_batch.push((
                    way.id.0,
                    highway_type,
                    serde_json::to_value(&way.tags).unwrap(),
                    way.nodes.iter().map(|n| n.0).collect(),
                ));
                total_ways += 1;

                if way_batch.len() >= args.batch_size {
                    insert_way_metadata_batch(&pool, &way_batch, source_id).await?;
                    info!("✓ ways batch {} (total: {})", way_batch.len(), total_ways);
                    way_batch.clear();
                }
            }
            Ok(OsmObj::Node(node)) => {
                node_batch.push((
                    node.id.0,
                    node.lon(),
                    node.lat(),
                    serde_json::to_value(&node.tags).unwrap(),
                ));
                total_nodes += 1;

                if node_batch.len() >= args.batch_size {
                    insert_node_batch(&pool, &node_batch, source_id).await?;
                    info!(
                        "✓ nodes batch {} (total: {})",
                        node_batch.len(),
                        total_nodes
                    );
                    node_batch.clear();
                }
            }
            _ => {}
        }
    }

    // Flush remaining batches
    if !way_batch.is_empty() {
        insert_way_metadata_batch(&pool, &way_batch, source_id).await?;
        info!("✓ ways final batch {}", way_batch.len());
    }
    if !node_batch.is_empty() {
        insert_node_batch(&pool, &node_batch, source_id).await?;
        info!("✓ nodes final batch {}", node_batch.len());
    }

    progress.finish_with_message(format!(
        "PBF done: {} ways, {} nodes, {} skipped",
        total_ways, total_nodes, skipped
    ));

    // ---------------------------------------------------------------
    // Geometry building
    // ---------------------------------------------------------------
    info!(
        "Building way geometries in chunks of {} …",
        args.geometry_chunk_size
    );
    update_way_geometries_sql(&pool, args.geometry_chunk_size).await?;

    // Update data source status
    sqlx::query(
        "UPDATE data_sources SET metadata = metadata || $1, status = $2, row_count = $3 WHERE id = $4"
    )
    .bind(json!({
        "import_completed": chrono::Utc::now(),
        "total_ways": total_ways,
        "total_nodes": total_nodes,
    }))
    .bind("ready")
    .bind((total_ways + total_nodes) as i64)
    .bind(source_id)
    .execute(&pool)
    .await?;

    info!(
        "Import complete! {} ways + {} nodes",
        total_ways, total_nodes
    );
    Ok(())
}

async fn insert_node_batch(
    pool: &PgPool,
    batch: &[(i64, f64, f64, serde_json::Value)],
    source_id: uuid::Uuid,
) -> Result<(), sqlx::Error> {
    let ids: Vec<i64> = batch.iter().map(|r| r.0).collect();
    let lons: Vec<f64> = batch.iter().map(|r| r.1).collect();
    let lats: Vec<f64> = batch.iter().map(|r| r.2).collect();
    let tags: Vec<String> = batch
        .iter()
        .map(|r| serde_json::to_string(&r.3).unwrap())
        .collect();

    sqlx::query(
        "WITH input AS (
            SELECT unnest($1::bigint[])            as osm_id,
                   unnest($2::double precision[])  as lon,
                   unnest($3::double precision[])  as lat,
                   unnest($4::text[])::jsonb       as tags
        )
        INSERT INTO osm_features (osm_id, osm_type, feature_type, geom, tags, source_id)
        SELECT osm_id, 'node', 'highway_node',
               ST_SetSRID(ST_MakePoint(lon, lat), 4326),
               tags, $5
        FROM input
        ON CONFLICT (osm_id, osm_type) DO UPDATE SET
            geom       = EXCLUDED.geom,
            tags       = EXCLUDED.tags,
            source_id  = EXCLUDED.source_id,
            updated_at = NOW()",
    )
    .bind(&ids[..])
    .bind(&lons[..])
    .bind(&lats[..])
    .bind(&tags[..])
    .bind(source_id)
    .execute(pool)
    .await?;

    Ok(())
}

async fn insert_way_metadata_batch(
    pool: &PgPool,
    batch: &[(i64, Option<String>, serde_json::Value, Vec<i64>)],
    source_id: uuid::Uuid,
) -> Result<(), sqlx::Error> {
    let mut query_builder = sqlx::QueryBuilder::new(
        "INSERT INTO osm_features (osm_id, osm_type, feature_type, tags, source_id) ",
    );

    query_builder.push_values(batch, |mut b, (osm_id, highway_type, tags, node_ids)| {
        let mut tags_with_nodes = tags.clone();
        if let serde_json::Value::Object(ref mut map) = tags_with_nodes {
            map.insert(
                "_node_refs".to_string(),
                serde_json::to_value(node_ids).unwrap(),
            );
        }

        b.push_bind(osm_id)
            .push_bind("way")
            .push_bind(highway_type.as_deref().unwrap_or("highway"))
            .push_bind(tags_with_nodes)
            .push_bind(source_id);
    });

    query_builder.push(
        " ON CONFLICT (osm_id, osm_type) DO UPDATE SET \
         feature_type = EXCLUDED.feature_type, \
         tags = EXCLUDED.tags, \
         source_id = EXCLUDED.source_id, \
         updated_at = NOW()",
    );

    query_builder.build().execute(pool).await?;
    Ok(())
}

async fn update_way_geometries_sql(
    pool: &PgPool,
    chunk_size: i64,
) -> Result<(), Box<dyn std::error::Error>> {
    info!("Building way geometries in chunks of {} …", chunk_size);

    let mut total_built: u64 = 0;
    let mut chunk_num: u64 = 0;

    loop {
        chunk_num += 1;
        let result = sqlx::query(
            "WITH target AS (
                SELECT id FROM osm_features
                WHERE osm_type = 'way' AND geom IS NULL
                ORDER BY id
                LIMIT $1
            ),
            way_nodes AS (
                SELECT
                    w.id as way_pk,
                    jsonb_array_elements_text(w.tags->'_node_refs')::bigint as node_osm_id,
                    ordinality as node_order
                FROM osm_features w,
                     jsonb_array_elements(w.tags->'_node_refs') WITH ORDINALITY
                WHERE w.id IN (SELECT id FROM target)
            ),
            way_coords AS (
                SELECT
                    wn.way_pk,
                    array_agg(n.geom ORDER BY wn.node_order) as points
                FROM way_nodes wn
                INNER JOIN osm_features n
                    ON n.osm_id = wn.node_osm_id
                    AND n.osm_type = 'node'
                GROUP BY wn.way_pk
                HAVING COUNT(*) >= 2
            )
            UPDATE osm_features w
            SET geom = ST_MakeLine(wc.points)
            FROM way_coords wc
            WHERE w.id = wc.way_pk",
        )
        .bind(chunk_size)
        .execute(pool)
        .await?;

        let built = result.rows_affected();
        if built == 0 {
            break;
        }

        total_built += built;
        info!(
            "  chunk {}: {} geometries (total: {})",
            chunk_num, built, total_built
        );
    }

    info!("✓ Geometry complete: {} ways", total_built);
    Ok(())
}
