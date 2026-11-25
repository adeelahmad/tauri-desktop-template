//! Keeping a restored window on screen.
//!
//! The web app's window comes back at the size and place it was left ("remember the
//! window"). When a display was removed, or the displays were rearranged, that place can be
//! partly or wholly off every screen. [`fit`] moves the window, and shrinks it when it is
//! larger than the display, so that it lies on the display it overlaps most (the first
//! display, usually the main one, when it overlaps none). A window that is fully visible,
//! also one that spans two displays, is left where it is.
//!
//! All values are physical pixels in the desktop's coordinates, as Tauri gives them.
//!
//! # Example
//!
//! ```
//! use desktop_core::wrapper::screen::{fit, Rect};
//!
//! let laptop = Rect { x: 0, y: 0, width: 1440, height: 900 };
//! // Left on an external display to the right, which is gone now.
//! let saved = Rect { x: 2000, y: 100, width: 1280, height: 800 };
//! assert_eq!(fit(saved, &[laptop]), Some(Rect { x: 160, y: 100, width: 1280, height: 800 }));
//! // Fully on screen: stays.
//! assert_eq!(fit(Rect { x: 10, y: 10, width: 800, height: 600 }, &[laptop]), None);
//! ```

/// A rectangle: the window's outer frame or a display's area.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Rect {
    /// Left edge.
    pub x: i32,
    /// Top edge.
    pub y: i32,
    /// Width.
    pub width: u32,
    /// Height.
    pub height: u32,
}

impl Rect {
    fn right(&self) -> i64 {
        self.x as i64 + self.width as i64
    }

    fn bottom(&self) -> i64 {
        self.y as i64 + self.height as i64
    }

    fn area(&self) -> u64 {
        self.width as u64 * self.height as u64
    }

    /// The area this rectangle shares with `other` (0 when they do not meet).
    pub fn overlap(&self, other: &Rect) -> u64 {
        let w = self.right().min(other.right()) - (self.x as i64).max(other.x as i64);
        let h = self.bottom().min(other.bottom()) - (self.y as i64).max(other.y as i64);
        if w <= 0 || h <= 0 {
            0
        } else {
            w as u64 * h as u64
        }
    }
}

/// Where the window should go so the user can see it: `None` when it is fully visible
/// already (or there is nothing to fit: no display, or an empty window), else the window
/// moved (and shrunk to the display, when larger) onto the display it overlaps most.
pub fn fit(window: Rect, displays: &[Rect]) -> Option<Rect> {
    let displays: Vec<&Rect> = displays.iter().filter(|d| d.area() > 0).collect();
    if window.area() == 0 || displays.is_empty() {
        return None;
    }
    // Displays do not overlap (mirrored ones are the same rectangle, counted once).
    let mut seen: Vec<&Rect> = Vec::new();
    let visible: u64 = displays
        .iter()
        .filter(|d| {
            let new = !seen.contains(d);
            seen.push(d);
            new
        })
        .map(|d| window.overlap(d))
        .sum();
    if visible >= window.area() {
        return None;
    }
    let best = displays.iter().max_by_key(|d| window.overlap(d)).filter(|d| window.overlap(d) > 0).unwrap_or(&displays[0]);
    let width = window.width.min(best.width);
    let height = window.height.min(best.height);
    let clamp = |at: i32, size: u32, start: i32, len: u32| -> i32 {
        let max = start as i64 + len as i64 - size as i64;
        (at as i64).clamp(start as i64, max) as i32
    };
    Some(Rect { x: clamp(window.x, width, best.x, best.width), y: clamp(window.y, height, best.y, best.height), width, height })
}

#[cfg(test)]
mod tests {
    use super::*;

    const MAIN: Rect = Rect { x: 0, y: 0, width: 1920, height: 1080 };
    const LEFT: Rect = Rect { x: -1280, y: 0, width: 1280, height: 1024 };

    fn r(x: i32, y: i32, width: u32, height: u32) -> Rect {
        Rect { x, y, width, height }
    }

    #[test]
    fn a_visible_window_stays() {
        assert_eq!(fit(r(100, 100, 800, 600), &[MAIN]), None);
        assert_eq!(fit(MAIN, &[MAIN]), None, "exactly the display");
        // Across two displays, fully covered: stays.
        assert_eq!(fit(r(-400, 100, 800, 600), &[MAIN, LEFT]), None);
        // Mirrored displays do not count twice.
        assert_eq!(fit(r(1500, 100, 800, 600), &[MAIN, MAIN]), Some(r(1120, 100, 800, 600)));
    }

    #[test]
    fn a_window_on_a_removed_display_comes_back() {
        // It was on LEFT, which is gone: onto the main display, same size.
        assert_eq!(fit(r(-1000, 200, 800, 600), &[MAIN]), Some(r(0, 200, 800, 600)));
        // Far away below: the first display, clamped to its bottom edge.
        assert_eq!(fit(r(50, 5000, 800, 600), &[MAIN]), Some(r(50, 480, 800, 600)));
    }

    #[test]
    fn a_partly_hidden_window_goes_to_the_display_it_overlaps_most() {
        // Mostly on LEFT, its top cut off.
        assert_eq!(fit(r(-1200, -100, 800, 600), &[MAIN, LEFT]), Some(r(-1200, 0, 800, 600)));
        // Hanging off the right edge of MAIN.
        assert_eq!(fit(r(1600, 100, 800, 600), &[LEFT, MAIN]), Some(r(1120, 100, 800, 600)));
    }

    #[test]
    fn a_window_larger_than_the_display_is_shrunk() {
        let small = r(0, 0, 1280, 800);
        assert_eq!(fit(r(-10, -10, 2560, 1440), &[small]), Some(small));
    }

    #[test]
    fn nothing_to_fit() {
        assert_eq!(fit(r(0, 0, 0, 0), &[MAIN]), None);
        assert_eq!(fit(r(5000, 5000, 800, 600), &[]), None);
        assert_eq!(fit(r(5000, 5000, 800, 600), &[r(0, 0, 0, 0)]), None);
        assert_eq!(r(0, 0, 10, 10).overlap(&r(10, 0, 10, 10)), 0, "touching edges do not overlap");
        assert_eq!(r(i32::MAX - 5, 0, u32::MAX, 10).overlap(&MAIN), 0, "no overflow");
    }
}
