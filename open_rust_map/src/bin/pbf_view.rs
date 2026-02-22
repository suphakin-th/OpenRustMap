use std::collections::HashMap;
use std::fs::File;
use std::io::Write as IoWrite;
use std::path::PathBuf;

use clap::Parser;
use osmpbfreader::{OsmObj, OsmPbfReader};
use serde_json::{json, Value};
use sqlx::PgPool;
use tracing::info;

// ---------------------------------------------------------------------------
// CLI args
// ---------------------------------------------------------------------------

#[derive(Parser, Debug)]
#[command(
    version,
    about = "Visualize OSM data from a PBF file or from a PostGIS database"
)]
struct Args {
    /// PBF file path (required unless --from-db)
    #[arg(short, long)]
    input: Option<PathBuf>,

    /// Read road data from PostGIS instead of a PBF file
    #[arg(long)]
    from_db: bool,

    /// Output HTML file
    #[arg(short, long, default_value = "map.html")]
    output: PathBuf,

    /// Bounding box  min_lat,min_lon,max_lat,max_lon
    /// Required when using --from-db.  Strongly recommended for large PBF files.
    #[arg(long)]
    bbox: Option<String>,

    /// Road types to include (comma-separated).
    /// Use "highway" for every road type, or list specific types:
    /// residential, primary, secondary, tertiary, service, track …
    #[arg(long, default_value = "highway")]
    features: String,

    /// Maximum number of features to render (default 50 000)
    #[arg(long, default_value = "50000")]
    limit: usize,

    /// Geometry simplification tolerance.
    /// 0 = full detail.  Omit to let the tool pick a value based on the bbox span.
    #[arg(long)]
    simplify: Option<f64>,

    /// PostgreSQL connection string (only used with --from-db)
    #[arg(
        long,
        env = "DATABASE_URL",
        default_value = "postgresql://postgres:test@localhost/openrustmap"
    )]
    db_url: String,
}

// ---------------------------------------------------------------------------
// BBox helper
// ---------------------------------------------------------------------------

#[derive(Debug, Clone)]
struct BBox {
    min_lat: f64,
    min_lon: f64,
    max_lat: f64,
    max_lon: f64,
}

impl BBox {
    fn from_string(s: &str) -> Result<Self, Box<dyn std::error::Error>> {
        let p: Vec<f64> = s
            .split(',')
            .map(|v| v.trim().parse())
            .collect::<Result<_, _>>()?;
        if p.len() != 4 {
            return Err("BBox needs 4 values: min_lat,min_lon,max_lat,max_lon".into());
        }
        Ok(BBox {
            min_lat: p[0],
            min_lon: p[1],
            max_lat: p[2],
            max_lon: p[3],
        })
    }

    fn contains(&self, lat: f64, lon: f64) -> bool {
        lat >= self.min_lat && lat <= self.max_lat && lon >= self.min_lon && lon <= self.max_lon
    }

    fn center(&self) -> (f64, f64) {
        (
            (self.min_lat + self.max_lat) / 2.0,
            (self.min_lon + self.max_lon) / 2.0,
        )
    }

    /// Pick a simplification tolerance based on the larger bbox dimension.
    /// Smaller bbox  →  finer detail; larger bbox  →  coarser to keep memory down.
    fn auto_simplify(&self) -> f64 {
        let span = (self.max_lat - self.min_lat).max(self.max_lon - self.min_lon);
        if span < 0.1 {
            0.0 // city neighbourhood – full detail
        } else if span < 1.0 {
            0.0001 // city
        } else if span < 10.0 {
            0.001 // province
        } else if span < 50.0 {
            0.01 // country
        } else {
            0.1 // continent
        }
    }
}

