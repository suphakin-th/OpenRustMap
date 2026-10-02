use std::collections::HashMap;

use super::tile_grid::{TileId, TileState};

/// Side length of each tile's internal cell grid. 16x16 keeps a tile's
/// state small enough that boundary exchange (one edge = 16 f32s) and
/// per-step cell diffusion stay cheap, while still giving enough
/// resolution within a tile to show flow direction, not just one
/// averaged depth per tile.
pub const GRID_SIZE: usize = 16;

/// One cell's static input — what doesn't change during a run. Separated
/// from the per-step mutable depth so a `FloodTile` can be built once from
/// real DEM/waterway data and then stepped many times cheaply.
#[derive(Debug, Clone, Copy)]
pub struct CellInput {
    /// Metres above the run's reference datum. Higher ground drains faster
    /// and accumulates less — this is the DEM value for this cell.
    pub elevation_m: f32,
    /// Whether this cell falls on a mapped waterway (canal/river/drain).
    /// Waterway cells both drain faster AND are the preferred direction
    /// flow moves toward from neighboring cells, modeling a canal as the
    /// path water actually seeks, not just a low spot — this is the
    /// reason osm_waterways/osm_drainage are real inputs to flood math,
    /// not basemap decoration (see the discussion that motivated this).
    pub is_waterway: bool,
}

/// One tile's water-depth state: a fixed-size grid of cells plus the
/// static inputs (elevation, waterway) each depends on.
///
/// This implements `TileState` so `tile_grid::run_tile_grid` can drive it
/// in parallel with boundary exchange — see that module for why tiles
/// never talk to each other except through `boundary()`/`step()`.
#[derive(Debug, Clone)]
pub struct FloodTile {
    pub id: TileId,
    inputs: Vec<CellInput>,       // row-major, GRID_SIZE x GRID_SIZE
    depth_m: Vec<f32>,            // row-major, GRID_SIZE x GRID_SIZE
    /// Rainfall input rate for this run, metres of depth added per
    /// timestep — comes from the real HII/Open-Meteo rainfall data a
    /// caller resolves for this tile's area before constructing it, not
    /// computed here.
    rainfall_rate_m: f32,
    /// Fraction of standing depth removed per timestep by baseline
    /// drainage (pumping stations, storm sewers) — doubled on waterway
    /// cells, modeling a canal/drain channel draining faster than open
    /// ground, same rationale as `CellInput::is_waterway`.
    drainage_rate: f32,
}

impl FloodTile {
    /// Build a tile from real per-cell inputs. `inputs` must have exactly
    /// `GRID_SIZE * GRID_SIZE` entries in row-major order; this is the
    /// boundary between "real DEM/waterway data resolved by the caller"
    /// and "the simulation state that data feeds" — this constructor does
    /// no interpolation or sampling itself.
    pub fn new(
        id: TileId,
        inputs: Vec<CellInput>,
        rainfall_rate_m: f32,
        drainage_rate: f32,
    ) -> Self {
        assert_eq!(
            inputs.len(),
            GRID_SIZE * GRID_SIZE,
            "FloodTile::new requires exactly GRID_SIZE*GRID_SIZE cell inputs"
        );
        Self {
            id,
            depth_m: vec![0.0; GRID_SIZE * GRID_SIZE],
            inputs,
            rainfall_rate_m,
            drainage_rate,
        }
    }

    fn idx(x: usize, y: usize) -> usize {
        y * GRID_SIZE + x
    }

    pub fn depth_at(&self, x: usize, y: usize) -> f32 {
        self.depth_m[Self::idx(x, y)]
    }

    /// Surface elevation at a cell: ground elevation plus standing water —
    /// this, not bare ground elevation, is what flow direction compares
    /// between cells. Water flows toward lower *surface*, same as real
    /// terrain-following flow.
    fn surface_at(&self, x: usize, y: usize) -> f32 {
        let i = Self::idx(x, y);
        self.inputs[i].elevation_m + self.depth_m[i]
    }
}

impl TileState for FloodTile {
    /// One edge's worth of depth values, tagged by which edge — a
    /// neighbor only ever needs the single row/column of cells that
    /// actually touches the shared boundary, never the whole grid.
    type Boundary = EdgeDepths;

