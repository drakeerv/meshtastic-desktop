Remaining notes:

- Make sure to doc comment everything ig.
- Make logging better/more thorough?
- Add a tray icon/process for background usage/notifications?

Roadmap ideas (roughly priority order):

1. CI (GitHub Actions) + README badges.
   fmt --check, clippy -D warnings and tests on push/PR.
2. Release packaging.
   AppImage + .deb, AUR PKGBUILD, and a tagged release workflow.
3. Logging overhaul.
   Structured tracing, level filter, copy/export from the Logs panel.
4. Tray icon + close-to-tray. (done)
   ksni system tray with show/hide and quit, close-to-tray, notifications
   attributed to the app icon.
5. Host integration. (done)
   Device clock on connect, "Fill from host" timezone, GeoClue host location.
6. Message UX. (done)
   Cross-message search, per-conversation unread badges (persisted per
   device), clear-conversation and per-message delete (both confirmed).
   History now loads from the last device's database at startup, so the
   Messages view works offline and clear/delete persist offline.
7. Waypoints.
   Receive, plot on the map, and send waypoints.
8. Remote admin.
   Configure another node from this one; settings currently target the local device.
9. Neighbor info / mesh topology.
   Visualize the neighbor-info module output on the map.
10. Localization.
    Fluent-based i18n; no scaffolding yet.
11. Accessibility + keyboard shortcuts.
    Done: global shortcuts, Escape-to-dismiss, Tab focus traversal (text
    fields only, since iced buttons are not focusable), high contrast, and a
    live UI scale.
    BLOCKED: real screen-reader support. iced 0.14 exposes no accessibility
    tree (no AccessKit/AT-SPI at all), so Orca sees an empty window. Upstream
    has an unmerged draft PR (iced-rs/iced#3111, accesskit-integration) that
    currently conflicts and is partial. The durable fix is either that PR
    landing or a GTK4/libadwaita UI port (the mt-* backend crates are
    UI-agnostic and would carry over).
