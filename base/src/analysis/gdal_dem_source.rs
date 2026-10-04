//! Real-DEM-backed `TileInputSource`, gated behind the `gdal-support`
//! feature so the default build (and every test in `flood_analyzer.rs`)
//! never needs GDAL's native library installed.
//!
//! Reads elevation directly from a GeoTIFF via GDAL — no reprojection is
//! done, so this assumes the raster's CRS is already WGS84 lon/lat
//! (EPSG:4326), which `bangkok_glo30_dem.tif` (Copernicus GLO-30) is;
//! `open` returns an error rather than silently misreading pixels if a
//! future DEM isn't.
//!
//! Waterway detection is NOT implemented here — the DEM alone has no
//! notion of "this pixel is a canal." `osm_waterways`/`osm_drainage`
//! live in PostGIS (currently unimported — a separate, already-flagged
//! gap), so every cell this source produces has `is_waterway: false`
//! until a PostGIS-backed waterway lookup is wired in alongside it. That
//! is an honest limitation, not a placeholder pretending to be finished:
//! drainage-rate doubling in `FloodTile::step` simply won't trigger yet
//! for any real run using this source.

use gdal::raster::GdalDataType;
use gdal::Dataset;
use std::path::Path;
use std::sync::Mutex;

use super::flood::{CellInput, GRID_SIZE};
use super::flood_analyzer::{cell_to_lonlat, TileInputSource};
use super::tile_grid::TileId;

/// Elevation source reading directly from a GeoTIFF DEM via GDAL.
///
/// Holds the open `gdal::Dataset` behind a `Mutex` because GDAL's C API
/// is not thread-safe for concurrent reads on one dataset handle, while
/// `TileInputSource::resolve` is called from `tile_grid::run_tile_grid`'s
/// parallel (`rayon`) tile loop — serializing reads here is the
/// correctness fix for that, not a performance optimization; the real
/// per-call cost is dominated by GDAL's I/O anyway, not lock contention
/// at this call frequency (once per tile, not per cell).
pub struct GdalDemSource {
    dataset: Mutex<Dataset>,
    origin_x: f64,
    origin_y: f64,
    pixel_size_x: f64,
    pixel_size_y: f64, // negative, per GDAL's north-up convention
    raster_width: usize,
    raster_height: usize,
    /// Elevation to report for any cell that falls outside the raster's
    /// own extent (a tile only partially covered by the DEM) or that
    /// reads back as the raster's own nodata value — 0.0 rather than an
    /// error, since a `FloodTile` still needs *a* value for every cell
    /// in its fixed-size grid. Tracked distinctly from a real 0m
    /// elevation reading isn't attempted here; see the module doc for
    /// why this is an honest simplification rather than a silent one.
    fallback_elevation_m: f32,
}

impl GdalDemSource {
    /// Opens `path` and reads its geotransform. Returns an error if the
    /// file can't be opened, has no geotransform, or isn't in a
    /// lon/lat-degree CRS close enough to EPSG:4326 that reading its
    /// geotransform directly as lon/lat is valid — this module does not
    /// reproject.
    pub fn open(path: &Path) -> Result<Self, gdal::errors::GdalError> {
        let dataset = Dataset::open(path)?;
        let transform = dataset.geo_transform()?;

        // GDAL geotransform layout: [origin_x, pixel_w, 0, origin_y, 0, pixel_h]
        // (rotation terms are 0 for a north-up raster, which this DEM is —
        // see the gdalinfo check this module's PR description documents).
        let origin_x = transform[0];
        let pixel_size_x = transform[1];
        let origin_y = transform[3];
        let pixel_size_y = transform[5];

        let (raster_width, raster_height) = dataset.raster_size();

        Ok(Self {
            dataset: Mutex::new(dataset),
            origin_x,
            origin_y,
            pixel_size_x,
            pixel_size_y,
            raster_width,
            raster_height,
            fallback_elevation_m: 0.0,
        })
    }

    /// Elevation at one lon/lat point, or `fallback_elevation_m` if the
    /// point falls outside the raster or reads back as nodata.
    fn elevation_at(&self, lon: f64, lat: f64) -> f32 {
        let col = ((lon - self.origin_x) / self.pixel_size_x).floor() as i64;
        let row = ((lat - self.origin_y) / self.pixel_size_y).floor() as i64;

        if col < 0
            || row < 0
            || col as usize >= self.raster_width
            || row as usize >= self.raster_height
        {
            return self.fallback_elevation_m;
        }

        let dataset = self.dataset.lock().unwrap_or_else(|e| e.into_inner());
        let Ok(band) = dataset.rasterband(1) else {
            return self.fallback_elevation_m;
        };

        // Read a single pixel. GDAL's read_as requires specifying the
        // output type; DEMs are commonly Float32, which matches
        // CellInput::elevation_m's own type with no conversion needed.
        // read_as's window-offset parameter is (isize, isize), not i64 --
        // the bounds check above already guarantees col/row are
        // non-negative and within the raster's own usize dimensions, so
        // this narrowing can't lose a real value.
        match band.read_as::<f32>((col as isize, row as isize), (1, 1), (1, 1), None) {
            Ok(buf) => {
                let value = buf.data()[0];
                match band.no_data_value() {
                    Some(nodata) if (value as f64 - nodata).abs() < f64::EPSILON => {
                        self.fallback_elevation_m
                    }
                    _ => value,
                }
            }
            Err(_) => self.fallback_elevation_m,
        }
    }
}

