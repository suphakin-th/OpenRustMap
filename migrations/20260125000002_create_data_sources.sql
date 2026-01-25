-- Metadata table for tracking imported data sources
CREATE TABLE data_sources (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    source_type TEXT NOT NULL,  -- 'osm', 'gisda', 'dem', 'lidar'
    file_name TEXT NOT NULL,
    file_path TEXT,
    file_format TEXT NOT NULL,  -- 'pbf', 'shp', 'geotiff', 'las'
    import_date TIMESTAMPTZ DEFAULT NOW(),
    bbox GEOMETRY(POLYGON, 4326),  -- Coverage area
    metadata JSONB,  -- Flexible storage for format-specific data
    row_count BIGINT,
    file_size_bytes BIGINT,
    status TEXT DEFAULT 'imported' CHECK (status IN ('imported', 'processing', 'ready', 'error'))
);

-- Indexes for common queries
CREATE INDEX idx_data_sources_type ON data_sources (source_type);
CREATE INDEX idx_data_sources_format ON data_sources (file_format);
CREATE INDEX idx_data_sources_status ON data_sources (status);
CREATE INDEX idx_data_sources_bbox ON data_sources USING GIST(bbox);
