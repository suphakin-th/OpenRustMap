#!/bin/bash
# OpenRustMap Database Setup Script
# Run this script to set up PostgreSQL with PostGIS

set -e  # Exit on error

echo "=== OpenRustMap Database Setup ==="
echo ""

# Colors for output
GREEN='\033[0;32m'
BLUE='\033[0;34m'
NC='\033[0m' # No Color

# Step 1: Install PostgreSQL and PostGIS
echo -e "${BLUE}[1/8] Installing PostgreSQL and PostGIS...${NC}"
sudo dnf install -y postgresql-server postgresql-contrib postgis

# Step 2: Initialize PostgreSQL
echo -e "${BLUE}[2/8] Initializing PostgreSQL...${NC}"
if [ ! -f /var/lib/pgsql/data/PG_VERSION ]; then
    sudo postgresql-setup --initdb
else
    echo "PostgreSQL already initialized, skipping..."
fi

# Step 3: Start and enable PostgreSQL
echo -e "${BLUE}[3/8] Starting PostgreSQL service...${NC}"
sudo systemctl enable postgresql
sudo systemctl start postgresql

# Wait for PostgreSQL to be ready
sleep 2

# Step 4: Create database
echo -e "${BLUE}[4/8] Creating openrustmap database...${NC}"
sudo -u postgres psql -tc "SELECT 1 FROM pg_database WHERE datname = 'openrustmap'" | grep -q 1 || \
    sudo -u postgres createdb openrustmap

# Step 5: Enable PostGIS extensions
echo -e "${BLUE}[5/8] Enabling PostGIS extensions...${NC}"
sudo -u postgres psql -d openrustmap -c "CREATE EXTENSION IF NOT EXISTS postgis;"
sudo -u postgres psql -d openrustmap -c "CREATE EXTENSION IF NOT EXISTS postgis_raster;"

# Step 6: Verify PostGIS
echo -e "${BLUE}[6/8] Verifying PostGIS installation...${NC}"
sudo -u postgres psql -d openrustmap -c "SELECT PostGIS_version();"

# Step 7: Configure PostgreSQL for local connections (if needed)
echo -e "${BLUE}[7/8] Configuring PostgreSQL authentication...${NC}"
PG_HBA="/var/lib/pgsql/data/pg_hba.conf"
if ! sudo grep -q "local.*openrustmap.*trust" "$PG_HBA"; then
    echo "Adding trust authentication for local connections..."
    sudo sed -i '/^local.*all.*postgres.*peer/a local   openrustmap     postgres                                trust' "$PG_HBA"
    sudo systemctl reload postgresql
fi

# Step 8: Create .env file
echo -e "${BLUE}[8/8] Creating .env configuration file...${NC}"
cd /home/babylvoob/Project/OpenRustMap
if [ ! -f .env ]; then
    cat > .env << 'EOF'
DATABASE_HOST=localhost
DATABASE_PORT=5432
DATABASE_USER=postgres
DATABASE_PASSWORD=test
DATABASE_NAME=openrustmap
RUST_LOG=info
EOF
    echo ".env file created"
else
    echo ".env file already exists, skipping..."
fi

echo ""
echo -e "${GREEN}✓ Database setup complete!${NC}"
echo ""
echo "Next steps:"
echo "  1. Install sqlx-cli: cargo install sqlx-cli --no-default-features --features postgres"
echo "  2. Run migrations: sqlx migrate run"
echo "  3. Test connection: psql -U postgres -d openrustmap -c 'SELECT PostGIS_version();'"
echo ""