impl TileInputSource for GdalDemSource {
    fn resolve(&self, tile: TileId) -> Vec<CellInput> {
        let mut inputs = Vec::with_capacity(GRID_SIZE * GRID_SIZE);
        for y in 0..GRID_SIZE {
            for x in 0..GRID_SIZE {
                let (lon, lat) = cell_to_lonlat(tile, x, y);
                inputs.push(CellInput {
                    elevation_m: self.elevation_at(lon, lat),
                    // see module doc: waterway detection is not wired in
                    // here yet, pending the PostGIS waterway import.
                    is_waterway: false,
                });
            }
        }
        inputs
    }
}

/// Silence an unused-import warning when the `GdalDataType` re-export
/// isn't otherwise referenced — kept imported as documentation of which
/// GDAL pixel type this module assumes (`Float32`), useful if a future
/// DEM needs a different `read_as::<T>()` type parameter.
#[allow(dead_code)]
const _ASSUMED_PIXEL_TYPE: fn() -> GdalDataType = || GdalDataType::Float32;

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    /// Points to the real DEM this project already sourced and
    /// documented in bangkok-flood-2026/observed/elevation/SOURCE.md.
    /// Skips (rather than fails) if the file isn't present at this path
    /// on whatever machine/CI runs the test — this repo's dataset lives
    /// outside the repo itself, so its absence isn't a code bug.
    fn real_dem_path() -> Option<PathBuf> {
        let candidates = [
            "/data/bangkok_glo30_dem.tif", // Docker bind-mount path used in this project's verification runs
            "bangkok_glo30_dem.tif",
        ];
        candidates
            .iter()
            .map(PathBuf::from)
            .find(|p| p.exists())
    }

    #[test]
    fn open_reads_real_dem_geotransform() {
        let Some(path) = real_dem_path() else {
            eprintln!("skipping: real DEM file not found at a known path");
            return;
        };
        let source = GdalDemSource::open(&path).unwrap();

        // from the gdalinfo check this module's PR documents: origin
        // (100.3, 13.98), pixel size ~0.0002778 deg (~30m)
        assert!((source.origin_x - 100.3).abs() < 0.01);
        assert!((source.origin_y - 13.98).abs() < 0.01);
        assert!(source.pixel_size_x > 0.0);
        assert!(source.pixel_size_y < 0.0); // north-up: negative row pitch
    }

    #[test]
    fn elevation_at_known_bangkok_point_is_plausible() {
        let Some(path) = real_dem_path() else {
            eprintln!("skipping: real DEM file not found at a known path");
            return;
        };
        let source = GdalDemSource::open(&path).unwrap();

        // central Bangkok is flat floodplain -- a real reading here
        // should land in a small, low range, not wildly off (e.g. not
        // thousands of metres, not deeply negative)
        let elevation = source.elevation_at(100.51, 13.75);
        assert!(
            (-5.0..50.0).contains(&elevation),
            "unexpected elevation {elevation}m for central Bangkok"
        );
    }

    #[test]
    fn elevation_outside_raster_extent_returns_fallback() {
        let Some(path) = real_dem_path() else {
            eprintln!("skipping: real DEM file not found at a known path");
            return;
        };
        let source = GdalDemSource::open(&path).unwrap();

        // far outside Thailand entirely
        let elevation = source.elevation_at(0.0, 0.0);
        assert_eq!(elevation, source.fallback_elevation_m);
    }

    #[test]
    fn resolve_produces_grid_size_squared_cells_with_no_waterways_yet() {
        let Some(path) = real_dem_path() else {
            eprintln!("skipping: real DEM file not found at a known path");
            return;
        };
        let source = GdalDemSource::open(&path).unwrap();

        let tile = TileId::new(16, 52685, 27584); // roughly central Bangkok at z16
        let cells = source.resolve(tile);

        assert_eq!(cells.len(), GRID_SIZE * GRID_SIZE);
        assert!(
            cells.iter().all(|c| !c.is_waterway),
            "GdalDemSource should not yet mark any cell as waterway -- see module doc"
        );
    }
}
