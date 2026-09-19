//! Online raster map tiles (the optional OpenStreetMap layer).
//!
//! The map canvas is otherwise tile-less; this module adds slippy-map tiles on
//! request. It owns the projection maths that maps the canvas viewport onto the
//! standard `z/x/y` tile grid, a small in-memory cache, and the fetch used to
//! pull a single tile from the OpenStreetMap tile server.
//!
//! Tiles are only fetched when the user turns the layer on. The tile server's
//! usage policy asks for an identifying `User-Agent`, which is set on the
//! shared [`reqwest::Client`] the application builds once at startup.

use std::collections::HashMap;
use std::sync::Arc;

use iced::widget::image;
use iced::{Rectangle, Size};
use tokio::sync::Semaphore;

use crate::map::{MapView, mercator_y};

/// Edge length of a tile, in pixels, at every zoom level.
pub const TILE_SIZE: f64 = 256.0;

/// The highest zoom the OpenStreetMap tile server serves.
pub const MAX_TILE_ZOOM: u8 = 19;

/// A tile coordinate in the standard slippy-map scheme.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct TileKey {
    pub z: u8,
    pub x: u32,
    pub y: u32,
}

/// The outcome of fetching one tile.
pub enum TileUpdate {
    Loaded(TileKey, image::Handle),
    Failed(TileKey),
}

/// The integer zoom whose resolution best matches a canvas `scale`, expressed
/// in pixels per radian.
///
/// At zoom `z` the grid has `256 * 2^z` pixels across the world, or
/// `128 * 2^z / pi` pixels per radian; inverting that gives the zoom.
pub fn zoom_for_scale(scale: f32) -> u8 {
    let tiles = (scale as f64 * std::f64::consts::PI / 128.0).log2();
    tiles.round().clamp(0.0, MAX_TILE_ZOOM as f64) as u8
}

/// The world pixel coordinate of a latitude/longitude at zoom `z`, measured
/// from the north-west corner of the world.
pub fn world_px(lat: f64, lon: f64, z: u8) -> (f64, f64) {
    let span = (1u32 << z) as f64 * TILE_SIZE;
    let x = (lon + 180.0) / 360.0 * span;
    let y = (std::f64::consts::PI - mercator_y(lat)) / (2.0 * std::f64::consts::PI) * span;
    (x, y)
}

/// Every tile overlapping the viewport, paired with the screen rectangle it
/// should be drawn into.
///
/// The rectangles are exact: the same Mercator projection drives both the tile
/// grid and the node markers, so tiles line up with the plotted positions even
/// when the canvas scale sits between two zoom levels. At most `max` tiles are
/// returned, which bounds both drawing and fetching when zoomed far out.
pub fn visible_tiles(view: &MapView, size: Size, max: usize) -> Vec<(TileKey, Rectangle)> {
    visible_tiles_at(view, size, zoom_for_scale(view.scale), max)
}

/// Tiles at a specific zoom that overlap the viewport, with their screen
/// rectangles. [`visible_tiles`] is the common case of the target zoom.
pub fn visible_tiles_at(
    view: &MapView,
    size: Size,
    z: u8,
    max: usize,
) -> Vec<(TileKey, Rectangle)> {
    let count = 1i64 << z;
    let span = count as f64 * TILE_SIZE;

    // World pixels per screen pixel: how much a tile is scaled on screen.
    let scale = view.scale as f64;
    let base_scale = span / std::f64::consts::TAU;
    let k = scale / base_scale;

    let (cx, cy) = world_px(view.lat, view.lon, z);
    let width = size.width as f64;
    let height = size.height as f64;

    let left = (cx - (width / 2.0) / k) / TILE_SIZE;
    let top = (cy - (height / 2.0) / k) / TILE_SIZE;
    let right = (cx + (width / 2.0) / k) / TILE_SIZE;
    let bottom = (cy + (height / 2.0) / k) / TILE_SIZE;

    let mut tiles = Vec::new();
    for ty in (top.floor() as i64)..=(bottom.floor() as i64) {
        for tx in (left.floor() as i64)..=(right.floor() as i64) {
            if tx < 0 || ty < 0 || tx >= count || ty >= count {
                continue;
            }
            let screen_x = width / 2.0 + (tx as f64 * TILE_SIZE - cx) * k;
            let screen_y = height / 2.0 + (ty as f64 * TILE_SIZE - cy) * k;
            let edge = (TILE_SIZE * k) as f32;
            tiles.push((
                TileKey {
                    z,
                    x: tx as u32,
                    y: ty as u32,
                },
                Rectangle {
                    x: screen_x as f32,
                    y: screen_y as f32,
                    width: edge,
                    height: edge,
                },
            ));
            if tiles.len() >= max {
                return tiles;
            }
        }
    }
    tiles
}

