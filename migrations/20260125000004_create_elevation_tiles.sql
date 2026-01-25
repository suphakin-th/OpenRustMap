-- Elevation Raster Data (for DEMs)
CREATE TABLE elevation_tiles (
    id BIGSERIAL PRIMARY KEY,
    tile_name TEXT NOT NULL,
    rast RASTER,  -- PostGIS raster type for DEM storage
    resolution_meters DOUBLE PRECISION,
    source_id UUID REFERENCES data_sources(id) ON DELETE CASCADE,
    nodata_value DOUBLE PRECISION,
    min_elevation DOUBLE PRECISION,
    max_elevation DOUBLE PRECISION,
    created_at TIMESTAMPTZ DEFAULT NOW()
);

-- Spatial index on raster convex hull
CREATE INDEX idx_elevation_tiles_rast ON elevation_tiles USING GIST(ST_ConvexHull(rast));

-- Index for tile lookup
CREATE INDEX idx_elevation_tiles_name ON elevation_tiles (tile_name);

-- Index for resolution queries
CREATE INDEX idx_elevation_tiles_resolution ON elevation_tiles (resolution_meters);

-- GISDA Administrative Boundaries and Features
CREATE TABLE gisda_features (
    id BIGSERIAL PRIMARY KEY,
    feature_id TEXT,  -- Original ID from GISDA
    layer_name TEXT NOT NULL,  -- Layer from shapefile/geodatabase
    geom GEOMETRY(GEOMETRY, 4326),
    properties JSONB,  -- All attributes from source
    source_id UUID REFERENCES data_sources(id) ON DELETE CASCADE,
    created_at TIMESTAMPTZ DEFAULT NOW()
);

-- Spatial index
CREATE INDEX idx_gisda_features_geom ON gisda_features USING GIST(geom);

-- Indexes for common queries
CREATE INDEX idx_gisda_layer ON gisda_features (layer_name);
CREATE INDEX idx_gisda_feature_id ON gisda_features (feature_id) WHERE feature_id IS NOT NULL;

-- GIN index for properties
CREATE INDEX idx_gisda_properties ON gisda_features USING GIN(properties);

-- Lidar Point Cloud (future phase, prepared structure)
CREATE TABLE lidar_points (
    id BIGSERIAL PRIMARY KEY,
    point_id BIGINT,
    geom GEOMETRY(POINTZ, 4326),  -- 3D point
    classification INTEGER,  -- LAS classification (ground, vegetation, building, etc.)
    intensity INTEGER,
    return_number SMALLINT,
    source_id UUID REFERENCES data_sources(id) ON DELETE CASCADE,
    created_at TIMESTAMPTZ DEFAULT NOW()
);

-- Spatial index for 3D points
CREATE INDEX idx_lidar_points_geom ON lidar_points USING GIST(geom);

-- Index for classification queries
CREATE INDEX idx_lidar_classification ON lidar_points (classification) WHERE classification IS NOT NULL;

-- Index for point_id lookup
CREATE INDEX idx_lidar_point_id ON lidar_points (point_id) WHERE point_id IS NOT NULL;
