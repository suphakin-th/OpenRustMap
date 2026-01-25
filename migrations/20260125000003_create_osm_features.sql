-- OSM Features (unified table for all OSM geometries)
CREATE TABLE osm_features (
    id BIGSERIAL PRIMARY KEY,
    osm_id BIGINT NOT NULL,
    osm_type TEXT NOT NULL CHECK (osm_type IN ('node', 'way', 'relation')),
    feature_type TEXT,  -- 'building', 'highway', 'waterway', etc.
    geom GEOMETRY(GEOMETRY, 4326),  -- Can be Point, LineString, Polygon
    tags JSONB,
    elevation DOUBLE PRECISION,  -- Extracted from tags if available
    source_id UUID REFERENCES data_sources(id) ON DELETE CASCADE,
    created_at TIMESTAMPTZ DEFAULT NOW()
);

-- Spatial index for geometry queries (most important!)
CREATE INDEX idx_osm_features_geom ON osm_features USING GIST(geom);

-- Indexes for common queries
CREATE INDEX idx_osm_features_osm_id ON osm_features (osm_id);
CREATE INDEX idx_osm_features_type ON osm_features (feature_type) WHERE feature_type IS NOT NULL;
CREATE INDEX idx_osm_features_osm_type ON osm_features (osm_type);

-- GIN index for JSONB tag searches
CREATE INDEX idx_osm_features_tags ON osm_features USING GIN(tags);

-- Index for elevation queries
CREATE INDEX idx_osm_features_elevation ON osm_features (elevation) WHERE elevation IS NOT NULL;

-- Composite index for common spatial + type queries
CREATE INDEX idx_osm_features_geom_type ON osm_features USING GIST(geom) WHERE feature_type IS NOT NULL;