/// The tiles to draw for a viewport, ordered back to front.
///
/// The target zoom is drawn last so it wins wherever it is loaded. Behind it
/// come cached tiles from nearby zoom levels: coarser ones first, then finer
/// ones. That is what lets the previous level keep showing, scaled to the new
/// zoom, until the target level's tiles arrive; without it the layer would
/// blank out for a moment on every zoom step.
pub fn draw_plan(view: &MapView, size: Size) -> Vec<(TileKey, Rectangle)> {
    let target = zoom_for_scale(view.scale);
    let coarsest = target.saturating_sub(4);
    // Only a little finer: each level down squares the tile count, so the
    // previous one or two levels are enough to cover a zoom-out.
    let finest = (target + 2).min(MAX_TILE_ZOOM);

    let mut plan = Vec::new();
    for z in coarsest..target {
        plan.extend(visible_tiles_at(view, size, z, MAX_VISIBLE_TILES));
    }
    for z in (target + 1)..=finest {
        plan.extend(visible_tiles_at(view, size, z, MAX_VISIBLE_TILES));
    }
    plan.extend(visible_tiles_at(view, size, target, MAX_VISIBLE_TILES));
    plan
}

/// Fetch one tile from the OpenStreetMap tile server.
///
/// Request concurrency is bounded by `permits`, so a viewport's worth of
/// missing tiles queues behind a handful of in-flight requests rather than
/// opening a socket each.
pub async fn fetch(client: reqwest::Client, permits: Arc<Semaphore>, key: TileKey) -> TileUpdate {
    let _permit = permits.acquire().await;
    let url = format!(
        "https://tile.openstreetmap.org/{}/{}/{}.png",
        key.z, key.x, key.y
    );
    let result = client.get(&url).send().await;
    match result {
        Ok(response) if response.status().is_success() => match response.bytes().await {
            Ok(bytes) => TileUpdate::Loaded(key, image::Handle::from_bytes(bytes)),
            Err(error) => {
                tracing::warn!(%url, %error, "failed to read map tile body");
                TileUpdate::Failed(key)
            }
        },
        Ok(response) => {
            tracing::warn!(%url, status = %response.status(), "map tile request rejected");
            TileUpdate::Failed(key)
        }
        Err(error) => {
            tracing::warn!(%url, %error, "map tile request failed");
            TileUpdate::Failed(key)
        }
    }
}

/// How many tiles a single viewport may request at once.
pub const MAX_VISIBLE_TILES: usize = 64;

/// How many tiles are kept in memory before the oldest are evicted.
pub const MAX_CACHED_TILES: usize = 256;

/// How many tile requests may be in flight at once.
pub const MAX_CONCURRENT_REQUESTS: usize = 8;

/// The shared tile cache, keyed by tile coordinate.
pub type TileCache = HashMap<TileKey, image::Handle>;

#[cfg(test)]
mod tests {
    use super::*;

    fn view(scale: f32) -> MapView {
        MapView {
            lat: 37.7749,
            lon: -122.4194,
            scale,
        }
    }

