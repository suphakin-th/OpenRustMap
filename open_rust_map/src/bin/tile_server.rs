use axum::{
    extract::{Path, State},
    http::{header, HeaderMap, StatusCode},
    response::{IntoResponse, Response},
    routing::get,
    Json, Router,
};
use clap::Parser;
use serde::Serialize;
use serde_json::json;
use sqlx::postgres::PgPoolOptions;
use sqlx::PgPool;
use tower_http::cors::{Any, CorsLayer};
use tracing::{error, info};

/// Vector tile server for OpenRustMap — serves PostGIS ST_AsMVT tiles to MapLibre GL JS
#[derive(Parser, Debug)]
#[command(
    version,
    about = "Serve vector tiles from PostGIS via Axum + MapLibre GL JS"
)]
struct Args {
    /// PostgreSQL connection URL
    #[arg(long, env = "DATABASE_URL")]
    database_url: String,

    /// HTTP port to listen on
    #[arg(long, default_value = "8080")]
    port: u16,

    /// Maximum database connections in pool
    #[arg(long, default_value = "10")]
    max_connections: u32,
}

#[derive(Clone)]
struct AppState {
    pool: PgPool,
}

// ---------------------------------------------------------------------------
// MapLibre GL JS frontend — served as a static HTML string
// ---------------------------------------------------------------------------
const INDEX_HTML: &str = r##"<!DOCTYPE html>
<html lang="en">
<head>
  <meta charset="utf-8">
  <title>OpenRustMap – Vector Tiles</title>
  <meta name="viewport" content="width=device-width, initial-scale=1.0">
  <link href="https://unpkg.com/maplibre-gl@4.7.1/dist/maplibre-gl.css" rel="stylesheet">
  <script src="https://unpkg.com/maplibre-gl@4.7.1/dist/maplibre-gl.js"></script>
  <style>
    html, body { margin: 0; padding: 0; height: 100%; }
    #map { width: 100%; height: 100%; }
    #info {
      position: absolute;
      top: 10px;
      left: 10px;
      background: rgba(255,255,255,0.92);
      padding: 10px 14px;
      border-radius: 6px;
      font: 13px/1.6 Arial, sans-serif;
      box-shadow: 0 2px 8px rgba(0,0,0,.2);
      max-width: 280px;
      z-index: 1;
    }
    #info h4 { margin: 0 0 4px; font-size: 15px; }
    #legend { margin-top: 8px; }
    .legend-row { display: flex; align-items: center; gap: 6px; font-size: 12px; }
    .legend-line { width: 24px; height: 3px; border-radius: 2px; }
  </style>
</head>
<body>
<div id="map"></div>
<div id="info">
  <h4>OpenRustMap</h4>
  <div id="feature-count">Loading...</div>
  <div id="legend">
    <div class="legend-row"><div class="legend-line" style="background:#e32017;height:4px"></div>Motorway / Trunk</div>
    <div class="legend-row"><div class="legend-line" style="background:#ff7800"></div>Primary</div>
    <div class="legend-row"><div class="legend-line" style="background:#ffcc00"></div>Secondary</div>
    <div class="legend-row"><div class="legend-line" style="background:#99cc00"></div>Tertiary</div>
    <div class="legend-row"><div class="legend-line" style="background:#3388ff"></div>Residential</div>
    <div class="legend-row"><div class="legend-line" style="background:#00cc88"></div>Footway / Cycleway</div>
    <div class="legend-row"><div class="legend-line" style="background:#aaaaaa"></div>Service / Other</div>
  </div>
</div>

<script>
// Road color palette — mirrors pbf_view.rs COLORS palette exactly
const ROAD_COLOR = [
  "match", ["get", "feature_type"],
  "motorway",       "#e32017",
  "motorway_link",  "#e32017",
  "trunk",          "#e67300",
  "trunk_link",     "#e67300",
  "primary",        "#ff7800",
  "primary_link",   "#ff7800",
  "secondary",      "#ffcc00",
  "secondary_link", "#ffcc00",
  "tertiary",       "#99cc00",
  "tertiary_link",  "#99cc00",
  "residential",    "#3388ff",
  "living_street",  "#3388ff",
  "unclassified",   "#88aadd",
  "service",        "#aaaaaa",
  "track",          "#996633",
  "footway",        "#00cc88",
  "cycleway",       "#00cc88",
  "path",           "#00cc88",
  "#3388ff"
];

// Zoom-interpolated line widths
const ROAD_WIDTH = [
  "interpolate", ["linear"], ["zoom"],
  6,  ["match", ["get", "feature_type"],
        ["motorway", "trunk", "motorway_link", "trunk_link"], 1.5,
        ["primary", "primary_link"], 1.0,
        0.5],
  12, ["match", ["get", "feature_type"],
        ["motorway", "trunk", "motorway_link", "trunk_link"], 5,
        ["primary", "primary_link"], 4,
        ["secondary", "secondary_link", "tertiary", "tertiary_link"], 3,
        2],
  16, ["match", ["get", "feature_type"],
        ["motorway", "trunk", "motorway_link", "trunk_link"], 10,
        ["primary", "primary_link"], 8,
        ["secondary", "secondary_link", "tertiary", "tertiary_link"], 6,
        4]
];

