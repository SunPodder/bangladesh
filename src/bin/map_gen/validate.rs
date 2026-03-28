use crate::geometry::Bounds;
use crate::types::{
    LodSettings, ParsedMapData, RawAreaFeature, RawBuildingFeature, RawPoiFeature, RawRoadFeature,
    TileSpec,
};
use anyhow::Result;
use bangladesh::shared::world::{
    AreaFeature, BuildingFeature, LodData, PoiFeature, QuantizedBounds, QuantizedPoint, RoadClass,
    RoadFeature, TILE_FORMAT_VERSION, TileData,
};
use geo::{
    BoundingRect, LineString, Polygon, RemoveRepeatedPoints, SimplifyVwPreserve, Validation,
};
use rstar::{AABB, RTree, RTreeObject};

#[derive(Clone, Copy)]
struct IndexedFeature {
    envelope: AABB<[f64; 2]>,
    index: usize,
}

impl RTreeObject for IndexedFeature {
    type Envelope = AABB<[f64; 2]>;

    fn envelope(&self) -> Self::Envelope {
        self.envelope
    }
}

pub struct SpatialIndex {
    area_index: RTree<IndexedFeature>,
    building_index: RTree<IndexedFeature>,
    road_index: RTree<IndexedFeature>,
    poi_index: RTree<IndexedFeature>,
}

impl SpatialIndex {
    pub fn build(parsed: &ParsedMapData) -> Self {
        Self {
            area_index: RTree::bulk_load(
                parsed
                    .areas
                    .iter()
                    .enumerate()
                    .map(|(index, area)| IndexedFeature {
                        envelope: aabb_from_bounds(bounds_from_points(&area.points)),
                        index,
                    })
                    .collect(),
            ),
            building_index: RTree::bulk_load(
                parsed
                    .buildings
                    .iter()
                    .enumerate()
                    .map(|(index, building)| IndexedFeature {
                        envelope: aabb_from_bounds(bounds_from_points(&building.points)),
                        index,
                    })
                    .collect(),
            ),
            road_index: RTree::bulk_load(
                parsed
                    .roads
                    .iter()
                    .enumerate()
                    .map(|(index, road)| IndexedFeature {
                        envelope: aabb_from_bounds(bounds_from_points(&road.points)),
                        index,
                    })
                    .collect(),
            ),
            poi_index: RTree::bulk_load(
                parsed
                    .pois
                    .iter()
                    .enumerate()
                    .map(|(index, poi)| IndexedFeature {
                        envelope: AABB::from_point(poi.point),
                        index,
                    })
                    .collect(),
            ),
        }
    }
}

pub fn build_tile(
    tile_spec: TileSpec,
    lods: &[LodSettings],
    parsed: &ParsedMapData,
    spatial_index: &SpatialIndex,
) -> Result<TileData> {
    let overlap = tile_spec.bounds.width() * 0.01;
    let query = aabb_from_bounds(tile_spec.bounds.expand(overlap));

    let area_ids = spatial_index
        .area_index
        .locate_in_envelope_intersecting(&query)
        .map(|entry| entry.index)
        .collect::<Vec<_>>();
    let building_ids = spatial_index
        .building_index
        .locate_in_envelope_intersecting(&query)
        .map(|entry| entry.index)
        .collect::<Vec<_>>();
    let road_ids = spatial_index
        .road_index
        .locate_in_envelope_intersecting(&query)
        .map(|entry| entry.index)
        .collect::<Vec<_>>();
    let poi_ids = spatial_index
        .poi_index
        .locate_in_envelope_intersecting(&query)
        .map(|entry| entry.index)
        .collect::<Vec<_>>();

    let lods = lods
        .iter()
        .enumerate()
        .map(|(lod_level, lod)| LodData {
            lod_level: lod_level as u8,
            roads: road_ids
                .iter()
                .filter_map(|index| {
                    simplify_road(
                        &parsed.roads[*index],
                        tile_spec.bounds,
                        lod_level as u8,
                        lod,
                    )
                })
                .collect(),
            buildings: building_ids
                .iter()
                .filter_map(|index| {
                    simplify_building(
                        &parsed.buildings[*index],
                        tile_spec.bounds,
                        lod_level as u8,
                        lod,
                    )
                })
                .collect(),
            areas: area_ids
                .iter()
                .filter_map(|index| {
                    simplify_area(
                        &parsed.areas[*index],
                        tile_spec.bounds,
                        lod_level as u8,
                        lod,
                    )
                })
                .collect(),
            pois: poi_ids
                .iter()
                .filter_map(|index| {
                    simplify_poi(&parsed.pois[*index], tile_spec.bounds, lod_level as u8)
                })
                .collect(),
        })
        .collect();

    Ok(TileData {
        version: TILE_FORMAT_VERSION,
        tile_id: tile_spec.id,
        grid_x: tile_spec.grid_x,
        grid_y: tile_spec.grid_y,
        tile_size_m: tile_spec.bounds.width() as u32,
        origin_x_m: tile_spec.bounds.min_x,
        origin_y_m: tile_spec.bounds.min_y,
        lods,
    })
}

