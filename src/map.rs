//! Canvas map: an offline, tile-less world view that plots node positions.
//!
//! Positions are projected with a simple equirectangular projection and drawn
//! directly to a [`canvas`](iced::widget::canvas). Pan, zoom and node focus
//! all flow back to the application as messages, so the viewport lives in
//! [`App`](crate::app::App) alongside the rest of the UI state.

use iced::mouse;
use iced::widget::canvas::{self, Canvas, Frame, Geometry, Path, Stroke};
use iced::{Color, Point, Rectangle, Size};

use crate::app::Message;
use crate::tiles::{self, TileCache};

/// The deepest zoom the canvas allows. OpenStreetMap serves tiles up to zoom
/// 19, so past that the imagery is overzoomed (scaled up) rather than fetched.
const MAX_ZOOM: u8 = 21;

/// Lowest and highest pixels-per-radian the map allows.
const MIN_SCALE: f32 = 0.01;
const MAX_SCALE: f32 = 128.0 * (1u32 << MAX_ZOOM) as f32 / std::f32::consts::PI;

/// Latitude clamp for the Web Mercator projection.
const MAX_MERCATOR_LAT: f64 = 85.051_128_78;

/// A node plotted on the map.
#[derive(Debug, Clone)]
pub struct MapNode {
    pub num: u32,
    pub name: String,
    pub short: String,
    pub lat: f64,
    pub lon: f64,
    pub is_me: bool,
}

/// The map viewport: a centre and a zoom expressed as pixels per radian.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct MapView {
    pub lat: f64,
    pub lon: f64,
    pub scale: f32,
}

/// The Mercator "latitude" of a coordinate, in radians.
pub(crate) fn mercator_y(lat: f64) -> f64 {
    let clamped = lat.clamp(-MAX_MERCATOR_LAT, MAX_MERCATOR_LAT).to_radians();
    (std::f64::consts::FRAC_PI_4 + clamped / 2.0).tan().ln()
}

/// The inverse of [`mercator_y`], in degrees.
fn inverse_mercator(y: f64) -> f64 {
    (2.0 * y.exp().atan() - std::f64::consts::FRAC_PI_2).to_degrees()
}

impl MapView {
    pub(crate) fn to_screen(self, lat: f64, lon: f64, size: Size) -> Point {
        Point::new(
            size.width / 2.0 + (lon.to_radians() - self.lon.to_radians()) as f32 * self.scale,
            size.height / 2.0 - (mercator_y(lat) - mercator_y(self.lat)) as f32 * self.scale,
        )
    }

    fn to_geo(self, point: Point, size: Size) -> (f64, f64) {
        let lon = self.lon.to_radians() + ((point.x - size.width / 2.0) / self.scale) as f64;
        let y = mercator_y(self.lat) - ((point.y - size.height / 2.0) / self.scale) as f64;
        (inverse_mercator(y), lon.to_degrees())
    }

    /// A viewport that frames every node with a small margin.
    pub fn fit(nodes: &[MapNode], size: Size) -> MapView {
        let Some(first) = nodes.first() else {
            return MapView {
                lat: 20.0,
                lon: 0.0,
                scale: 8_000.0,
            };
        };

        let mut min_lat = first.lat;
        let mut max_lat = first.lat;
        let mut min_lon = first.lon;
        let mut max_lon = first.lon;
        for node in nodes {
            min_lat = min_lat.min(node.lat);
            max_lat = max_lat.max(node.lat);
            min_lon = min_lon.min(node.lon);
            max_lon = max_lon.max(node.lon);
        }

        let lat = inverse_mercator((mercator_y(min_lat) + mercator_y(max_lat)) / 2.0);
        let lon = (min_lon + max_lon) / 2.0;
        let span_y = (mercator_y(max_lat) - mercator_y(min_lat)).max(1e-6);
        let span_x = (max_lon - min_lon).to_radians().abs().max(1e-6);

        let width = size.width.max(1.0) as f64;
        let height = size.height.max(1.0) as f64;
        let scale = (height * 0.72 / span_y).min(width * 0.72 / span_x) as f32;

        MapView {
            lat,
            lon,
            scale: scale.clamp(MIN_SCALE, MAX_SCALE),
        }
    }