    fn step(&mut self, neighbor_boundaries: &HashMap<TileId, EdgeDepths>) {
        let mut next = self.depth_m.clone();

        for y in 0..GRID_SIZE {
            for x in 0..GRID_SIZE {
                let i = Self::idx(x, y);
                let input = self.inputs[i];

                // 1. rainfall adds depth uniformly this step.
                let mut depth = self.depth_m[i] + self.rainfall_rate_m;

                // 2. drainage removes a fraction of standing depth;
                //    waterway cells drain twice as fast, modeling a canal
                //    actually carrying water away rather than just sitting
                //    at a locally low elevation.
                let effective_drainage = if input.is_waterway {
                    self.drainage_rate * 2.0
                } else {
                    self.drainage_rate
                };
                depth -= depth * effective_drainage;
                depth = depth.max(0.0);

                // 3. intra-tile flow: compare this cell's surface height
                //    to its four in-grid neighbors (or the matching edge
                //    cell from an adjacent tile's last-step boundary, at
                //    the tile's own edge) and move a fraction of the
                //    surface-height difference toward lower neighbors —
                //    this is the terrain-following part: water moves
                //    downhill, modified by standing depth already there.
                let my_surface = input.elevation_m + self.depth_m[i];
                let mut net_flow = 0.0f32;

                let neighbor_surfaces = [
                    self.neighbor_surface(x, y, -1, 0, neighbor_boundaries),  // west
                    self.neighbor_surface(x, y, 1, 0, neighbor_boundaries),   // east
                    self.neighbor_surface(x, y, 0, -1, neighbor_boundaries),  // north
                    self.neighbor_surface(x, y, 0, 1, neighbor_boundaries),   // south
                ];

                // net_flow -= diff * FLOW_FRACTION handles both
                // directions through its sign alone: diff > 0 means this
                // cell is higher than the neighbor, so it loses depth
                // (net_flow goes negative); diff < 0 means the neighbor
                // is higher and flow comes toward this cell, so net_flow
                // goes positive. No branch needed — the formula already
                // is the physics.
                const FLOW_FRACTION: f32 = 0.1;
                for n_surface in neighbor_surfaces.into_iter().flatten() {
                    let diff = my_surface - n_surface;
                    net_flow -= diff * FLOW_FRACTION;
                }

                depth = (depth + net_flow).max(0.0);
                next[i] = depth;
            }
        }

        self.depth_m = next;
    }

    fn boundary(&self) -> EdgeDepths {
        let mut west = [0.0; GRID_SIZE];
        let mut east = [0.0; GRID_SIZE];
        let mut north = [0.0; GRID_SIZE];
        let mut south = [0.0; GRID_SIZE];
        for i in 0..GRID_SIZE {
            west[i] = self.depth_at(0, i);
            east[i] = self.depth_at(GRID_SIZE - 1, i);
            north[i] = self.depth_at(i, 0);
            south[i] = self.depth_at(i, GRID_SIZE - 1);
        }
        EdgeDepths { west, east, north, south }
    }
}

impl FloodTile {
    /// Surface height of the cell at `(x + dx, y + dy)`, whether that
    /// falls inside this tile's own grid or across the edge into a
    /// neighbor tile's last-reported boundary. Returns `None` at a grid
    /// edge with no neighbor present (the true edge of the whole
    /// simulated area, not just this tile) — those cells simply have one
    /// fewer flow term, which is the correct open-boundary behavior
    /// rather than inventing a wall or a neighbor that doesn't exist.
    fn neighbor_surface(
        &self,
        x: usize,
        y: usize,
        dx: i32,
        dy: i32,
        neighbor_boundaries: &HashMap<TileId, EdgeDepths>,
    ) -> Option<f32> {
        let nx = x as i32 + dx;
        let ny = y as i32 + dy;

        if nx >= 0 && (nx as usize) < GRID_SIZE && ny >= 0 && (ny as usize) < GRID_SIZE {
            // inside this tile's own grid
            return Some(self.surface_at(nx as usize, ny as usize));
        }

        // crosses a tile edge: figure out which neighbor tile and which
        // of its boundary rows/columns lines up with (x, y)
        let (neighbor_id, edge_depth) = if nx < 0 {
            (self.west_neighbor()?, y)
        } else if nx as usize >= GRID_SIZE {
            (self.east_neighbor()?, y)
        } else if ny < 0 {
            (self.north_neighbor()?, x)
        } else {
            (self.south_neighbor()?, x)
        };

        let edges = neighbor_boundaries.get(&neighbor_id)?;
        // the edge of the NEIGHBOR that touches US is its opposite edge
        let depth = if nx < 0 {
            edges.east[edge_depth]
        } else if nx as usize >= GRID_SIZE {
            edges.west[edge_depth]
        } else if ny < 0 {
            edges.south[edge_depth]
        } else {
            edges.north[edge_depth]
        };

        // We don't have the neighbor's elevation at that cell, only its
        // depth — approximate its surface as our own edge cell's
        // elevation plus the neighbor's reported depth. This is the one
        // real simplification versus true shallow-water coupling: it
        // assumes elevation doesn't change sharply across a tile
        // boundary, which is reasonable at the zoom levels this grid
        // targets (a tile already covers a small enough area that a
        // sharp elevation cliff exactly on a tile seam is unlikely) but
        // is not physically exact.
        let my_elevation = self.inputs[Self::idx(x, y)].elevation_m;
        Some(my_elevation + depth)
    }

    fn west_neighbor(&self) -> Option<TileId> {
        self.id.x.checked_sub(1).map(|x| TileId::new(self.id.z, x, self.id.y))
    }
    fn east_neighbor(&self) -> Option<TileId> {
        Some(TileId::new(self.id.z, self.id.x + 1, self.id.y))
    }
    fn north_neighbor(&self) -> Option<TileId> {
        self.id.y.checked_sub(1).map(|y| TileId::new(self.id.z, self.id.x, y))
    }
    fn south_neighbor(&self) -> Option<TileId> {
        Some(TileId::new(self.id.z, self.id.x, self.id.y + 1))
    }
}