async function init() {
  // Fetch metadata to get data extent and feature count
  let bounds = null;
  let centerLng = 100.5018;  // Bangkok fallback
  let centerLat = 13.7563;

  try {
    const meta = await fetch("/api/metadata").then(r => r.json());
    document.getElementById("feature-count").textContent =
      "Features: " + meta.feature_count.toLocaleString();
    if (meta.bounds) {
      bounds = meta.bounds;  // [minLon, minLat, maxLon, maxLat]
      centerLng = (bounds[0] + bounds[2]) / 2;
      centerLat = (bounds[1] + bounds[3]) / 2;
    }
  } catch (e) {
    console.warn("Could not fetch metadata:", e);
    document.getElementById("feature-count").textContent = "Metadata unavailable";
  }

  const map = new maplibregl.Map({
    container: "map",
    style: {
      version: 8,
      sources: {
        "osm-raster": {
          type: "raster",
          tiles: ["https://tile.openstreetmap.org/{z}/{x}/{y}.png"],
          tileSize: 256,
          attribution: "&copy; <a href='https://openstreetmap.org'>OpenStreetMap</a> contributors"
        },
        "osm-roads": {
          type: "vector",
          tiles: [window.location.origin + "/tiles/{z}/{x}/{y}.mvt"],
          minzoom: 5,
          maxzoom: 18
        }
      },
      layers: [
        {
          id: "background",
          type: "background",
          paint: { "background-color": "#f8f4f0" }
        },
        {
          id: "osm-basemap",
          type: "raster",
          source: "osm-raster",
          paint: { "raster-opacity": 0.4 }
        },
        {
          id: "roads",
          type: "line",
          source: "osm-roads",
          "source-layer": "roads",
          layout: { "line-join": "round", "line-cap": "round" },
          paint: {
            "line-color": ROAD_COLOR,
            "line-width": ROAD_WIDTH,
            "line-opacity": 0.85
          }
        }
      ]
    },
    center: [centerLng, centerLat],
    zoom: 11
  });

  // Fit to data bounding box once tiles have loaded
  map.on("load", () => {
    if (bounds) {
      map.fitBounds(
        [[bounds[0], bounds[1]], [bounds[2], bounds[3]]],
        { padding: 40, duration: 800 }
      );
    }
  });

  // Feature popup on click
  map.on("click", "roads", (e) => {
    const props = e.features[0].properties;
    let tags = {};
    try { tags = JSON.parse(props.tags || "{}"); } catch (_) {}
    const name = tags.name || tags["name:en"] || tags["name:th"] || "—";
    new maplibregl.Popup()
      .setLngLat(e.lngLat)
      .setHTML(
        "<b>Type:</b> " + (props.feature_type || "unknown") + "<br>" +
        "<b>Name:</b> " + name + "<br>" +
        "<b>OSM ID:</b> " + props.osm_id
      )
      .addTo(map);
  });

  map.on("mouseenter", "roads", () => { map.getCanvas().style.cursor = "pointer"; });
  map.on("mouseleave", "roads", () => { map.getCanvas().style.cursor = ""; });
}

init();
</script>
</body>
</html>"##;

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

    info!("Connecting to database...");
    let pool = PgPoolOptions::new()
        .max_connections(args.max_connections)
        .connect(&args.database_url)
        .await?;
    info!("Database connected");

    let state = AppState { pool };

    let cors = CorsLayer::new()
        .allow_origin(Any)
        .allow_methods(Any)
        .allow_headers(Any);

    let app = Router::new()
        .route("/", get(serve_index))
        .route("/tiles/:z/:x/:y", get(serve_tile))
        .route("/api/metadata", get(serve_metadata))
        .with_state(state)
        .layer(cors);

    let addr = format!("0.0.0.0:{}", args.port);
    let listener = tokio::net::TcpListener::bind(&addr).await?;
    info!("Tile server ready → http://{}", addr);
    axum::serve(listener, app).await?;

    Ok(())
}

// ---------------------------------------------------------------------------
// Route: GET /
// ---------------------------------------------------------------------------
async fn serve_index() -> impl IntoResponse {
    (
        StatusCode::OK,
        [(header::CONTENT_TYPE, "text/html; charset=utf-8")],
        INDEX_HTML,
    )
}

