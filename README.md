# OpenRustMap: Extensible Geospatial Data Platform

## Project Vision

OpenRustMap is a production-ready geospatial data platform for environmental analysis and strategic planning. Built with Rust, PostgreSQL/PostGIS, and designed with extensibility at its core.

### Core Use Cases

- **Flood Hazard Mapping** - Analyze flood scenarios using DEM elevation data and OSM features
- **Environmental Analysis** - GISDA data integration for administrative boundaries and planning
- **Carbon Tracking** - Forest mapping and biomass estimation using Lidar data (planned)
- **Strategic Planning** - Multi-source data integration for decision-making

## Key Features

### ✅ Implemented (v0.1.0)

#### 1. Direct PBF Visualization (`pbf_view`)
- ⚡ **Instant visualization** - No database setup needed
- 🗺️ **Interactive Leaflet maps** - Generates standalone HTML
- 🎯 **Bounding box filtering** - Focus on specific areas
- 🏷️ **Feature type filtering** - highways, buildings, waterways
- 💾 **Zero memory issues** - Streams data efficiently

#### 2. Database-Backed Storage (`pbf_import`)
- 📦 **Streaming import** - Handles unlimited dataset sizes
- 🛣️ **Highway ways only** - Imports public roads/paths (filtered by access/toll)
- 🗄️ **PostgreSQL + PostGIS** - Full spatial database with GIST spatial indexes
- 💪 **Memory efficient** - Nodes stored in a temporary scratch table during import, then dropped
- 🔍 **Advanced queries** - Complex spatial operations via PostGIS

#### 3. Pathfinding (`open_rust_map`)
- 🗺️ **A* algorithm** - Geodesic distance heuristic
- ⚡ **Graph-based routing** - In-memory road networks
- 📍 **Coordinate queries** - Lat/lon to nearest node
- 📊 **GeoJSON output** - Visualize results

#### 4. Infrastructure
- 🔧 **Automated setup** - Database configuration scripts
- 📋 **Schema migrations** - Version-controlled with sqlx
- ✅ **Setup verification** - Health check scripts
- 🎨 **Facade pattern** - Clean modular architecture

#### 5. Vector Tile Server (`tile_server`)
- **WebGL rendering** - MapLibre GL JS, 10-30x faster than Leaflet/SVG
- **Streaming tiles** - Only loads the area visible on screen
- **PostGIS ST_AsMVT** - Server-side vector tile generation
- **Axum HTTP server** - Lightweight async Rust web server
- **Auto-fit to data** - Fetches bounding box and centers map on startup
- **Click popups** - Feature type, name, OSM ID on click

### 🔄 In Progress

- DEM/elevation data import
- Flood analysis module

### 📋 Planned Capabilities

#### Data Import
- Multi-format support (GeoTIFF DEMs, Lidar LAS, GISDA shapefiles)
- Parallel import processing
- Incremental updates

#### Disaster Analysis
- **Flood hazard mapping** - Water level simulation, impact assessment
- **Blast radius analysis** - Building damage, population impact
- **Wildfire risk** - Vegetation modeling, spread prediction
- **Evacuation routing** - Safe path finding, avoid hazard zones

#### Visualization & API
- Real-time web dashboard
- REST API with Axum
- Multi-layer map interface
- Export to KML, Shapefile, GeoJSON

#### Advanced Features
- 3D terrain visualization
- Temporal change detection
- Network analysis (accessibility, centrality)
- Resolution-adaptive queries

## Architecture

### Design Pattern: Facade Pattern

The project uses a facade design pattern for clean module organization:

```
base/src/
├── model.rs              # Facade: re-exports from model/
├── model/
│   ├── osm_model.rs      # OSM data structures
│   └── config_model.rs   # Configuration models
├── service.rs            # Facade: re-exports from service/
├── service/
│   └── osm_data.rs       # OSM data services
├── configuration.rs      # Facade: re-exports from configuration/
└── configuration/
    └── environment.rs    # Environment settings
```