    /// Zooms by `factor`, keeping the geography under `point` in place.
    fn zoomed_at(&self, point: Point, size: Size, factor: f32) -> MapView {
        let (lat, lon) = self.to_geo(point, size);
        let scale = (self.scale * factor).clamp(MIN_SCALE, MAX_SCALE);
        let center_lon = lon.to_radians() - (point.x - size.width / 2.0) as f64 / scale as f64;
        let center_y = mercator_y(lat) + (point.y - size.height / 2.0) as f64 / scale as f64;
        MapView {
            lat: inverse_mercator(center_y),
            lon: center_lon.to_degrees(),
            scale,
        }
    }

    pub fn zoomed(&self, factor: f32) -> MapView {
        MapView {
            scale: (self.scale * factor).clamp(MIN_SCALE, MAX_SCALE),
            ..*self
        }
    }
}

/// Interaction state kept by the canvas widget between frames.
#[derive(Default)]
pub struct MapState {
    dragging: bool,
    last: Option<Point>,
    moved: f32,
    /// The canvas size the program last reported to the application. Used to
    /// publish the bounds once, after layout, without looping on redraws.
    last_bounds: Option<Size>,
}

/// The overlay canvas: markers and chrome, drawn above the tile canvas so the
/// imagery can never cover them. It also owns the pan/zoom interaction.
pub struct MapProgram {
    pub nodes: Vec<MapNode>,
    pub view: Option<MapView>,
    /// Whether the online layer is on, which controls the attribution notice.
    pub online: bool,
}

impl MapProgram {
    fn effective_view(&self, bounds: Rectangle) -> MapView {
        self.view
            .unwrap_or_else(|| MapView::fit(&self.nodes, bounds.size()))
    }

    fn node_at(&self, point: Point, bounds: Rectangle) -> Option<u32> {
        let view = self.effective_view(bounds);
        let mut best: Option<(u32, f32)> = None;
        for node in &self.nodes {
            let p = view.to_screen(node.lat, node.lon, bounds.size());
            let distance = (p.x - point.x).hypot(p.y - point.y);
            if distance <= 16.0 && best.is_none_or(|(_, d)| distance < d) {
                best = Some((node.num, distance));
            }
        }
        best.map(|(num, _)| num)
    }
}

/// The base canvas: the background and the online tiles (or the offline
/// graticule). It needs no interaction state; the overlay canvas above it
/// handles input.
pub struct TileProgram<'a> {
    pub nodes: Vec<MapNode>,
    pub view: Option<MapView>,
    pub tiles: &'a TileCache,
    pub online: bool,
}

impl canvas::Program<Message> for TileProgram<'_> {
    type State = ();

    fn draw(
        &self,
        _state: &Self::State,
        renderer: &iced::Renderer,
        _theme: &iced::Theme,
        bounds: Rectangle,
        _cursor: mouse::Cursor,
    ) -> Vec<Geometry> {
        use crate::theme;

        let view = self
            .view
            .unwrap_or_else(|| MapView::fit(&self.nodes, bounds.size()));
        let size = bounds.size();

        let mut frame = Frame::new(renderer, size);
        frame.fill_rectangle(Point::ORIGIN, size, theme::surface_alt());
        if self.online {
            draw_tiles(&mut frame, self, &view, size);
        } else {
            draw_graticule(&mut frame, &view, size);
        }

        vec![frame.into_geometry()]
    }
}

impl canvas::Program<Message> for MapProgram {
    type State = MapState;

