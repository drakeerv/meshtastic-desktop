//! UI icons from the bundled Lucide icon font (via `iced_fonts`).
//!
//! A font is used instead of individual SVG files so icons inherit the
//! surrounding text colour and scale cleanly (no per-size rasterisation).
//! The font bytes are registered once in `main`.

pub use iced_fonts::lucide;