**Benefits of this pattern:**
- Clean separation of concerns
- Easy to add new modules without breaking existing code
- Consistent import structure across the codebase
- Facade files provide single entry point for each module

### Planned Module Structure

Following the same facade pattern, new modules will be added as:

```
base/src/
├── loader.rs             # Facade for data loaders
├── loader/
│   ├── osm_loader.rs
│   ├── dem_loader.rs
│   ├── gisda_loader.rs
│   └── registry.rs
├── db.rs                 # Facade for database layer
├── db/
│   ├── pool.rs
│   └── repository/
├── analysis.rs           # Facade for analysis modules
├── analysis/
│   ├── flood_analyzer.rs
│   └── carbon_tracker.rs
└── pipeline.rs           # Facade for data pipelines
    └── ...
```

## Quick Start

### Prerequisites

- Rust 1.70+ ([Install Rust](https://rustup.rs/))
- PostgreSQL 13+ with PostGIS 3.0+ (optional, for database features)
- At least 4GB RAM (for direct viewing) or 8GB+ (for database import)

### Option 1: Quick Visualization (No Database)

Visualize OSM data instantly without database setup:

```bash
# Build the viewer
cargo build --release --bin pbf_view

# Generate interactive map
target/release/pbf_view \
    --input sea-260124.osm.pbf \
    --output map.html \
    --features highway,building \
    --bbox 47.6,-122.4,47.7,-122.2

# Open in browser
firefox map.html
```

**Best for:** Quick exploration, small/medium areas, no setup required

### Option 2: Database Import (Production)

For large datasets and complex queries:

```bash
# 1. Setup database (automated)
./setup_database.sh
./configure_postgres_password.sh

# 2. Run migrations
sqlx migrate run

# 3. Import data (handles any size)
./run_db_import.sh your_file.osm.pbf
# or manually:
cargo run --release --bin pbf_import -- --input your_file.osm.pbf

# 4. Query your data
psql -U postgres -d openrustmap -h localhost
```

**Best for:** Large datasets, repeated queries, multi-layer analysis

### Option 3: Vector Tile Server (Production)

Fast WebGL map powered by PostGIS + MapLibre GL JS. Requires data already imported via `pbf_import`.

```bash
# Build
cargo build --release --bin tile_server

# Run (DATABASE_URL env var or --database-url flag)
export DATABASE_URL="postgresql://postgres:yourpass@localhost/openrustmap"
./target/release/tile_server --port 8080

# Optional: apply geom_3857 migration for 40-80% faster tiles on large datasets
sqlx migrate run

# Open browser
firefox http://localhost:8080
```

**Best for:** Large datasets, repeated viewing, best rendering performance

### Option 4: Pathfinding

Find shortest routes on road networks:

```bash
cargo run --release -- \
    -i sea-260124.osm.pbf \
    --start-lat 47.6062 \
    --start-lon -122.3321 \
    --end-lat 47.6205 \
    --end-lon -122.3493
```

**Best for:** Route planning, navigation

---

## Installation

### Automated Database Setup (Fedora)

#### Install PostgreSQL and PostGIS

**Ubuntu/Debian:**
```bash
sudo apt update
sudo apt install -y postgresql postgresql-contrib postgis gdal-bin
```

**Fedora:**
```bash
sudo dnf install postgresql-server postgis gdal
```

#### Create Database

```bash
sudo -u postgres createdb openrustmap
sudo -u postgres psql -d openrustmap -c "CREATE EXTENSION postgis;"
sudo -u postgres psql -d openrustmap -c "CREATE EXTENSION postgis_raster;"
```

#### Configure Environment

Create `.env` file:
```bash
DATABASE_HOST=localhost
DATABASE_PORT=5432
DATABASE_USER=postgres
DATABASE_PASSWORD=your_password
DATABASE_NAME=openrustmap
```

#### Import Data

```bash
# Run migrations first
sqlx migrate run

# Import OSM PBF data
./run_db_import.sh your_file.osm.pbf
# or manually:
cargo run --release --bin pbf_import -- --input your_file.osm.pbf
```

#### Analyze Floods (Coming Soon)

```bash
# Planned feature — not yet implemented
```

---

## 🛠️ Tools & Commands Reference

### Available Binaries

| Tool | Purpose | Database? | Memory Usage |
|------|---------|-----------|--------------|
| `pbf_view` | Direct visualization (Leaflet HTML) | No | Medium |
| `pbf_import` | Database import | Yes | Low |
| `tile_server` | Vector tile server (MapLibre GL JS) | Yes | Low |
| `open_rust_map` | Pathfinding | No | High |

### Quick Command Reference

```bash
# Visualize without database
cargo run --release --bin pbf_view -- \
    -i data.osm.pbf \
    -o map.html \
    --features highway,building \
    --bbox MIN_LAT,MIN_LON,MAX_LAT,MAX_LON

# Import to database
cargo run --release --bin pbf_import -- \
    -i data.osm.pbf

# Find route
cargo run --release -- \
    -i data.osm.pbf \
    --start-lat LAT1 --start-lon LON1 \
    --end-lat LAT2 --end-lon LON2

# Database queries
psql -U postgres -d openrustmap -h localhost -c "
SELECT osm_type, feature_type, COUNT(*)
FROM osm_features
GROUP BY osm_type, feature_type;
"
```

### Setup Scripts

```bash
./setup_database.sh                          # Complete database setup
./configure_postgres_password.sh             # Configure authentication
./check.sh                                   # Verify setup status
./run_db_import.sh <file.osm.pbf>           # Import PBF data (defaults to sea-260124.osm.pbf)
```

### Tool Help

```bash
# Get detailed help for any tool
cargo run --release --bin pbf_view -- --help
cargo run --release --bin pbf_import -- --help
cargo run --release -- --help
```

---

## 📊 When to Use Each Tool

### Use `pbf_view` when:
- ✅ You want to visualize data quickly
- ✅ Dataset is small to medium (city/district)
- ✅ One-time exploration
- ✅ No database setup available

### Use `pbf_import` when:
- ✅ Dataset is large (province/country)
- ✅ You need repeated queries
- ✅ Complex spatial analysis required
- ✅ Multi-layer data integration

### Use `open_rust_map` when:
- ✅ You need route finding
- ✅ Single pathfinding query
- ✅ Testing routing algorithms

---

## Database Schema

### ✅ Implemented Tables

```sql
-- Track imported data sources
CREATE TABLE data_sources (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    source_type TEXT NOT NULL,       -- 'osm', 'dem', 'lidar', 'gisda'
    file_name TEXT NOT NULL,
    file_path TEXT,
    file_format TEXT NOT NULL,       -- 'pbf', 'geotiff', 'las', 'shp'
    import_date TIMESTAMPTZ DEFAULT NOW(),
    bbox GEOMETRY(POLYGON, 4326),
    metadata JSONB,
    row_count BIGINT,
    file_size_bytes BIGINT,
    status TEXT DEFAULT 'imported'
);

-- Store OSM highway ways (imported by pbf_import)
CREATE TABLE osm_features (
    id BIGSERIAL PRIMARY KEY,
    osm_id BIGINT NOT NULL,
    osm_type TEXT NOT NULL,          -- 'way' (nodes are temporary during import only)
    feature_type TEXT,               -- 'highway', 'residential', 'footway', etc.
    geom GEOMETRY(GEOMETRY, 4326),   -- LineString (built after import)
    tags JSONB,
    elevation DOUBLE PRECISION,
    source_id UUID REFERENCES data_sources(id),
    created_at TIMESTAMPTZ DEFAULT NOW(),
    updated_at TIMESTAMPTZ DEFAULT NOW(),
    UNIQUE (osm_id, osm_type)
);

-- Indexes for performance
CREATE INDEX idx_osm_features_geom ON osm_features USING GIST(geom);
CREATE INDEX idx_osm_features_type ON osm_features (feature_type);
CREATE INDEX idx_osm_features_tags ON osm_features USING GIN(tags);
```

### 🔄 Ready for Data (Schema Created)

```sql
-- Elevation raster storage
CREATE TABLE elevation_tiles (
    id BIGSERIAL PRIMARY KEY,
    rast RASTER,
    resolution_meters DOUBLE PRECISION,
    source_id UUID REFERENCES data_sources(id),
    created_at TIMESTAMPTZ DEFAULT NOW()
);

-- Flood analysis results
CREATE TABLE flood_zones (
    id BIGSERIAL PRIMARY KEY,
    scenario_name TEXT,
    water_level_meters DOUBLE PRECISION,
    geom GEOMETRY(MULTIPOLYGON, 4326),
    affected_area_sqm DOUBLE PRECISION,
    created_at TIMESTAMPTZ DEFAULT NOW()
);
```

### Example Queries

```sql
-- Count features by type
SELECT osm_type, feature_type, COUNT(*)
FROM osm_features
GROUP BY osm_type, feature_type
ORDER BY COUNT(*) DESC;

-- Find all highways within 1km of a point
SELECT osm_id, tags->>'name', ST_AsGeoJSON(geom)
FROM osm_features
WHERE feature_type LIKE 'highway%'
AND ST_DWithin(
    geom::geography,
    ST_MakePoint(-122.3321, 47.6062)::geography,
    1000
);

-- Buildings in a polygon
SELECT osm_id, tags->>'name',
       ST_Area(geom::geography) as area_m2
FROM osm_features
WHERE feature_type = 'building'
AND ST_Within(geom, ST_GeomFromText('POLYGON(...)'));

-- Intersection query (roads crossing polygon)
SELECT COUNT(*)
FROM osm_features
WHERE feature_type LIKE 'highway%'
AND ST_Intersects(geom, ST_GeomFromText('POLYGON(...)'));
```

## Development Guide

### Adding New Modules (Following Facade Pattern)

#### Step 1: Create Subdirectory

```bash
mkdir -p base/src/loader
```

#### Step 2: Create Implementation File

```rust
// base/src/loader/osm_loader.rs
pub struct OsmLoader {
    // implementation
}
```

#### Step 3: Create Facade File

```rust
// base/src/loader.rs
pub mod osm_loader;
pub mod dem_loader;
pub mod registry;

// Re-export commonly used items
pub use osm_loader::OsmLoader;
pub use registry::LoaderRegistry;
```

#### Step 4: Export from lib.rs

```rust
// base/src/lib.rs
pub mod loader;
```

### Running Tests

```bash
cargo test --lib
cargo test --package base
```

### Code Quality

```bash
cargo fmt         # Format code
cargo clippy      # Lint code
```

## Implementation Roadmap

### Phase 1: Database Foundation
- [x] Create database migration system
- [ ] Implement connection pooling
- [ ] Create `db.rs` facade with repository pattern
- [ ] Add database configuration to `configuration/`

### Phase 2: Data Loaders
- [ ] Create `loader.rs` facade
- [ ] Implement DataLoader trait
- [ ] Build OsmLoader, DemLoader, GisdaLoader
- [ ] Create LoaderRegistry factory

### Phase 3: Analysis Modules
- [ ] Create `analysis.rs` facade
- [ ] Implement FloodAnalyzer
- [ ] Add raster operations
- [ ] Implement affected area calculations

### Phase 4: CLI Enhancement
- [ ] Restructure CLI with subcommands
- [ ] Add import command
- [ ] Add flood analysis command
- [ ] Add data listing command

### Phase 5: API & Visualization
- [x] Axum web framework integration (`tile_server`)
- [x] Vector tile endpoint (PostGIS ST_AsMVT)
- [x] MapLibre GL JS WebGL frontend
- [ ] Full REST API endpoints
- [ ] Real-time monitoring

## Project Structure

```
OpenRustMap/
├── Cargo.toml                 # Workspace root
├── .cargo/
│   └── config.toml
├── base/                      # Shared library
│   ├── Cargo.toml
│   └── src/
│       ├── lib.rs             # Module exports
│       ├── error.rs           # Error types
│       ├── utils.rs
│       ├── configuration.rs   # Facade
│       ├── configuration/     # Implementations
│       │   └── environment.rs
│       ├── model.rs           # Facade
│       ├── model/             # Implementations
│       │   ├── osm_model.rs
│       │   └── config_model.rs
│       ├── service.rs         # Facade
│       └── service/           # Implementations
│           └── osm_data.rs
└── open_rust_map/             # Main application
    ├── Cargo.toml
    └── src/
        ├── main.rs
        ├── app.rs
        ├── model.rs
        └── bin/
            ├── pbf_view.rs
            ├── pbf_import.rs
            ├── pbf_dump.rs
            └── tile_server.rs
```

## Troubleshooting

### GDAL Not Found (Future)

```bash
# Ubuntu/Debian
sudo apt install libgdal-dev

# Fedora
sudo dnf install gdal-devel
```

### PostGIS Extension Error (Future)

```sql
-- Check if PostGIS is installed
SELECT * FROM pg_available_extensions WHERE name = 'postgis';

-- Create extension
CREATE EXTENSION postgis;
```

## Contributing

1. Fork the repository
2. Create a feature branch
3. Follow the facade pattern for new modules
4. Write tests
5. Submit a pull request

### Code Style

- Follow Rust naming conventions
- Use facade pattern for module organization
- Document public APIs
- Write unit tests for new functionality

## License

MIT License - see LICENSE file for details

---

## 📍 Current Status (v0.1.0)

### ✅ What Works Now

1. **Direct Visualization** (`pbf_view`)
   - Read PBF files without import
   - Generate interactive HTML maps
   - Filter by bbox and feature types
   - Works on any system (no database needed)

2. **Database Storage** (`pbf_import`)
   - Stream PBF data to PostgreSQL
   - Memory-efficient batch processing
   - Handles unlimited dataset sizes
   - Full spatial indexing with PostGIS

3. **Pathfinding** (`open_rust_map`)
   - A* algorithm on road networks
   - Geodesic distance calculations
   - GeoJSON route output

4. **Infrastructure**
   - Automated database setup scripts
   - Schema migrations with sqlx
   - Setup verification tools

5. **Vector Tile Server** (`tile_server`)
   - WebGL rendering via MapLibre GL JS
   - PostGIS ST_AsMVT tile generation
   - Axum HTTP server
   - Auto-fit map to imported data bounds

### 🎯 Next Steps

**Immediate (Week 1-2):**
- [x] Vector tile server (`tile_server` — Axum + PostGIS ST_AsMVT + MapLibre GL JS)
- [ ] DEM/elevation import tool
- [ ] Basic flood analysis

**Short-term (Month 1-2):**
- [ ] Lidar point cloud support
- [ ] Blast radius calculations
- [ ] Multi-layer map interface

**Medium-term (Month 3-6):**
- [ ] REST API
- [ ] Advanced disaster modeling
- [ ] Report generation
- [ ] Performance optimizations

### 🚀 Try It Now

```bash
# Quick start - visualize data in 30 seconds
git clone https://github.com/your-repo/OpenRustMap
cd OpenRustMap
cargo build --release --bin pbf_view
cargo run --release --bin pbf_view -- -i your_file.osm.pbf -o map.html
firefox map.html
```

---

## Resources

### Documentation
- [Rust Book](https://doc.rust-lang.org/book/)
- [PostgreSQL Documentation](https://www.postgresql.org/docs/)
- [PostGIS Manual](https://postgis.net/documentation/)
- [GDAL API](https://gdal.org/api/)

### Data Sources
- [OpenStreetMap Downloads](https://download.geofabrik.de/)
- [SRTM Elevation Data](https://srtm.csi.cgiar.org/)
- [OpenTopography Lidar](https://opentopography.org/)

### Tools
- [QGIS](https://qgis.org/) - Desktop GIS
- [osmium-tool](https://osmcode.org/osmium-tool/) - OSM processing
- [rasterio](https://rasterio.readthedocs.io/) - Raster I/O

---

**Built with Rust, PostgreSQL, and PostGIS**

*Geospatial data platform for environmental analysis and strategic planning*
