use crate::geometry::{Bounds, lat_lon_to_web_mercator};
use crate::types::TileSpec;
use anyhow::{Result, anyhow, ensure};

#[derive(Debug, Clone, Copy)]
pub struct TileGrid {
    pub origin_x_m: f64,
    pub origin_y_m: f64,
    pub tile_size_m: u32,
    pub cols: u32,
    pub rows: u32,
}

impl TileGrid {
    pub fn from_bounds(bounds: Bounds, tile_size_m: u32) -> Result<Self> {
        ensure!(tile_size_m > 0, "tile_size_m must be > 0");
        let tile_size = tile_size_m as f64;
        let origin_x_m = (bounds.min_x / tile_size).floor() * tile_size;
        let origin_y_m = (bounds.min_y / tile_size).floor() * tile_size;
        let cols = ((bounds.max_x - origin_x_m) / tile_size).ceil().max(1.0) as u32;
        let rows = ((bounds.max_y - origin_y_m) / tile_size).ceil().max(1.0) as u32;

        Ok(Self {
            origin_x_m,
            origin_y_m,
            tile_size_m,
            cols,
            rows,
        })
    }

    pub fn tile_spec(&self, grid_x: u32, grid_y: u32) -> TileSpec {
        let tile_size = self.tile_size_m as f64;
        let min_x = self.origin_x_m + f64::from(grid_x) * tile_size;
        let min_y = self.origin_y_m + f64::from(grid_y) * tile_size;
        TileSpec {
            id: grid_y * self.cols + grid_x,
            grid_x,
            grid_y,
            bounds: Bounds {
                min_x,
                min_y,
                max_x: min_x + tile_size,
                max_y: min_y + tile_size,
            },
        }
    }

    pub fn tile_specs(&self) -> Vec<TileSpec> {
        let mut specs = Vec::with_capacity((self.cols * self.rows) as usize);
        for grid_y in 0..self.rows {
            for grid_x in 0..self.cols {
                specs.push(self.tile_spec(grid_x, grid_y));
            }
        }
        specs
    }

    pub fn tile_ids_for_lat_lon_bounds(
        &self,
        min_lat: f64,
        min_lon: f64,
        max_lat: f64,
        max_lon: f64,
    ) -> Result<Vec<u32>> {
        let sw = lat_lon_to_web_mercator(min_lat, min_lon);
        let ne = lat_lon_to_web_mercator(max_lat, max_lon);
        let bounds = Bounds {
            min_x: sw[0].min(ne[0]),
            min_y: sw[1].min(ne[1]),
            max_x: sw[0].max(ne[0]),
            max_y: sw[1].max(ne[1]),
        };
        Ok(self.tile_ids_for_bounds(bounds))
    }

    pub fn tile_ids_for_bounds(&self, bounds: Bounds) -> Vec<u32> {
        let tile_size = self.tile_size_m as f64;
        let min_x = (((bounds.min_x - self.origin_x_m) / tile_size).floor() as i64)
            .clamp(0, i64::from(self.cols.saturating_sub(1)));
        let max_x = (((bounds.max_x - self.origin_x_m) / tile_size).floor() as i64)
            .clamp(0, i64::from(self.cols.saturating_sub(1)));
        let min_y = (((bounds.min_y - self.origin_y_m) / tile_size).floor() as i64)
            .clamp(0, i64::from(self.rows.saturating_sub(1)));
        let max_y = (((bounds.max_y - self.origin_y_m) / tile_size).floor() as i64)
            .clamp(0, i64::from(self.rows.saturating_sub(1)));

        let mut ids = Vec::new();
        for y in min_y..=max_y {
            for x in min_x..=max_x {
                ids.push((y as u32) * self.cols + (x as u32));
            }
        }
        ids
    }
}

pub fn parse_tile_id_ranges(input: &str, max_tile_id: u32) -> Result<Vec<u32>> {
    let mut ids = Vec::new();

    for part in input
        .split(',')
        .map(str::trim)
        .filter(|part| !part.is_empty())
    {
        if let Some((start, end)) = part.split_once('-') {
            let start: u32 = start
                .trim()
                .parse()
                .map_err(|_| anyhow!("invalid tile id range start: {start}"))?;
            let end: u32 = end
                .trim()
                .parse()
                .map_err(|_| anyhow!("invalid tile id range end: {end}"))?;
            ensure!(start <= end, "invalid tile range {part}");
            ensure!(
                end <= max_tile_id,
                "tile range {part} exceeds max tile id {max_tile_id}"
            );
            ids.extend(start..=end);
        } else {
            let id: u32 = part
                .parse()
                .map_err(|_| anyhow!("invalid tile id: {part}"))?;
            ensure!(
                id <= max_tile_id,
                "tile id {id} exceeds max tile id {max_tile_id}"
            );
            ids.push(id);
        }
    }

    ids.sort_unstable();
    ids.dedup();
    Ok(ids)
}
