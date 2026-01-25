-- Enable PostGIS and PostGIS Raster extensions
CREATE EXTENSION IF NOT EXISTS postgis;
CREATE EXTENSION IF NOT EXISTS postgis_raster;

-- Verify PostGIS version
SELECT PostGIS_version();