    fn update(
        &self,
        state: &mut Self::State,
        event: &canvas::Event,
        bounds: Rectangle,
        cursor: mouse::Cursor,
    ) -> Option<canvas::Action<Message>> {
        // Publish the canvas size once it is known, and again after any
        // resize, so the application can fetch tiles for the visible region.
        // Publishing only on change keeps redraws from looping.
        if state.last_bounds != Some(bounds.size()) {
            state.last_bounds = Some(bounds.size());
            return Some(canvas::Action::publish(Message::MapBounds(bounds.size())));
        }

        match event {
            canvas::Event::Mouse(mouse::Event::ButtonPressed(mouse::Button::Left)) => {
                if let Some(position) = cursor.position_in(bounds) {
                    state.dragging = true;
                    state.last = Some(position);
                    state.moved = 0.0;
                    return Some(canvas::Action::capture());
                }
                None
            }
            canvas::Event::Mouse(mouse::Event::ButtonReleased(mouse::Button::Left)) => {
                state.dragging = false;
                state.last = None;
                if state.moved < 5.0 {
                    if let Some(position) = cursor.position_in(bounds) {
                        if let Some(num) = self.node_at(position, bounds) {
                            let view = self.effective_view(bounds);
                            if let Some(node) = self.nodes.iter().find(|node| node.num == num) {
                                let target = MapView {
                                    lat: node.lat,
                                    lon: node.lon,
                                    scale: view.scale,
                                };
                                return Some(
                                    canvas::Action::publish(Message::MapViewChanged(target))
                                        .and_capture(),
                                );
                            }
                        }
                    }
                }
                Some(canvas::Action::capture())
            }
            canvas::Event::Mouse(mouse::Event::CursorMoved { .. }) => {
                let position = cursor.position_in(bounds)?;
                if state.dragging {
                    if let Some(last) = state.last {
                        let view = self.effective_view(bounds);
                        let dx = position.x - last.x;
                        let dy = position.y - last.y;
                        state.moved += dx.abs() + dy.abs();
                        let moved = MapView {
                            lat: inverse_mercator(mercator_y(view.lat) + (dy / view.scale) as f64),
                            lon: (view.lon.to_radians() - (dx / view.scale) as f64).to_degrees(),
                            scale: view.scale,
                        };
                        state.last = Some(position);
                        return Some(
                            canvas::Action::publish(Message::MapViewChanged(moved)).and_capture(),
                        );
                    }
                }
                state.last = Some(position);
                None
            }
            canvas::Event::Mouse(mouse::Event::WheelScrolled { delta }) => {
                let position = cursor.position_in(bounds)?;
                let amount = match delta {
                    mouse::ScrollDelta::Lines { y, .. } => *y,
                    mouse::ScrollDelta::Pixels { y, .. } => *y / 40.0,
                };
                if amount == 0.0 {
                    return None;
                }
                let factor = if amount > 0.0 { 1.15 } else { 1.0 / 1.15 };
                let view = self
                    .effective_view(bounds)
                    .zoomed_at(position, bounds.size(), factor);
                Some(canvas::Action::publish(Message::MapViewChanged(view)).and_capture())
            }
            _ => None,
        }
    }

    fn draw(
        &self,
        _state: &Self::State,
        renderer: &iced::Renderer,
        _theme: &iced::Theme,
        bounds: Rectangle,
        cursor: mouse::Cursor,
    ) -> Vec<Geometry> {
        let view = self.effective_view(bounds);
        let size = bounds.size();

        // The overlay is transparent: the tile canvas beneath it draws the
        // background and imagery. Keeping the markers in their own canvas puts
        // them on their own renderer layer, which is the only thing that beats
        // the fixed primitive/image ordering inside a single layer.
        let mut frame = Frame::new(renderer, size);
        if self.online {
            draw_attribution(&mut frame, size);
        }
        draw_scale_bar(&mut frame, &view, size);
        draw_nodes(&mut frame, self, &view, size, cursor, bounds);

        vec![frame.into_geometry()]
    }

    fn mouse_interaction(
        &self,
        state: &Self::State,
        bounds: Rectangle,
        cursor: mouse::Cursor,
    ) -> mouse::Interaction {
        if state.dragging {
            return mouse::Interaction::Grabbing;
        }
        match cursor.position_in(bounds) {
            Some(position) if self.node_at(position, bounds).is_some() => {
                mouse::Interaction::Pointer
            }
            Some(_) => mouse::Interaction::Grab,
            None => mouse::Interaction::default(),
        }
    }
}

/// The overlay canvas widget: markers and chrome.
pub fn canvas(program: MapProgram) -> Canvas<MapProgram, Message> {
    Canvas::new(program)
        .width(iced::Length::Fill)
        .height(iced::Length::Fill)
}

