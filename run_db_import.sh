#!/bin/bash
# Database-backed PBF import - No memory limits!
# Usage: ./run_db_import.sh <file.osm.pbf>

set -e

PBF_FILE="${1:-sea-260124.osm.pbf}"

if [ ! -f "$PBF_FILE" ]; then
    echo "Error: PBF file '$PBF_FILE' not found"
    echo "Usage: ./run_db_import.sh <file.osm.pbf>"
    exit 1
fi

echo "=== OpenRustMap Database Import ==="
echo ""

GREEN='\033[0;32m'
BLUE='\033[0;34m'
YELLOW='\033[1;33m'
NC='\033[0m'

# Check database connection
echo -e "${BLUE}Checking database connection...${NC}"
if ! PGPASSWORD=test psql -U postgres -d openrustmap -h localhost -c "SELECT 1;" &>/dev/null; then
    echo -e "${YELLOW}✗ Cannot connect to database${NC}"
    echo "Please run: ./configure_postgres_password.sh"
    exit 1
fi
echo -e "${GREEN}✓ Database connection OK${NC}"

echo ""
echo -e "${GREEN}Starting database import...${NC}"
echo "  File: $PBF_FILE"
echo ""
echo "This will:"
echo "  - Stream PBF data directly to PostgreSQL"
echo "  - Process in batches (100000 rows at a time)"
echo "  - Use minimal RAM (only batch size in memory)"
echo "  - Can handle datasets of ANY size"
echo ""

# Source .env file
if [ -f .env ]; then
    export $(cat .env | grep -v '^#' | xargs)
fi

# Run the import
time cargo run --release --bin pbf_import -- \
    --input "$PBF_FILE"

echo ""
echo -e "${GREEN}=== Import Complete! ===${NC}"
echo ""
echo "Query your data:"
echo "  psql -U postgres -d openrustmap -h localhost"
echo ""
echo "Example queries:"
echo "  -- Count imported features"
echo "  SELECT osm_type, feature_type, COUNT(*) FROM osm_features GROUP BY osm_type, feature_type;"
echo ""
echo "  -- Find highways near a point"
echo "  SELECT osm_id, feature_type, ST_AsText(geom)"
echo "  FROM osm_features"
echo "  WHERE feature_type LIKE 'highway%'"
echo "  AND ST_DWithin(geom::geography, ST_MakePoint(-122.3321, 47.6062)::geography, 1000);"
echo ""