fn simplify_area(
    raw: &RawAreaFeature,
    tile_bounds: Bounds,
    lod_level: u8,
    lod: &LodSettings,
) -> Option<AreaFeature> {
    let mut polygon = polygon_from_points(&raw.points)?;
    polygon.remove_repeated_points_mut();
    if !polygon.is_valid() {
        return None;
    }

    if lod_level > 0 {
        polygon = polygon.simplify_vw_preserve(f64::from(lod.simplify_tolerance_m.powi(2)));
    }

    if !polygon.is_valid() {
        return None;
    }

    let rect = polygon.bounding_rect()?;
    let area = (rect.max().x - rect.min().x) * (rect.max().y - rect.min().y);
    let min_area = f64::from(lod.simplify_tolerance_m.max(1.0).powi(2)) * 4.0;
    if lod_level > 0 && area < min_area {
        return None;
    }

    let ring = polygon
        .exterior()
        .0
        .iter()
        .map(|coord| quantize_point(coord.x, coord.y, tile_bounds))
        .collect::<Option<Vec<_>>>()?;
    let bounds = quantized_bounds(&ring)?;

    Some(AreaFeature {
        kind: raw.kind,
        bounds,
        rings: vec![ring],
    })
}

fn simplify_building(
    raw: &RawBuildingFeature,
    tile_bounds: Bounds,
    lod_level: u8,
    lod: &LodSettings,
) -> Option<BuildingFeature> {
    let mut polygon = polygon_from_points(&raw.points)?;
    polygon.remove_repeated_points_mut();
    if !polygon.is_valid() {
        return None;
    }

    if lod_level > 0 {
        polygon = polygon.simplify_vw_preserve(f64::from(lod.simplify_tolerance_m.powi(2)));
    }

    let rect = polygon.bounding_rect()?;
    let area = (rect.max().x - rect.min().x) * (rect.max().y - rect.min().y);
    let min_area = if lod_level == 0 {
        8.0
    } else {
        f64::from(lod.simplify_tolerance_m.max(1.0).powi(2)) * 6.0
    };
    if area < min_area {
        return None;
    }

    let footprint = polygon
        .exterior()
        .0
        .iter()
        .map(|coord| quantize_point(coord.x, coord.y, tile_bounds))
        .collect::<Option<Vec<_>>>()?;

    Some(BuildingFeature {
        bounds: quantized_bounds(&footprint)?,
        footprint,
    })
}