/// The base canvas widget: background and online tiles. Place it under
/// [`canvas`] in a `Stack` so the markers render on their own layer.
pub fn tile_canvas(program: TileProgram<'_>) -> Canvas<TileProgram<'_>, Message> {
    Canvas::new(program)
        .width(iced::Length::Fill)
        .height(iced::Length::Fill)
}

/// Draw the cached online tiles for the viewport. Tiles not yet fetched simply
/// leave the background showing.
fn draw_tiles(frame: &mut Frame, program: &TileProgram<'_>, view: &MapView, size: Size) {
    for (key, rect) in tiles::draw_plan(view, size) {
        if let Some(handle) = program.tiles.get(&key) {
            frame.draw_image(rect, handle);
        }
    }
}

/// The attribution required by the OpenStreetMap tile usage policy, on a faint
/// backing so it stays readable over dark or light imagery.
fn draw_attribution(frame: &mut Frame, size: Size) {
    use crate::theme;

    let label = "© OpenStreetMap contributors";
    let text_width = label.chars().count() as f32 * 5.6;
    let padding = 5.0;
    let x = size.width - text_width - padding * 2.0 - 6.0;
    let y = size.height - 20.0;
    frame.fill_rectangle(
        Point::new(x, y),
        Size::new(text_width + padding * 2.0, 16.0),
        Color {
            a: 0.6,
            ..theme::surface_alt()
        },
    );
    frame.fill_text(canvas::Text {
        content: label.to_string(),
        position: Point::new(size.width - 8.0, size.height - 7.0),
        color: theme::text_muted(),
        size: 10.0.into(),
        align_x: iced::alignment::Horizontal::Right.into(),
        align_y: iced::alignment::Vertical::Bottom,
        ..Default::default()
    });
}

/// A grid step that keeps lines roughly 70 px apart at the current zoom.
fn grid_step(scale: f32, lat: f64) -> f64 {
    const STEPS: [f64; 19] = [
        0.001, 0.002, 0.005, 0.01, 0.02, 0.05, 0.1, 0.2, 0.5, 1.0, 2.0, 5.0, 10.0, 15.0, 20.0,
        30.0, 45.0, 60.0, 90.0,
    ];
    for step in STEPS {
        let spacing = (mercator_y(lat + step) - mercator_y(lat)).abs() * scale as f64;
        if spacing >= 70.0 {
            return step;
        }
    }
    STEPS[STEPS.len() - 1]
}

fn draw_graticule(frame: &mut Frame, view: &MapView, size: Size) {
    use crate::theme;

    let step = grid_step(view.scale, view.lat);
    let line = Color {
        a: 0.35,
        ..theme::border()
    };
    let stroke = Stroke::default().with_color(line).with_width(1.0);

    let center_y = mercator_y(view.lat);
    let half_y = (size.height / 2.0) as f64 / view.scale as f64;
    let top = inverse_mercator(center_y + half_y);
    let bottom = inverse_mercator(center_y - half_y);
    let mut lat = (bottom / step).floor() * step;
    while lat <= top {
        let y = view.to_screen(lat, view.lon, size).y;
        frame.stroke(
            &Path::line(Point::new(0.0, y), Point::new(size.width, y)),
            stroke,
        );
        lat += step;
    }

    let half_x = (size.width / 2.0) as f64 / view.scale as f64;
    let left = (view.lon.to_radians() - half_x).to_degrees();
    let right = (view.lon.to_radians() + half_x).to_degrees();
    let mut lon = (left / step).floor() * step;
    while lon <= right {
        let x = view.to_screen(view.lat, lon, size).x;
        frame.stroke(
            &Path::line(Point::new(x, 0.0), Point::new(x, size.height)),
            stroke,
        );
        lon += step;
    }
}

fn draw_scale_bar(frame: &mut Frame, view: &MapView, size: Size) {
    use crate::theme;

    let step = grid_step(view.scale, view.lat);
    let px =
        ((mercator_y(view.lat + step) - mercator_y(view.lat)).abs() * view.scale as f64) as f32;
    if px < 20.0 || px > size.width - 40.0 {
        return;
    }

    let km = step * 111.32;
    let label = if km >= 100.0 {
        format!("{km:.0} km")
    } else if km >= 1.0 {
        format!("{km:.1} km")
    } else {
        format!("{:.0} m", km * 1000.0)
    };

    let y = size.height - 18.0;
    let x0 = 16.0;
    let x1 = x0 + px;
    let stroke = Stroke::default()
        .with_color(theme::text_muted())
        .with_width(2.0);
    frame.stroke(&Path::line(Point::new(x0, y), Point::new(x1, y)), stroke);
    frame.stroke(
        &Path::line(Point::new(x0, y - 4.0), Point::new(x0, y + 4.0)),
        stroke,
    );
    frame.stroke(
        &Path::line(Point::new(x1, y - 4.0), Point::new(x1, y + 4.0)),
        stroke,
    );
    frame.fill_text(canvas::Text {
        content: label,
        position: Point::new(x0, y - 8.0),
        color: theme::text_muted(),
        size: 10.0.into(),
        align_y: iced::alignment::Vertical::Bottom,
        ..Default::default()
    });
}

fn draw_nodes(
    frame: &mut Frame,
    program: &MapProgram,
    view: &MapView,
    size: Size,
    cursor: mouse::Cursor,
    bounds: Rectangle,
) {
    use crate::theme;

    let hovered = cursor
        .position_in(bounds)
        .and_then(|position| program.node_at(position, bounds));

    for node in &program.nodes {
        let point = view.to_screen(node.lat, node.lon, size);
        if point.x < -40.0
            || point.y < -40.0
            || point.x > size.width + 40.0
            || point.y > size.height + 40.0
        {
            continue;
        }

        let is_hovered = hovered == Some(node.num);
        let radius = if node.is_me || is_hovered { 15.0 } else { 13.0 };
        let fill = if node.is_me {
            theme::primary()
        } else {
            theme::avatar_color(node.num)
        };

        if node.is_me {
            frame.stroke(
                &Path::circle(point, radius + 4.0),
                Stroke::default()
                    .with_color(Color { a: 0.35, ..fill })
                    .with_width(2.0),
            );
        }

        frame.fill(&Path::circle(point, radius), fill);
        frame.stroke(
            &Path::circle(point, radius),
            Stroke::default()
                .with_color(Color {
                    a: 0.9,
                    ..theme::text()
                })
                .with_width(2.0),
        );

        // The short name sits inside the circle, avatar-style. Longer callsigns
        // drop a little in size so they still fit.
        let short = if node.short.is_empty() {
            "?".to_string()
        } else {
            node.short.clone()
        };
        let label_size = if short.chars().count() <= 3 { 9.5 } else { 8.0 };
        frame.fill_text(canvas::Text {
            content: short,
            position: point,
            color: if node.is_me {
                theme::on_primary()
            } else {
                Color::WHITE
            },
            size: label_size.into(),
            align_x: iced::alignment::Horizontal::Center.into(),
            align_y: iced::alignment::Vertical::Center,
            ..Default::default()
        });

        // The full name appears above the marker on hover.
        if is_hovered {
            frame.fill_text(canvas::Text {
                content: node.name.clone(),
                position: Point::new(point.x, point.y - radius - 4.0),
                color: theme::text(),
                size: 12.0.into(),
                align_x: iced::alignment::Horizontal::Center.into(),
                align_y: iced::alignment::Vertical::Bottom,
                ..Default::default()
            });
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use iced::widget::canvas::Program;

    fn size() -> Size {
        Size::new(800.0, 600.0)
    }

    fn bounds() -> Rectangle {
        Rectangle::new(Point::ORIGIN, size())
    }

    fn node(num: u32, lat: f64, lon: f64) -> MapNode {
        MapNode {
            num,
            name: format!("Node {num}"),
            short: format!("N{num}"),
            lat,
            lon,
            is_me: false,
        }
    }

    fn published(action: canvas::Action<Message>) -> Option<Message> {
        action.into_inner().0
    }

    #[test]
    fn projection_round_trips() {
        let view = MapView {
            lat: 37.7749,
            lon: -122.4194,
            scale: 1_000.0,
        };
        let point = view.to_screen(37.8044, -122.2712, size());
        let (lat, lon) = view.to_geo(point, size());
        assert!((lat - 37.8044).abs() < 1e-6);
        assert!((lon + 122.2712).abs() < 1e-6);
    }

    #[test]
    fn zoom_keeps_the_anchor_point_fixed() {
        let view = MapView {
            lat: 37.7749,
            lon: -122.4194,
            scale: 1_000.0,
        };
        let anchor = Point::new(650.0, 150.0);
        let before = view.to_geo(anchor, size());
        let zoomed = view.zoomed_at(anchor, size(), 2.0);
        let after = zoomed.to_geo(anchor, size());
        assert!((before.0 - after.0).abs() < 1e-6);
        assert!((before.1 - after.1).abs() < 1e-6);
        assert!((zoomed.scale - 2_000.0).abs() < f32::EPSILON);
    }

    #[test]
    fn zoom_clamps_to_limits() {
        let view = MapView {
            lat: 0.0,
            lon: 0.0,
            scale: MAX_SCALE,
        };
        assert_eq!(view.zoomed(4.0).scale, MAX_SCALE);
        let view = MapView {
            lat: 0.0,
            lon: 0.0,
            scale: MIN_SCALE,
        };
        assert_eq!(view.zoomed(0.01).scale, MIN_SCALE);
    }

    #[test]
    fn fit_frames_every_node() {
        let nodes = vec![node(1, 37.6, -122.5), node(2, 37.9, -122.2)];
        let view = MapView::fit(&nodes, size());
        for node in &nodes {
            let point = view.to_screen(node.lat, node.lon, size());
            assert!(point.x >= 0.0 && point.x <= size().width);
            assert!(point.y >= 0.0 && point.y <= size().height);
        }
    }

    #[test]
    fn wheel_scroll_publishes_a_view_change() {
        let program = MapProgram {
            nodes: vec![node(7, 37.8, -122.27)],
            view: Some(MapView {
                lat: 37.8,
                lon: -122.27,
                scale: 1_000.0,
            }),
            online: false,
        };
        let mut state = MapState {
            last_bounds: Some(size()),
            ..Default::default()
        };
        let action = program
            .update(
                &mut state,
                &canvas::Event::Mouse(mouse::Event::WheelScrolled {
                    delta: mouse::ScrollDelta::Lines { x: 0.0, y: 1.0 },
                }),
                bounds(),
                mouse::Cursor::Available(Point::new(400.0, 300.0)),
            )
            .expect("scroll handled");
        assert!(matches!(
            published(action),
            Some(Message::MapViewChanged(_))
        ));
    }

    #[test]
    fn clicking_a_marker_centres_on_it() {
        let program = MapProgram {
            nodes: vec![node(7, 37.8, -122.27)],
            view: Some(MapView {
                lat: 37.8,
                lon: -122.27,
                scale: 1_000.0,
            }),
            online: false,
        };
        let mut state = MapState {
            last_bounds: Some(size()),
            ..Default::default()
        };
        let marker = program
            .effective_view(bounds())
            .to_screen(37.8, -122.27, size());
        let cursor = mouse::Cursor::Available(marker);

        program
            .update(
                &mut state,
                &canvas::Event::Mouse(mouse::Event::ButtonPressed(mouse::Button::Left)),
                bounds(),
                cursor,
            )
            .expect("press handled");

        let action = program
            .update(
                &mut state,
                &canvas::Event::Mouse(mouse::Event::ButtonReleased(mouse::Button::Left)),
                bounds(),
                cursor,
            )
            .expect("release handled");

        match published(action) {
            Some(Message::MapViewChanged(view)) => {
                assert!((view.lat - 37.8).abs() < 1e-6);
                assert!((view.lon + 122.27).abs() < 1e-6);
                assert!((view.scale - 1_000.0).abs() < f32::EPSILON);
            }
            other => panic!("expected a view change, got {other:?}"),
        }
    }

    #[test]
    fn canvas_publishes_its_bounds_once() {
        let program = MapProgram {
            nodes: Vec::new(),
            view: None,
            online: false,
        };
        let mut state = MapState::default();
        let redraw = canvas::Event::Window(iced::window::Event::RedrawRequested(
            iced::time::Instant::now(),
        ));

        let action = program
            .update(&mut state, &redraw, bounds(), mouse::Cursor::Unavailable)
            .expect("first redraw reports bounds");
        assert!(matches!(
            published(action),
            Some(Message::MapBounds(published)) if published == size()
        ));

        // The bounds are unchanged, so the next redraw stays quiet and the
        // canvas does not spin on self-published messages.
        assert!(
            program
                .update(&mut state, &redraw, bounds(), mouse::Cursor::Unavailable)
                .is_none()
        );
    }
}
