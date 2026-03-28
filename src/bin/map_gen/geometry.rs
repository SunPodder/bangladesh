use std::f64::consts::PI;

pub const WEB_MERCATOR_MAX_LAT: f64 = 85.051_128_78;

#[derive(Debug, Clone, Copy)]
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

    pub fn expand(&self, margin: f64) -> Self {
        Self {
            min_x: self.min_x - margin,
            min_y: self.min_y - margin,
            max_x: self.max_x + margin,
            max_y: self.max_y + margin,
        }
    }

    pub fn width(&self) -> f64 {
        self.max_x - self.min_x
    }

    pub fn height(&self) -> f64 {
        self.max_y - self.min_y
    }

    pub fn area(&self) -> f64 {
        self.width().max(0.0) * self.height().max(0.0)
    }
}

#[derive(Debug, Clone, Copy)]
pub struct LatLonBounds {
    pub min_lat: f64,
    pub min_lon: f64,
    pub max_lat: f64,
    pub max_lon: f64,
}

impl LatLonBounds {
    pub fn new(lat: f64, lon: f64) -> Self {
        Self {
            min_lat: lat,
            min_lon: lon,
            max_lat: lat,
            max_lon: lon,
        }
    }

    pub fn include(&mut self, lat: f64, lon: f64) {
        self.min_lat = self.min_lat.min(lat);
        self.min_lon = self.min_lon.min(lon);
        self.max_lat = self.max_lat.max(lat);
        self.max_lon = self.max_lon.max(lon);
    }
}

pub fn lat_lon_to_web_mercator(lat: f64, lon: f64) -> [f64; 2] {
    let clamped_lat = lat.clamp(-WEB_MERCATOR_MAX_LAT, WEB_MERCATOR_MAX_LAT);
    let x = lon.to_radians() * 6_378_137.0;
    let y = ((PI / 4.0) + (clamped_lat.to_radians() / 2.0)).tan().ln() * 6_378_137.0;
    [x, y]
}
