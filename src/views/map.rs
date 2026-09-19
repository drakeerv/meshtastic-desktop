//! Map view: node positions plotted on an offline canvas, with pan, zoom and
//! a list of positioned nodes.

use iced::widget::{Space, button, column, container, row, scrollable, text};
use iced::{Alignment, Border, Color, Element, Length, Padding, Size};

use crate::app::{App, Message};
use crate::format;
use crate::icons::lucide;
use crate::map::{MapNode, MapProgram, MapView, TileProgram};
use crate::theme;
use crate::widgets;

/// A nominal canvas size used to derive an initial fit before the canvas has
/// reported its real bounds.
const NOMINAL_SIZE: Size = Size::new(1000.0, 700.0);

/// Marker colour for nodes other than the local one (matches `map.rs`).
const MARKER_COLOR: Color = Color::from_rgb8(0x4F, 0x9A, 0xA8);

pub fn view(app: &App) -> Element<'_, Message> {
    let nodes = app.map_nodes();
    // With the tile layer on the map is worth showing even before any node
    // reports a position.
    if nodes.is_empty() && !app.online_tiles {
        return empty(app);
    }

    let view = app
        .map_view
        .unwrap_or_else(|| MapView::fit(&nodes, NOMINAL_SIZE));

    let mut actions = vec![
        tiles_toggle(app),
        icon_button(
            lucide::zoom_out().into(),
            Message::MapViewChanged(view.zoomed(1.0 / 1.25)),
        ),
        icon_button(
            lucide::zoom_in().into(),
            Message::MapViewChanged(view.zoomed(1.25)),
        ),
        icon_button(lucide::maximize().into(), Message::MapFit),
    ];
    if let Some((lat, lon)) = app.my_coords() {
        actions.push(icon_button(
            lucide::locate_fixed().into(),
            Message::MapViewChanged(MapView {
                lat,
                lon,
                scale: view.scale,
            }),
        ));
    }

    // Two canvases in a stack: imagery on its own layer below, markers above.
    // Within a single canvas, tile images would be drawn over the marker
    // shapes regardless of order, so the split is what keeps pins visible.
    let canvas = container(
        iced::widget::stack![
            crate::map::tile_canvas(TileProgram {
                nodes: nodes.clone(),
                view: app.map_view,
                tiles: &app.map_tiles,
                online: app.online_tiles,
            }),
            crate::map::canvas(MapProgram {
                nodes: nodes.clone(),
                view: app.map_view,
                online: app.online_tiles,
            }),
        ]
        .width(Length::Fill)
        .height(Length::Fill),
    )
    .width(Length::Fill)
    .height(Length::Fill)
    .style(theme::card);

    let body = row![canvas, node_panel(app, &nodes, view)]
        .spacing(12)
        .width(Length::Fill)
        .height(Length::Fill);

    column![
        widgets::page_header("Map", Some(subtitle(app, &nodes)), actions),
        container(body)
            .width(Length::Fill)
            .height(Length::Fill)
            .padding(Padding::from([4, 22])),
    ]
    .width(Length::Fill)
    .height(Length::Fill)
    .into()
}

/// Toggle for the optional online OpenStreetMap tile layer.
fn tiles_toggle(app: &App) -> Element<'static, Message> {
    let on = app.online_tiles;
    button(widgets::labelled(
        lucide::globe()
            .size(14)
            .color(if on { theme::primary() } else { theme::text() }),
        "OSM tiles",
    ))
    .padding(Padding::from([7, 10]))
    .style(if on {
        theme::secondary_button
    } else {
        theme::ghost_button
    })
    .on_press(Message::ToggleOnlineTiles(!on))
    .into()
}

fn subtitle(app: &App, nodes: &[MapNode]) -> String {
    if nodes.len() == 1 {
        format!(
            "{} of {} nodes reports a position",
            nodes.len(),
            app.nodes.len()
        )
    } else {
        format!(
            "{} of {} nodes report a position",
            nodes.len(),
            app.nodes.len()
        )
    }
}

fn icon_button(icon: Element<'static, Message>, message: Message) -> Element<'static, Message> {
    button(icon)
        .padding(Padding::from([7, 9]))
        .style(theme::ghost_button)
        .on_press(message)
        .into()
}

fn node_panel(app: &App, nodes: &[MapNode], view: MapView) -> Element<'static, Message> {
    let mut list = column![].spacing(4).width(Length::Fill);
    list = list.push(
        container(text("POSITIONED NODES").size(11).color(theme::text_faint()))
            .width(Length::Fill)
            .padding(Padding::from([6, 8])),
    );
    for node in nodes {
        list = list.push(node_row(app, node, view));
    }

    container(scrollable(list.padding(Padding::from(10))).height(Length::Fill))
        .width(264)
        .height(Length::Fill)
        .style(theme::panel)
        .into()
}

fn node_row(app: &App, node: &MapNode, view: MapView) -> Element<'static, Message> {
    let mut parts = vec![format::node_id(node.num)];
    if !node.is_me {
        if let Some(origin) = app.my_coords() {
            let start = ((origin.0 * 1e7) as i32, (origin.1 * 1e7) as i32);
            let end = ((node.lat * 1e7) as i32, (node.lon * 1e7) as i32);
            parts.push(format::format_distance(
                format::distance_km(start, end),
                app.settings.imperial,
            ));
        }
    }

    let mut title = row![text(node.name.clone()).size(14).color(theme::text())];
    if node.is_me {
        title = title.push(widgets::tag("you", theme::primary()));
    }

    button(
        row![
            dot(node.is_me),
            column![
                title.spacing(6).align_y(Alignment::Center),
                text(parts.join(" · ")).size(11).color(theme::text_muted()),
            ]
            .width(Length::Fill)
            .spacing(2),
        ]
        .spacing(10)
        .align_y(Alignment::Center),
    )
    .width(Length::Fill)
    .padding(Padding::from([8, 10]))
    .style(theme::nav_button(false))
    .on_press(Message::MapViewChanged(MapView {
        lat: node.lat,
        lon: node.lon,
        scale: view.scale,
    }))
    .into()
}

fn dot(is_me: bool) -> Element<'static, Message> {
    let color = if is_me {
        theme::primary()
    } else {
        MARKER_COLOR
    };
    container(Space::new().width(10).height(10))
        .style(move |_| iced::widget::container::Style {
            background: Some(color.into()),
            border: Border {
                color,
                width: 0.0,
                radius: 999.0.into(),
            },
            ..Default::default()
        })
        .into()
}

fn empty(app: &App) -> Element<'_, Message> {
    let hint = format!(
        "{} nodes are known, but none report a GPS position yet. Positions show up here once a node shares one.",
        app.nodes.len()
    );
    container(column![widgets::empty_state(
        lucide::map().size(42).color(theme::text_faint()).into(),
        "No positions yet",
        &hint,
    )])
    .width(Length::Fill)
    .height(Length::Fill)
    .padding(Padding::from(20))
    .style(theme::content)
    .into()
}