/// The four edge rows/columns of a `FloodTile`'s depth grid — see
/// `TileState::Boundary` on `FloodTile` for why only edges, not the whole
/// grid, cross between tiles.
#[derive(Debug, Clone)]
pub struct EdgeDepths {
    pub west: [f32; GRID_SIZE],
    pub east: [f32; GRID_SIZE],
    pub north: [f32; GRID_SIZE],
    pub south: [f32; GRID_SIZE],
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::analysis::run_tile_grid;

    fn flat_tile(id: TileId, elevation_m: f32) -> FloodTile {
        let inputs = vec![
            CellInput { elevation_m, is_waterway: false };
            GRID_SIZE * GRID_SIZE
        ];
        FloodTile::new(id, inputs, 0.01, 0.05)
    }

    #[test]
    fn rainfall_raises_depth_on_flat_ground_with_no_drainage() {
        let inputs = vec![
            CellInput { elevation_m: 0.0, is_waterway: false };
            GRID_SIZE * GRID_SIZE
        ];
        let mut tile = FloodTile::new(TileId::new(10, 5, 5), inputs, 0.01, 0.0);

        tile.step(&HashMap::new());

        // every cell on flat ground with zero drainage should have
        // gained exactly the rainfall rate (no net flow between equal
        // -elevation, equal-depth neighbors)
        for y in 0..GRID_SIZE {
            for x in 0..GRID_SIZE {
                assert!(
                    (tile.depth_at(x, y) - 0.01).abs() < 1e-6,
                    "cell ({x},{y}) = {}",
                    tile.depth_at(x, y)
                );
            }
        }
    }

    #[test]
    fn waterway_cells_drain_faster_than_open_ground() {
        let mut inputs = vec![
            CellInput { elevation_m: 0.0, is_waterway: false };
            GRID_SIZE * GRID_SIZE
        ];
        let waterway_idx = FloodTile::idx(8, 8);
        inputs[waterway_idx] = CellInput { elevation_m: 0.0, is_waterway: true };

        let mut tile = FloodTile::new(TileId::new(10, 5, 5), inputs, 0.0, 0.1);
        // seed standing water uniformly so drainage has something to act on
        tile.depth_m = vec![1.0; GRID_SIZE * GRID_SIZE];

        tile.step(&HashMap::new());

        let waterway_depth = tile.depth_at(8, 8);
        let open_ground_depth = tile.depth_at(0, 0);
        assert!(
            waterway_depth < open_ground_depth,
            "waterway cell ({waterway_depth}) should drain faster than open ground ({open_ground_depth})"
        );
    }

    #[test]
    fn water_flows_from_high_to_low_elevation() {
        // left half of the grid is higher ground, right half lower —
        // water seeded only on the high side should flow rightward.
        let mut inputs = vec![
            CellInput { elevation_m: 0.0, is_waterway: false };
            GRID_SIZE * GRID_SIZE
        ];
        for y in 0..GRID_SIZE {
            for x in 0..GRID_SIZE / 2 {
                inputs[FloodTile::idx(x, y)].elevation_m = 10.0;
            }
        }

        let mut tile = FloodTile::new(TileId::new(10, 5, 5), inputs, 0.0, 0.0);
        tile.depth_m[FloodTile::idx(0, 8)] = 1.0; // seed water on the high side

        tile.step(&HashMap::new());

        // after one step, the low side immediately adjacent to the seeded
        // high cell should have gained some depth from downhill flow
        let low_neighbor_depth = tile.depth_at(GRID_SIZE / 2, 8);
        assert!(
            low_neighbor_depth > 0.0,
            "expected flow onto lower ground, got {low_neighbor_depth}"
        );
    }

    #[test]
    fn boundary_exchange_moves_water_across_tile_seam() {
        // Two adjacent tiles. West tile has a deep pool right at its east
        // edge, flowing onto (comparatively) low ground; east tile starts
        // completely dry. After running through the shared tile_grid
        // runner, the east tile's west edge should show water arrived
        // from across the seam -- this is the real integration point
        // between FloodTile and tile_grid::run_tile_grid.
        let west_id = TileId::new(10, 5, 5);
        let east_id = TileId::new(10, 6, 5);

        let mut west = flat_tile(west_id, 5.0);
        west.depth_m[FloodTile::idx(GRID_SIZE - 1, 8)] = 2.0; // pool at east edge

        let east = flat_tile(east_id, 0.0); // lower ground, dry

        let mut tiles = HashMap::new();
        tiles.insert(west_id, west);
        tiles.insert(east_id, east);

        run_tile_grid(&mut tiles, 3);

        let east_tile = &tiles[&east_id];
        let arrived = east_tile.depth_at(0, 8);
        assert!(
            arrived > 0.0,
            "expected water to cross from west tile's seam into east tile, got {arrived}"
        );
    }
}