    #[test]
    fn zoom_tracks_scale() {
        // One tile per world at the very lowest zoom.
        assert_eq!(zoom_for_scale(1.0), 0);
        // `128 * 2^z / pi` pixels per radian selects zoom z.
        let z = 12u8;
        let base = 128.0 * (1u64 << z) as f32 / std::f32::consts::PI;
        assert_eq!(zoom_for_scale(base), z);
        // Slightly more than half a level up rounds back down.
        assert_eq!(zoom_for_scale(base * 1.2), z);
        // And it never exceeds the server's limit.
        assert_eq!(zoom_for_scale(1.0e9), MAX_TILE_ZOOM);
    }

    #[test]
    fn world_pixel_origin_is_north_west() {
        let (x, y) = world_px(0.0, -180.0, 0);
        assert!((x - 0.0).abs() < 1e-6);
        assert!((y - TILE_SIZE / 2.0).abs() < 1e-6);

        let (x, y) = world_px(0.0, 0.0, 1);
        assert!((x - TILE_SIZE).abs() < 1e-6);
        // The equator sits midway down the world at every zoom.
        assert!((y - TILE_SIZE).abs() < 1e-6);
    }

    #[test]
    fn visible_tiles_cover_the_viewport() {
        let view = view(12_000.0);
        let size = Size::new(800.0, 600.0);
        let tiles = visible_tiles(&view, size, MAX_VISIBLE_TILES);

        assert!(!tiles.is_empty());
        // The canvas centre must fall inside some tile's rectangle.
        let centre = iced::Point::new(size.width / 2.0, size.height / 2.0);
        assert!(tiles.iter().any(|(_, rect)| {
            centre.x >= rect.x
                && centre.x <= rect.x + rect.width
                && centre.y >= rect.y
                && centre.y <= rect.y + rect.height
        }));
    }

    #[test]
    fn visible_tiles_are_capped_and_in_range() {
        let tiles = visible_tiles(&view(1.0), Size::new(4000.0, 4000.0), 8);
        assert!(tiles.len() <= 8);
        for (key, _) in tiles {
            assert!(key.x < (1 << key.z));
            assert!(key.y < (1 << key.z));
        }
    }

    #[test]
    fn tiles_align_with_a_projected_point() {
        // A point projected onto the canvas must land inside the tile the same
        // coordinate maps to. This is what keeps markers on the imagery.
        let view = view(9_000.0);
        let size = Size::new(800.0, 600.0);
        let tiles = visible_tiles(&view, size, MAX_VISIBLE_TILES);

        let lat = 37.8044;
        let lon = -122.2712;
        let point = view.to_screen(lat, lon, size);
        let z = zoom_for_scale(view.scale);
        let (wx, wy) = world_px(lat, lon, z);
        let key = TileKey {
            z,
            x: (wx / TILE_SIZE).floor() as u32,
            y: (wy / TILE_SIZE).floor() as u32,
        };

        let (_, rect) = tiles
            .iter()
            .find(|(candidate, _)| *candidate == key)
            .expect("tile for the point is visible");
        assert!(point.x >= rect.x && point.x <= rect.x + rect.width);
        assert!(point.y >= rect.y && point.y <= rect.y + rect.height);
    }

    #[test]
    fn draw_plan_puts_the_target_level_last() {
        let view = view(12_000.0);
        let size = Size::new(800.0, 600.0);
        let plan = draw_plan(&view, size);
        let target = zoom_for_scale(view.scale);

        let first_target = plan
            .iter()
            .position(|(key, _)| key.z == target)
            .expect("the target level is always planned");
        // Once the target level starts it runs to the end, so it paints over
        // every fallback.
        assert!(plan[first_target..].iter().all(|(key, _)| key.z == target));
        // A coarser fallback is queued ahead of it, which is what avoids a
        // blank map on a zoom step.
        assert!(plan[..first_target].iter().any(|(key, _)| key.z < target));
    }
}
