- macOS key-window notification handling in crates/platform/ui/src/window_activation.rs
  GPUI 0.2.2 (Apache-2.0), https://crates.io/crates/gpui/0.2.2,
  based on Zed commit 69e2130295c2649963eb639fc70b4f2ee8ea1624,
  crates/gpui/src/platform/mac/window.rs. The adapter preserves GPUI's
  balancing resignation for a spurious become-key notification and sends it
  before entering the original delegate so AppKit can synchronously reenter.
