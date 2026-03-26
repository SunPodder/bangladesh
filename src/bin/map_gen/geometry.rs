use crate::constants::WEB_MERCATOR_MAX_LAT;
use crate::terrain_types::TerrainPolygon;
use std::f64::consts::PI;

#[derive(Clone, Copy)]
pub struct Bounds {
    pub min_x: f64,
    pub min_y: f64,
    pub max_x: f64,
    pub max_y: f64,
}

impl Bounds {
    pub fn new(point: [f64; 2]) -> Self {
        Self {
            min_x: point[0],
            min_y: point[1],
            max_x: point[0],
            max_y: point[1],
        }
    }

    pub fn include(&mut self, point: [f64; 2]) {
        self.min_x = self.min_x.min(point[0]);
        self.min_y = self.min_y.min(point[1]);
        self.max_x = self.max_x.max(point[0]);
        self.max_y = self.max_y.max(point[1]);
    }

    pub fn include_bounds(&mut self, other: Bounds) {
        self.min_x = self.min_x.min(other.min_x);
        self.min_y = self.min_y.min(other.min_y);
        self.max_x = self.max_x.max(other.max_x);
        self.max_y = self.max_y.max(other.max_y);
    }
}

pub fn lat_lon_to_web_mercator(lat: f64, lon: f64) -> [f64; 2] {
    let lat = lat.clamp(-WEB_MERCATOR_MAX_LAT, WEB_MERCATOR_MAX_LAT);
    let x = lon.to_radians() * 6_378_137.0;
    let y = ((PI / 4.0) + (lat.to_radians() / 2.0)).tan().ln() * 6_378_137.0;
    [x, y]
}

pub fn polygon_bounds(points: &[[f64; 2]]) -> Bounds {
    let mut bounds = Bounds::new(points[0]);
    for point in points.iter().copied().skip(1) {
        bounds.include(point);
    }
    bounds
}

pub fn compute_global_bounds(polygons: &[TerrainPolygon]) -> Option<Bounds> {
    let first = polygons.first()?;
    let mut bounds = polygon_bounds(&first.points);

    for polygon in polygons.iter().skip(1) {
        bounds.include_bounds(polygon_bounds(&polygon.points));
    }

    Some(bounds)
}

pub fn point_in_polygon(point: [f64; 2], polygon: &[[f64; 2]]) -> bool {
    let mut inside = false;
    let mut j = polygon.len() - 1;

    for i in 0..polygon.len() {
        let xi = polygon[i][0];
        let yi = polygon[i][1];
        let xj = polygon[j][0];
        let yj = polygon[j][1];

        let intersects = ((yi > point[1]) != (yj > point[1]))
            && (point[0] < (xj - xi) * (point[1] - yi) / ((yj - yi) + 1e-12) + xi);

        if intersects {
            inside = !inside;
        }

        j = i;
    }

    inside
}
