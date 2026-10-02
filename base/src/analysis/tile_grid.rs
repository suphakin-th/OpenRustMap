use rayon::prelude::*;
use std::collections::HashMap;

/// Address of one tile in a Web Mercator slippy-map grid (z/x/y).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct TileId {
    pub z: u8,
    pub x: u32,
    pub y: u32,
}

impl TileId {
    pub fn new(z: u8, x: u32, y: u32) -> Self {
        Self { z, x, y }
    }

    /// The four edge-adjacent tiles (north/south/east/west), skipping any
    /// that would fall outside the valid 0..2^z range for this zoom level.
    /// Diagonal neighbors are deliberately excluded — a shallow-water or
    /// fire-spread step only exchanges flux across a shared edge, never
    /// through a shared corner alone.
    pub fn edge_neighbors(&self) -> Vec<TileId> {
        let max = 1u32 << self.z;
        let mut out = Vec::with_capacity(4);
        if self.y > 0 {
            out.push(TileId::new(self.z, self.x, self.y - 1));
        }
        if self.y + 1 < max {
            out.push(TileId::new(self.z, self.x, self.y + 1));
        }
        if self.x > 0 {
            out.push(TileId::new(self.z, self.x - 1, self.y));
        }
        if self.x + 1 < max {
            out.push(TileId::new(self.z, self.x + 1, self.y));
        }
        out
    }
}

/// What one tile needs from its neighbors to take its next step, and what
/// it hands back for them to use on theirs — e.g. the water-depth values
/// along the shared edge for a shallow-water step, or the fuel/heat state
/// along the shared edge for a fire-spread step. Left generic (`State`)
/// rather than hardcoded to flood so wildfire's `TileGrid` analyzer can
/// reuse this same runner with its own per-tile state type.
pub trait TileState: Send {
    /// The piece of this tile's state that a neighbor needs to see across
    /// their shared edge — never the whole tile's state, only the boundary
    /// slice, which is what keeps the exchange cheap.
    type Boundary: Clone + Send + Sync;

    /// Advance this tile by one timestep using this tile's own state plus
    /// whatever boundary values its neighbors handed over after *their*
    /// previous step. `neighbor_boundaries` only contains entries for
    /// neighbors that exist (edge tiles at the grid's edge have fewer).
    fn step(&mut self, neighbor_boundaries: &HashMap<TileId, Self::Boundary>);

    /// Extract this tile's current boundary slice, to hand to neighbors
    /// before they take their own next step.
    fn boundary(&self) -> Self::Boundary;
}

/// Runs a `TileGrid`-model hazard simulation: every tile takes one step in
/// parallel (via rayon — within a single process, deliberately never across
/// a network socket, since the boundary exchange below happens every
/// timestep and network latency would dominate real compute at that
/// frequency), then every tile's new boundary is collected and handed to
/// its neighbors before the next step begins. The barrier between "all
/// tiles step" and "all tiles see new neighbor state" is what makes this
/// correct — a tile must never see a neighbor's *in-progress* step, only
/// its fully-settled state from the step that just completed.
///
/// `tiles` is mutated in place; after `run` returns, every tile holds its
/// state as of `timesteps` steps forward. Call `TileState::boundary` per
/// tile afterward (or snapshot it after each step by extending this loop)
/// to pull out whatever a `HazardAnalyzer` needs for its `ScenarioResult`
/// or per-point `WaterLevelSample`s.
pub fn run_tile_grid<S: TileState>(tiles: &mut HashMap<TileId, S>, timesteps: u32) {
    for _ in 0..timesteps {
        // Boundary exchange uses each tile's state as of the END of the
        // previous step — collect it before anyone steps again, so step()
        // below never reads a neighbor's half-updated state.
        let boundaries: HashMap<TileId, S::Boundary> = tiles
            .iter()
            .map(|(id, tile)| (*id, tile.boundary()))
            .collect();

        tiles.par_iter_mut().for_each(|(id, tile)| {
            let neighbor_boundaries: HashMap<TileId, S::Boundary> = id
                .edge_neighbors()
                .into_iter()
                .filter_map(|n| boundaries.get(&n).map(|b| (n, b.clone())))
                .collect();
            tile.step(&neighbor_boundaries);
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn edge_neighbors_excludes_out_of_range() {
        // top-left corner tile at z=4 has only two valid edge neighbors
        let corner = TileId::new(4, 0, 0);
        let neighbors = corner.edge_neighbors();
        assert_eq!(neighbors.len(), 2);
        assert!(neighbors.contains(&TileId::new(4, 1, 0)));
        assert!(neighbors.contains(&TileId::new(4, 0, 1)));
    }

    #[test]
    fn edge_neighbors_interior_tile_has_four() {
        let interior = TileId::new(10, 500, 500);
        assert_eq!(interior.edge_neighbors().len(), 4);
    }

    /// Minimal TileState: each tile just sums its neighbors' boundary
    /// values into its own, like a trivial diffusion — enough to prove
    /// the runner actually propagates state across the grid rather than
    /// leaving every tile isolated.
    #[derive(Clone)]
    struct SumTile {
        value: f64,
    }

    impl TileState for SumTile {
        type Boundary = f64;

        fn step(&mut self, neighbor_boundaries: &HashMap<TileId, f64>) {
            self.value += neighbor_boundaries.values().sum::<f64>();
        }

        fn boundary(&self) -> f64 {
            self.value
        }
    }

    #[test]
    fn run_tile_grid_propagates_neighbor_state() {
        // A 2x2 grid at z=1. Seed only (0,0) with a nonzero value; after
        // one step its edge neighbors (1,0) and (0,1) must have picked up
        // that value, and the diagonal (1,1) — not an edge neighbor — must
        // still be zero, proving the runner respects edge-only adjacency.
        let mut tiles: HashMap<TileId, SumTile> = HashMap::new();
        tiles.insert(TileId::new(1, 0, 0), SumTile { value: 10.0 });
        tiles.insert(TileId::new(1, 1, 0), SumTile { value: 0.0 });
        tiles.insert(TileId::new(1, 0, 1), SumTile { value: 0.0 });
        tiles.insert(TileId::new(1, 1, 1), SumTile { value: 0.0 });

        run_tile_grid(&mut tiles, 1);

        assert_eq!(tiles[&TileId::new(1, 1, 0)].value, 10.0);
        assert_eq!(tiles[&TileId::new(1, 0, 1)].value, 10.0);
        assert_eq!(tiles[&TileId::new(1, 1, 1)].value, 0.0);
    }

    #[test]
    fn run_tile_grid_multiple_steps_keeps_propagating() {
        let mut tiles: HashMap<TileId, SumTile> = HashMap::new();
        tiles.insert(TileId::new(1, 0, 0), SumTile { value: 1.0 });
        tiles.insert(TileId::new(1, 1, 0), SumTile { value: 0.0 });
        tiles.insert(TileId::new(1, 0, 1), SumTile { value: 0.0 });
        tiles.insert(TileId::new(1, 1, 1), SumTile { value: 0.0 });

        run_tile_grid(&mut tiles, 2);

        // after step 1: (1,0) and (0,1) = 1.0, (1,1) = 0.0, (0,0) = 1.0
        // after step 2: (1,1) picks up from both its neighbors (1,0)+(0,1) = 2.0
        assert_eq!(tiles[&TileId::new(1, 1, 1)].value, 2.0);
    }
}
