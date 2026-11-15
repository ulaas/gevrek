//! Geometry of what decorations leave for the window content.

/// Content rect `(x, y, width, height)` left for a container's windows when a
/// stackbar of `bar_height` pixels reserves the top strip of `rect`. The
/// reservation is clamped so the content always keeps
/// at least one pixel; a non-positive bar height reserves nothing.
pub fn stackbar_content_rect(rect: (i32, i32, i32, i32), bar_height: i32) -> (i32, i32, i32, i32) {
    let (x, y, width, height) = rect;
    let reserved = bar_height.clamp(0, (height - 1).max(0));
    (x, y + reserved, width, height - reserved)
}

/// Content rect `(x, y, width, height)` left for a window when invisible
/// borders reserve `size` pixels on every edge: the same band a border of that width would occupy,
/// but nothing is drawn. The reservation is clamped so the content always
/// keeps at least one pixel; a non-positive size reserves nothing.
pub fn invisible_border_content_rect(
    rect: (i32, i32, i32, i32),
    size: i32,
) -> (i32, i32, i32, i32) {
    let (x, y, width, height) = rect;
    let horizontal = size.clamp(0, (width - 1).max(0) / 2);
    let vertical = size.clamp(0, (height - 1).max(0) / 2);
    (
        x + horizontal,
        y + vertical,
        width - horizontal * 2,
        height - vertical * 2,
    )
}
