-- Add updated_at column used by upsert operations in pbf_import
ALTER TABLE osm_features ADD COLUMN updated_at TIMESTAMPTZ DEFAULT NOW();

-- Add unique constraint required by ON CONFLICT (osm_id, osm_type) in pbf_import
ALTER TABLE osm_features ADD CONSTRAINT osm_features_osm_id_osm_type_unique UNIQUE (osm_id, osm_type);
