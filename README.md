# OpenRustMap: Extensible Geospatial Data Platform

## Project Vision

OpenRustMap is a production-ready geospatial data platform for environmental analysis and strategic planning. Built with Rust, PostgreSQL/PostGIS, and designed with extensibility at its core.

### Core Use Cases

- **Flood Hazard Mapping** - Analyze flood scenarios using DEM elevation data and OSM features
- **Environmental Analysis** - GISDA data integration for administrative boundaries and planning
- **Carbon Tracking** - Forest mapping and biomass estimation using Lidar data (planned)
- **Strategic Planning** - Multi-source data integration for decision-making

## Key Features

### Current Capabilities
- OSM PBF file parsing and graph-based routing (A* pathfinding)
- Modular architecture with facade design pattern
- Async runtime with Tokio
- Comprehensive error handling with Snafu
- Configuration management for multiple environments

### Planned Capabilities
- Multi-format data import (OSM PBF, GISDA shapefiles, GeoTIFF DEMs, Lidar)
- PostgreSQL + PostGIS storage with spatial indexing
- Flood extent calculation and impact analysis
- Extensible data loader system with trait-based polymorphism
- Functional data processing pipelines
- Web API for remote access
- Interactive Leaflet visualization

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
- PostgreSQL 13+ with PostGIS 3.0+ (for database features)
- GDAL 3.0+ (for geospatial data processing)
- At least 8GB RAM

### Current Usage

#### Build the Project

```bash
cd OpenRustMap
cargo build --release
```

#### Run Pathfinding

```bash
./target/release/open_rust_map <pbf_file> <start_lat> <start_lon> <end_lat> <end_lon>
```

### Future Usage (After Implementation)

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
# Import OSM data
openrustmap import --file data/thailand.osm.pbf

# Import elevation data
openrustmap import --file data/dem.tif

# Import GISDA shapefiles
openrustmap import --file data/provinces.shp
```

#### Analyze Floods

```bash
openrustmap flood analyze \
  --water-level 5.0 \
  --bbox "100.0,13.0,101.0,14.0" \
  --output flood_5m.geojson
```

## Database Schema (Planned)

### Core Tables

```sql
-- Track imported data sources
CREATE TABLE data_sources (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    source_type TEXT NOT NULL,
    file_format TEXT NOT NULL,
    import_date TIMESTAMPTZ DEFAULT NOW(),
    metadata JSONB
);

-- Store OSM features
CREATE TABLE osm_features (
    id BIGSERIAL PRIMARY KEY,
    osm_id BIGINT NOT NULL,
    feature_type TEXT,
    geom GEOMETRY(GEOMETRY, 4326),
    tags JSONB
);

-- Store elevation rasters
CREATE TABLE elevation_tiles (
    id BIGSERIAL PRIMARY KEY,
    rast RASTER,
    resolution_meters DOUBLE PRECISION
);

-- Store flood analysis results
CREATE TABLE flood_zones (
    id BIGSERIAL PRIMARY KEY,
    scenario_name TEXT,
    water_level_meters DOUBLE PRECISION,
    geom GEOMETRY(MULTIPOLYGON, 4326),
    affected_area_sqm DOUBLE PRECISION
);
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
- [ ] Create database migration system
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

### Phase 5: API & Visualization (Future)
- [ ] Axum web framework integration
- [ ] REST API endpoints
- [ ] Leaflet frontend
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
        └── model.rs
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