fn simplify_road(
    raw: &RawRoadFeature,
    tile_bounds: Bounds,
    lod_level: u8,
    lod: &LodSettings,
) -> Option<RoadFeature> {
    if !road_class_visible(raw.class, lod_level) {
        return None;
    }

    let mut line = line_string_from_points(&raw.points)?;
    line.remove_repeated_points_mut();
    if line.0.len() < 2 {
        return None;
    }

    if lod_level > 0 {
        line = line.simplify_vw_preserve(f64::from(lod.simplify_tolerance_m.powi(2)));
    }

    if line.0.len() < 2 {
        return None;
    }

    let rect = line.bounding_rect()?;
    let length_hint = (rect.max().x - rect.min().x).hypot(rect.max().y - rect.min().y);
    if lod_level > 0 && length_hint < f64::from(lod.simplify_tolerance_m * 2.0) {
        return None;
    }

    let points = line
        .0
        .iter()
        .map(|coord| quantize_point(coord.x, coord.y, tile_bounds))
        .collect::<Option<Vec<_>>>()?;

    Some(RoadFeature {
        class: raw.class,
        width_m: (raw.width_m / (1.0 + lod_level as f32 * 0.12)).max(1.0),
        bounds: quantized_bounds(&points)?,
        points,
    })
}

fn simplify_poi(raw: &RawPoiFeature, tile_bounds: Bounds, lod_level: u8) -> Option<PoiFeature> {
    if lod_level >= 4 && !(raw.kind.starts_with("place:") || raw.kind == "railway:station") {
        return None;
    }

    Some(PoiFeature {
        kind: raw.kind.clone(),
        name: raw.name.clone(),
        point: quantize_point(raw.point[0], raw.point[1], tile_bounds)?,
    })
}

fn road_class_visible(class: RoadClass, lod_level: u8) -> bool {
    match class {
        RoadClass::Motorway | RoadClass::Primary => true,
        RoadClass::Secondary => lod_level <= 5,
        RoadClass::Local => lod_level <= 3,
        RoadClass::Service => lod_level <= 2,
        RoadClass::Track => lod_level <= 1,
    }
}

fn polygon_from_points(points: &[[f64; 2]]) -> Option<Polygon<f64>> {
    let line = line_string_from_points(points)?;
    Some(Polygon::new(line, vec![]))
}

fn line_string_from_points(points: &[[f64; 2]]) -> Option<LineString<f64>> {
    if points.len() < 2 {
        return None;
    }
    Some(LineString::from(
        points
            .iter()
            .map(|point| (point[0], point[1]))
            .collect::<Vec<_>>(),
    ))
}

fn bounds_from_points(points: &[[f64; 2]]) -> Bounds {
    let mut bounds = Bounds::new(points[0]);
    for point in points.iter().copied().skip(1) {
        bounds.include(point);
    }
    bounds
}

fn aabb_from_bounds(bounds: Bounds) -> AABB<[f64; 2]> {
    AABB::from_corners([bounds.min_x, bounds.min_y], [bounds.max_x, bounds.max_y])
}

fn quantize_point(x: f64, y: f64, tile_bounds: Bounds) -> Option<QuantizedPoint> {
    let local_x = ((x - tile_bounds.min_x)
        * f64::from(bangladesh::shared::world::QUANTIZATION_SCALE))
    .round();
    let local_y = ((y - tile_bounds.min_y)
        * f64::from(bangladesh::shared::world::QUANTIZATION_SCALE))
    .round();
    if !(i32::MIN as f64..=i32::MAX as f64).contains(&local_x)
        || !(i32::MIN as f64..=i32::MAX as f64).contains(&local_y)
    {
        return None;
    }

    Some(QuantizedPoint {
        x: local_x as i32,
        y: local_y as i32,
    })
}

fn quantized_bounds(points: &[QuantizedPoint]) -> Option<QuantizedBounds> {
    let first = *points.first()?;
    let mut min_x = first.x;
    let mut min_y = first.y;
    let mut max_x = first.x;
    let mut max_y = first.y;

    for point in points.iter().copied().skip(1) {
        min_x = min_x.min(point.x);
        min_y = min_y.min(point.y);
        max_x = max_x.max(point.x);
        max_y = max_y.max(point.y);
    }

    Some(QuantizedBounds {
        min_x,
        min_y,
        max_x,
        max_y,
    })
}
