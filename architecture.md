# System Architecture

## 1. World Scaling & GIS
- **Scale**: 1:5 (1 real-meter = 0.2 game units).
- **Coordinate System**: WGS84 (Lat/Lon) projected to Web Mercator (EPSG:3857).
- **Chunking**: The map is divided into $1km \times 1km$ spatial chunks for lazy loading.

## 2. Data Pipeline
- **Ingestion**: `build.rs` fetches PBF from Geofabrik/BBBike.
- **Processing**: `map_gen` binary filters OSM tags:
    - `building`: Prefilled structures (colliders).
    - `highway`: Vehicle navigation paths.
    - `landuse=grass/meadow`: Player build-zones.
- **Storage**: Processed data is stored in `bincode` format for zero-cost deserialization.

## 3. Networking (Server-Authoritative)
- **Library**: `lightyear`.
- **Sync**: Components marked with `Replicate` are sent from Server -> Client.
- **Input**: Client-side prediction for vehicles; server-side reconciliation.
- **Headless**: Server runs with `minimal_plugins` (no rendering/window).
