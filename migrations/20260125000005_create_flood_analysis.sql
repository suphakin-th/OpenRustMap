-- Flood Analysis Results
CREATE TABLE flood_zones (
    id BIGSERIAL PRIMARY KEY,
    scenario_name TEXT NOT NULL,
    water_level_meters DOUBLE PRECISION NOT NULL,
    affected_area_sqm DOUBLE PRECISION,
    affected_population INTEGER,
    flood_depth_category TEXT CHECK (flood_depth_category IN ('low', 'medium', 'high')),
    geom GEOMETRY(MULTIPOLYGON, 4326),
    analysis_date TIMESTAMPTZ DEFAULT NOW(),
    metadata JSONB
);

-- Spatial index
CREATE INDEX idx_flood_zones_geom ON flood_zones USING GIST(geom);

-- Indexes for queries
CREATE INDEX idx_flood_zones_scenario ON flood_zones (scenario_name);
CREATE INDEX idx_flood_zones_level ON flood_zones (water_level_meters);
CREATE INDEX idx_flood_zones_category ON flood_zones (flood_depth_category) WHERE flood_depth_category IS NOT NULL;
CREATE INDEX idx_flood_zones_date ON flood_zones (analysis_date DESC);

-- Carbon Storage Estimates (future phase)
CREATE TABLE carbon_estimates (
    id BIGSERIAL PRIMARY KEY,
    forest_area_id UUID,
    geom GEOMETRY(POLYGON, 4326),
    tree_coverage_percent DOUBLE PRECISION CHECK (tree_coverage_percent BETWEEN 0 AND 100),
    estimated_carbon_tons DOUBLE PRECISION CHECK (estimated_carbon_tons >= 0),
    biomass_density DOUBLE PRECISION,
    calculation_method TEXT,
    analysis_date TIMESTAMPTZ DEFAULT NOW(),
    metadata JSONB
);

-- Spatial index
CREATE INDEX idx_carbon_estimates_geom ON carbon_estimates USING GIST(geom);

-- Indexes for queries
CREATE INDEX idx_carbon_forest_id ON carbon_estimates (forest_area_id) WHERE forest_area_id IS NOT NULL;
CREATE INDEX idx_carbon_date ON carbon_estimates (analysis_date DESC);
CREATE INDEX idx_carbon_method ON carbon_estimates (calculation_method) WHERE calculation_method IS NOT NULL;