// ---------------------------------------------------------------------------
// Route: GET /tiles/:z/:x/:y   (MapLibre appends .mvt — handled via strip)
// ---------------------------------------------------------------------------
async fn serve_tile(
    State(state): State<AppState>,
    Path((z, x, y_raw)): Path<(i32, i32, String)>,
) -> Response {
    // Strip optional .mvt extension
    let y_str = y_raw.strip_suffix(".mvt").unwrap_or(&y_raw);
    let y: i32 = match y_str.parse() {
        Ok(v) => v,
        Err(_) => {
            return (StatusCode::BAD_REQUEST, "Invalid y tile coordinate").into_response();
        }
    };

    // Validate tile coordinates to avoid PostGIS errors
    if !(0..=22).contains(&z) {
        return (StatusCode::BAD_REQUEST, "Zoom level out of range (0–22)").into_response();
    }
    let max_tile = 1i32 << z;
    if x < 0 || x >= max_tile || y < 0 || y >= max_tile {
        return (StatusCode::BAD_REQUEST, "Tile x/y out of range for zoom").into_response();
    }

    match fetch_tile(&state.pool, z, x, y).await {
        Ok(Some(bytes)) => {
            let mut headers = HeaderMap::new();
            headers.insert(
                header::CONTENT_TYPE,
                "application/x-protobuf".parse().unwrap(),
            );
            headers.insert(
                header::CACHE_CONTROL,
                "public, max-age=3600".parse().unwrap(),
            );
            (StatusCode::OK, headers, bytes).into_response()
        }
        Ok(None) => StatusCode::NO_CONTENT.into_response(),
        Err(e) => {
            error!("Tile error z={z} x={x} y={y}: {e}");
            StatusCode::INTERNAL_SERVER_ERROR.into_response()
        }
    }
}

/// Query PostGIS for a single MVT tile using ST_AsMVT + ST_TileEnvelope.
async fn fetch_tile(pool: &PgPool, z: i32, x: i32, y: i32) -> Result<Option<Vec<u8>>, sqlx::Error> {
    // The GIST index on geom (SRID 4326) is used by the ST_Intersects predicate.
    // ST_AsMVTGeom clips and projects features into tile pixel space (0..4096).
    // COALESCE(geom_3857, ST_Transform(...)) uses the pre-projected column from
    // migration 20260222000002 when available, falling back to live transform.
    // Layer name 'roads' must match the source-layer in the MapLibre style.
    let row: Option<(Option<Vec<u8>>,)> = sqlx::query_as(
        r#"
        WITH
          tile_env AS (
            SELECT ST_TileEnvelope($1, $2, $3) AS env
          ),
          tile_env_4326 AS (
            SELECT ST_Transform(env, 4326) AS env4326
            FROM tile_env
          ),
          mvt_geom AS (
            SELECT
              f.osm_id,
              f.feature_type,
              (f.tags - '_node_refs') AS tags,
              ST_AsMVTGeom(
                COALESCE(f.geom_3857, ST_Transform(f.geom, 3857)),
                te.env,
                4096,
                256,
                true
              ) AS geom
            FROM
              osm_features f,
              tile_env te,
              tile_env_4326 t4
            WHERE
              f.geom IS NOT NULL
              AND ST_Intersects(f.geom, t4.env4326)
              AND f.osm_type = 'way'
          ),
          valid_geom AS (
            SELECT * FROM mvt_geom WHERE geom IS NOT NULL
          )
        SELECT ST_AsMVT(valid_geom.*, 'roads', 4096, 'geom')
        FROM valid_geom
        "#,
    )
    .bind(z)
    .bind(x)
    .bind(y)
    .fetch_optional(pool)
    .await?;

    match row {
        Some((Some(bytes),)) if !bytes.is_empty() => Ok(Some(bytes)),
        _ => Ok(None),
    }
}

// ---------------------------------------------------------------------------
// Route: GET /api/metadata
// ---------------------------------------------------------------------------
#[derive(Serialize)]
struct MetadataResponse {
    /// [min_lon, min_lat, max_lon, max_lat] in WGS84
    bounds: Option<[f64; 4]>,
    feature_count: i64,
    tile_url_template: String,
}

type BoundsRow = (Option<f64>, Option<f64>, Option<f64>, Option<f64>, i64);

async fn serve_metadata(State(state): State<AppState>) -> Response {
    let row: Result<BoundsRow, sqlx::Error> =
        sqlx::query_as(
            r#"
            SELECT
              ST_XMin(ST_Extent(geom))::double precision,
              ST_YMin(ST_Extent(geom))::double precision,
              ST_XMax(ST_Extent(geom))::double precision,
              ST_YMax(ST_Extent(geom))::double precision,
              COUNT(*)::bigint
            FROM osm_features
            WHERE geom IS NOT NULL
              AND osm_type = 'way'
            "#,
        )
        .fetch_one(&state.pool)
        .await;

    match row {
        Ok((min_x, min_y, max_x, max_y, count)) => {
            let bounds = match (min_x, min_y, max_x, max_y) {
                (Some(x0), Some(y0), Some(x1), Some(y1)) => Some([x0, y0, x1, y1]),
                _ => None,
            };
            (
                StatusCode::OK,
                Json(MetadataResponse {
                    bounds,
                    feature_count: count,
                    tile_url_template: "/tiles/{z}/{x}/{y}.mvt".to_string(),
                }),
            )
                .into_response()
        }
        Err(e) => {
            error!("Metadata query error: {e}");
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(json!({ "error": "database error" })),
            )
                .into_response()
        }
    }
}
