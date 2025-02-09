# 🚀 Rust Map Project Documentation

## 📌 Project Overview

This project aims to create a **scalable mapping system** using:

- **Leaflet** for map visualization
- **PostgreSQL + PostGIS** for geospatial data storage
- **Rust** for backend development

---

## 📦 Installation Guide

### 🔹 1. Install PostgreSQL and PostGIS (Ubuntu)

```bash
sudo apt update
sudo apt install -y postgresql postgresql-contrib postgis
```

### 🔹 2. Start and Enable PostgreSQL Service

```bash
sudo systemctl enable postgresql
sudo systemctl start postgresql
```

Check the service status:

```bash
sudo systemctl status postgresql
```

---

## 📌 Database Setup

### 🔹 3. Create a PostgreSQL User (Optional)

```bash
sudo -u postgres createuser --interactive
```

When prompted:

- **Enter name of role to add:** (e.g., `myuser`)
- **Shall the new role be a superuser?** (Type `y` for yes)

If you need to set a password:

```bash
sudo -u postgres psql
```

Inside PostgreSQL shell:

```sql
ALTER USER myuser WITH PASSWORD 'mypassword';
```

Exit PostgreSQL shell using:

```sql
\q
```

### 🔹 4. Create a Database and Enable PostGIS

Login to PostgreSQL:

```bash
sudo -u postgres psql
```

Create a database and enable PostGIS:

```sql
CREATE DATABASE my_map;
\c my_map;
CREATE EXTENSION postgis;
```

Verify PostGIS installation:

```sql
SELECT postgis_version();
```

Expected output:

```
postgis_version  
----------------  
3.2.1  
(1 row)
```

Exit PostgreSQL shell using `\q`.

---

## 🛠️ Rust Integration with PostgreSQL

To use PostgreSQL in Rust, install `sqlx` or `diesel` ORM.

### 🔹 Install `sqlx` (Async PostgreSQL for Rust)

```bash
cargo add sqlx --features postgres,runtime-tokio-native-tls
```

Example connection in Rust:

```rust
use sqlx::{PgPool};

#[tokio::main]
async fn main() -> Result<(), sqlx::Error> {
    let database_url = "postgres://myuser:mypassword@localhost/my_map";
    let pool = PgPool::connect(database_url).await?;
    println!("Connected to PostgreSQL!");
    Ok(())
}
```

### 🔹 Install `diesel` (Rust ORM for PostgreSQL)

```bash
cargo install diesel_cli --no-default-features --features postgres
```

Initialize Diesel:

```bash
diesel setup
```

---

## 🎯 Next Steps

- ✅ Set up Rust backend with PostgreSQL
- ✅ Use Leaflet for map visualization
- ✅ Store and query geospatial data using PostGIS

---

## 💡 Resources

- [PostgreSQL Documentation](https://www.postgresql.org/docs/)
- [PostGIS Documentation](https://postgis.net/documentation/)
- [Leaflet Documentation](https://leafletjs.com/)
- [Rust sqlx](https://github.com/launchbadge/sqlx)
- [Diesel ORM](https://diesel.rs/)

---

🔹 **Happy Coding! 🚀**