// ---------------------------------------------------------------------------
// Entry point
// ---------------------------------------------------------------------------

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
        .with_target(false)
        .init();

    let args = Args::parse();

    // --- validate --------------------------------------------------------
    if args.from_db && args.bbox.is_none() {
        return Err("--bbox is required when using --from-db".into());
    }
    if !args.from_db && args.input.is_none() {
        return Err("Provide --input <file>  or use --from-db".into());
    }

    let bbox = args
        .bbox
        .as_ref()
        .map(|s| BBox::from_string(s))
        .transpose()?;

    let features: Vec<String> = args
        .features
        .split(',')
        .map(|s| s.trim().to_string())
        .collect();

    // --- load ------------------------------------------------------------
    let all_features = if args.from_db {
        load_from_db(&args, bbox.as_ref().unwrap(), &features).await?
    } else {
        load_from_pbf(&args, bbox.as_ref(), &features)?
    };

    info!("Total features: {}", all_features.len());

    // --- render ----------------------------------------------------------
    let (center_lat, center_lon) = bbox.as_ref().map(|b| b.center()).unwrap_or((15.87, 100.99)); // Thailand default

    let geojson = json!({ "type": "FeatureCollection", "features": all_features });

    let source_label = if args.from_db {
        "PostGIS Database".to_string()
    } else {
        args.input
            .as_ref()
            .map(|p| {
                p.file_name()
                    .unwrap_or_default()
                    .to_string_lossy()
                    .to_string()
            })
            .unwrap_or_default()
    };

    let html = generate_leaflet_html(
        &geojson.to_string(),
        center_lat,
        center_lon,
        &source_label,
        all_features.len(),
    );

    let mut file = File::create(&args.output)?;
    file.write_all(html.as_bytes())?;
    info!("Map saved: {}", args.output.display());
    Ok(())
}

// ---------------------------------------------------------------------------
// Load from PostGIS  (--from-db)
// ---------------------------------------------------------------------------

async fn load_from_db(
    args: &Args,
    bbox: &BBox,
    features: &[String],
) -> Result<Vec<Value>, Box<dyn std::error::Error>> {
    let pool = PgPool::connect(&args.db_url).await?;

    let simplify = args.simplify.unwrap_or_else(|| bbox.auto_simplify());
    let limit = args.limit as i64;
    info!(
        "DB query: simplify={}, limit={}, bbox={:?}",
        simplify, limit, bbox
    );

    let all_highways = features.iter().any(|f| f == "highway");

    let rows: Vec<(i64, String, String, String)> = if all_highways {
        sqlx::query_as::<_, (i64, String, String, String)>(
            "SELECT osm_id, \
                    COALESCE(feature_type, 'highway'), \
                    COALESCE((tags - '_node_refs')::text, '{}'), \
                    ST_AsGeoJSON(ST_Simplify(geom, $1))::text \
             FROM   osm_features \
             WHERE  osm_type = 'way' \
               AND  geom IS NOT NULL \
               AND  ST_Intersects(geom, ST_MakeEnvelope($2,$3,$4,$5, 4326)) \
               AND  ST_Simplify(geom, $1) IS NOT NULL \
             LIMIT  $6",
        )
        .bind(simplify)
        .bind(bbox.min_lon)
        .bind(bbox.min_lat)
        .bind(bbox.max_lon)
        .bind(bbox.max_lat)
        .bind(limit)
        .fetch_all(&pool)
        .await?
    } else {
        sqlx::query_as::<_, (i64, String, String, String)>(
            "SELECT osm_id, \
                    COALESCE(feature_type, 'highway'), \
                    COALESCE((tags - '_node_refs')::text, '{}'), \
                    ST_AsGeoJSON(ST_Simplify(geom, $1))::text \
             FROM   osm_features \
             WHERE  osm_type = 'way' \
               AND  geom IS NOT NULL \
               AND  feature_type = ANY($2) \
               AND  ST_Intersects(geom, ST_MakeEnvelope($3,$4,$5,$6, 4326)) \
               AND  ST_Simplify(geom, $1) IS NOT NULL \
             LIMIT  $7",
        )
        .bind(simplify)
        .bind(features.to_vec())
        .bind(bbox.min_lon)
        .bind(bbox.min_lat)
        .bind(bbox.max_lon)
        .bind(bbox.max_lat)
        .bind(limit)
        .fetch_all(&pool)
        .await?
    };

    // Helpful message when bbox contains no geometry yet
    if rows.is_empty() {
        let null_geom: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM osm_features WHERE osm_type='way' AND geom IS NULL",
        )
        .fetch_one(&pool)
        .await?;
        if null_geom > 0 {
            info!(
                "{} ways exist but have no geometry yet. \
                 Run pbf_import to finish Pass 2 (nodes) and Pass 3 (geometries).",
                null_geom
            );
        } else {
            info!("No roads found in this bbox. Try expanding the area.");
        }
    }

    info!("Fetched {} features", rows.len());

    Ok(rows
        .into_iter()
        .filter_map(|(osm_id, ftype, tags_str, geom_str)| {
            let tags: Value = serde_json::from_str(&tags_str).unwrap_or(json!({}));
            let geometry: Value = serde_json::from_str(&geom_str).ok()?;
            Some(json!({
                "type":       "Feature",
                "properties": { "osm_id": osm_id, "feature_type": ftype, "tags": tags },
                "geometry":   geometry,
            }))
        })
        .collect())
}

// ---------------------------------------------------------------------------
// Load from PBF file  (default mode)
// ---------------------------------------------------------------------------

fn load_from_pbf(
    args: &Args,
    bbox: Option<&BBox>,
    features: &[String],
) -> Result<Vec<Value>, Box<dyn std::error::Error>> {
    let input = args.input.as_ref().unwrap();

    // Block country-level files without bbox to prevent OOM
    let file_size = std::fs::metadata(input)?.len();
    if bbox.is_none() && file_size > 500_000_000 {
        return Err(format!(
            "File is {:.1} GB. Use --bbox to limit the area, \
             or use --from-db to query the database directly.",
            file_size as f64 / 1_000_000_000.0
        )
        .into());
    }

    if bbox.is_none() {
        info!(
            "No --bbox: loading entire file (limited to {} features).",
            args.limit
        );
    }

    let all_highways = features.iter().any(|f| f == "highway");

    // --- Pass 1: nodes ---------------------------------------------------
    let file = File::open(input)?;
    let mut pbf = OsmPbfReader::new(file);
    let mut nodes_map: HashMap<osmpbfreader::NodeId, (f64, f64)> = HashMap::new();

    info!("Pass 1: Reading nodes …");
    for obj in pbf.iter() {
        if let Ok(OsmObj::Node(node)) = obj {
            if let Some(bbox) = bbox {
                if !bbox.contains(node.lat(), node.lon()) {
                    continue;
                }
            }
            nodes_map.insert(node.id, (node.lat(), node.lon()));
        }
    }
    info!("Loaded {} nodes", nodes_map.len());

    // --- Pass 2: ways ----------------------------------------------------
    let file = File::open(input)?;
    let mut pbf = OsmPbfReader::new(file);
    let mut result = Vec::new();

    info!("Pass 2: Reading ways …");
    for obj in pbf.iter() {
        if result.len() >= args.limit {
            break;
        }

        if let Ok(OsmObj::Way(way)) = obj {
            // Match feature type
            let feature_type = if all_highways {
                way.tags.get("highway").cloned()
            } else {
                features
                    .iter()
                    .find_map(|f| way.tags.get(f.as_str()).cloned())
            };

            let feature_type = match feature_type {
                Some(t) => t,
                None => continue,
            };

            // Resolve node coordinates – skip ways with missing nodes
            let coords: Vec<Value> = way
                .nodes
                .iter()
                .filter_map(|nid| nodes_map.get(nid).map(|&(lat, lon)| json!([lon, lat])))
                .collect();

            if coords.len() < 2 {
                continue;
            }

            result.push(json!({
                "type":       "Feature",
                "properties": { "osm_id": way.id.0, "feature_type": feature_type },
                "geometry":   { "type": "LineString", "coordinates": coords },
            }));

            if result.len() % 5000 == 0 {
                info!("  … {} features so far", result.len());
            }
        }
    }

    Ok(result)
}

// ---------------------------------------------------------------------------
// Leaflet HTML template
// ---------------------------------------------------------------------------

fn generate_leaflet_html(
    geojson: &str,
    center_lat: f64,
    center_lon: f64,
    source_label: &str,
    feature_count: usize,
) -> String {
    format!(
        r#"<!DOCTYPE html>
<html>
<head>
<meta charset="utf-8">
<title>OpenRustMap – {source_label}</title>
<meta name="viewport" content="width=device-width, initial-scale=1.0">
<link  rel="stylesheet" href="https://unpkg.com/leaflet@1.9.4/dist/leaflet.css"/>
<script src="https://unpkg.com/leaflet@1.9.4/dist/leaflet.js"></script>
<style>
  body          {{ margin:0; padding:0; }}
  #map          {{ position:absolute; top:0; bottom:0; width:100%; }}
  .info, .legend {{
      padding:6px 8px; font:13px/18px Arial,Helvetica,sans-serif;
      background:#fff; box-shadow:0 0 15px rgba(0,0,0,.2); border-radius:5px;
  }}
  .info h4, .legend h4 {{ margin:0 0 4px; color:#555; }}
  .legend-row {{ display:flex; align-items:center; gap:6px; }}
  .legend-swatch {{ display:inline-block; width:28px; height:4px; border-radius:2px; }}
</style>
</head>
<body>
<div id="map"></div>
<script>
// ------------------------------------------------------------------
// colour palette  –  keyed on the highway=* value
// ------------------------------------------------------------------
const COLORS = {{
    motorway:'#e32017', motorway_link:'#e32017',
    trunk:'#e67300',    trunk_link:'#e67300',
    primary:'#ff7800',  primary_link:'#ff7800',
    secondary:'#ffcc00',secondary_link:'#ffcc00',
    tertiary:'#99cc00', tertiary_link:'#99cc00',
    residential:'#3388ff', living_street:'#3388ff', unclassified:'#88aadd',
    service:'#aaaaaa',  track:'#996633',
    footway:'#00cc88',  cycleway:'#00cc88', path:'#00cc88'
}};
const DEFAULT_COLOR = '#3388ff';

function getColor(type) {{ return COLORS[type] || DEFAULT_COLOR; }}
function getWeight(type) {{
    if (!type) return 1.5;
    if (type.startsWith('motorway'))  return 4;
    if (type.startsWith('trunk') || type.startsWith('primary'))    return 3;
    if (type.startsWith('secondary') || type.startsWith('tertiary')) return 2;
    return 1.5;
}}

// ------------------------------------------------------------------
// map
// ------------------------------------------------------------------
const map = L.map('map').setView([{center_lat}, {center_lon}], 10);

L.tileLayer('https://{{s}}.tile.openstreetmap.org/{{z}}/{{x}}/{{y}}.png', {{
    maxZoom: 19, attribution: '© OpenStreetMap contributors'
}}).addTo(map);

const geojsonData = {geojson};

const layer = L.geoJSON(geojsonData, {{
    style: function(f) {{
        const t = f.properties.feature_type;
        return {{ color: getColor(t), weight: getWeight(t), opacity: 0.85 }};
    }},
    onEachFeature: function(f, l) {{
        const p = f.properties;
        let html = '<b>Type:</b> ' + p.feature_type;
        if (p.tags && p.tags.name) html += '<br><b>Name:</b> ' + p.tags.name;
        html += '<br><b>OSM ID:</b> ' + p.osm_id;
        l.bindPopup(html);
    }}
}}).addTo(map);

if (geojsonData.features.length > 0) {{
    try {{ map.fitBounds(layer.getBounds()); }} catch(e) {{}}
}}

// ------------------------------------------------------------------
// info box
// ------------------------------------------------------------------
const info = L.control();
info.onAdd = function() {{
    const d = L.DomUtil.create('div', 'info');
    d.innerHTML = '<h4>OpenRustMap</h4>' +
        '<b>Source:</b> {source_label}<br>' +
        '<b>Features:</b> {feature_count}';
    return d;
}};
info.addTo(map);

// ------------------------------------------------------------------
// legend  –  only show types actually present in the data
// ------------------------------------------------------------------
const legend = L.control({{position:'bottomright'}});
legend.onAdd = function() {{
    const d = L.DomUtil.create('div', 'legend');
    // collect unique types present
    const seen = {{}};
    geojsonData.features.forEach(function(f) {{ seen[f.properties.feature_type] = true; }});
    let html = '<h4>Road types</h4>';
    Object.keys(seen).sort().forEach(function(t) {{
        html += '<div class="legend-row">' +
            '<span class="legend-swatch" style="background:' + getColor(t) + '"></span>' +
            t + '</div>';
    }});
    d.innerHTML = html;
    return d;
}};
legend.addTo(map);
</script>
</body>
</html>"#,
        source_label = source_label,
        feature_count = feature_count,
        geojson = geojson,
        center_lat = center_lat,
        center_lon = center_lon,
    )
}
