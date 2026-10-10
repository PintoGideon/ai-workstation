//! Pure pieces of the TUI split view (DESIGN.md §4.1): tile layout, paging,
//! focus movement, hit testing and key routing. No `App`, no tmux — the
//! stateful side lives in `app/fork_split.rs`, drawing in `ui/fork_split.rs`.

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::layout::Rect;

/// Tiles shown at once; more sessions than this are paged.
pub const PAGE_SIZE: usize = 4;

/// Rows reserved under the tiles for the key-hint footer.
pub const FOOTER_ROWS: u16 = 1;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Direction {
    Left,
    Right,
    Up,
    Down,
}

/// Number of pages for `total` tiles (at least one, so an empty split view
/// still has a page to show its message on).
pub fn page_count(total: usize) -> usize {
    total.div_ceil(PAGE_SIZE).max(1)
}

/// Indices (into the full tile list) shown on `page`.
pub fn page_range(total: usize, page: usize) -> std::ops::Range<usize> {
    let start = (page * PAGE_SIZE).min(total);
    let end = (start + PAGE_SIZE).min(total);
    start..end
}

/// The page that shows tile `index`.
pub fn page_of(index: usize) -> usize {
    index / PAGE_SIZE
}

/// Split `area` into the outer rectangles of `n` tiles (1–4):
/// 1 = full, 2 = side by side, 3 = one large on the left plus two stacked on
/// the right, 4 = 2×2. More than 4 is clamped to 4 (callers page instead).
pub fn tile_rects(area: Rect, n: usize) -> Vec<Rect> {
    let n = n.min(PAGE_SIZE);
    if n == 0 || area.width == 0 || area.height == 0 {
        return Vec::new();
    }
    let left_w = area.width / 2;
    let right_w = area.width - left_w;
    let top_h = area.height / 2;
    let bottom_h = area.height - top_h;
    let left = Rect::new(area.x, area.y, left_w, area.height);
    let right = Rect::new(area.x + left_w, area.y, right_w, area.height);
    match n {
        1 => vec![area],
        2 => vec![left, right],
        3 => vec![
            left,
            Rect::new(area.x + left_w, area.y, right_w, top_h),
            Rect::new(area.x + left_w, area.y + top_h, right_w, bottom_h),
        ],
        _ => vec![
            Rect::new(area.x, area.y, left_w, top_h),
            Rect::new(area.x + left_w, area.y, right_w, top_h),
            Rect::new(area.x, area.y + top_h, left_w, bottom_h),
            Rect::new(area.x + left_w, area.y + top_h, right_w, bottom_h),
        ],
    }
}

/// The area tiles are laid out in: the whole frame minus the footer.
pub fn tiles_area(frame: Rect) -> Rect {
    Rect::new(
        frame.x,
        frame.y,
        frame.width,
        frame.height.saturating_sub(FOOTER_ROWS),
    )
}

/// The live-terminal area inside a tile: its border (the top border carries
/// the header) taken off every side. This is the size the tile's tmux window
/// is resized to.
pub fn tile_content(rect: Rect) -> Rect {
    Rect::new(
        rect.x.saturating_add(1),
        rect.y.saturating_add(1),
        rect.width.saturating_sub(2),
        rect.height.saturating_sub(2),
    )
}

/// Which tile `(col, row)` falls in.
pub fn tile_at(rects: &[Rect], col: u16, row: u16) -> Option<usize> {
    rects.iter().position(|r| {
        col >= r.x && col < r.x.saturating_add(r.width) && row >= r.y && row < r.y + r.height
    })
}

/// Move focus from tile `from` towards `dir`: the nearest tile lying entirely
/// on that side, preferring ones that overlap the focused tile across the
/// movement axis. Stays put at an edge.
pub fn move_focus(rects: &[Rect], from: usize, dir: Direction) -> usize {
    let Some(current) = rects.get(from) else {
        return from;
    };
    let (cx, cy) = center2(current);
    let mut best: Option<(u32, usize)> = None;
    for (i, r) in rects.iter().enumerate() {
        if i == from {
            continue;
        }
        let beyond = match dir {
            Direction::Left => r.x + r.width <= current.x,
            Direction::Right => r.x >= current.x + current.width,
            Direction::Up => r.y + r.height <= current.y,
            Direction::Down => r.y >= current.y + current.height,
        };
        if !beyond {
            continue;
        }
        let overlaps = match dir {
            Direction::Left | Direction::Right => {
                r.y < current.y + current.height && current.y < r.y + r.height
            }
            Direction::Up | Direction::Down => {
                r.x < current.x + current.width && current.x < r.x + r.width
            }
        };
        let (rx, ry) = center2(r);
        let (primary, secondary) = match dir {
            Direction::Left | Direction::Right => (cx.abs_diff(rx), cy.abs_diff(ry)),
            Direction::Up | Direction::Down => (cy.abs_diff(ry), cx.abs_diff(rx)),
        };
        // Overlapping tiles always beat non-overlapping ones.
        let score = if overlaps { 0 } else { 1 << 30 } + primary * 4 + secondary;
        if best.is_none_or(|(s, _)| score < s) {
            best = Some((score, i));
        }
    }
    best.map_or(from, |(_, i)| i)
}

/// Doubled centre (avoids halves) of a rect.
fn center2(r: &Rect) -> (u32, u32) {
    (
        2 * r.x as u32 + r.width as u32,
        2 * r.y as u32 + r.height as u32,
    )
}

/// What a key press means in split view.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SplitAction {
    /// Send the key to the focused tile's agent.
    Forward,
    /// `Ctrl+Space`: wait for a split-view command key.
    Leader,
    Focus(Direction),
    /// Toggle the focused tile full screen (the regular single view).
    Zoom,
    /// Back to the dashboard.
    Leave,
    NextPage,
    PrevPage,
    /// Focus the tile whose agent has needed attention the longest.
    FocusAttention,
    /// Ignore (unknown leader command, key release, …).
    Nothing,
}

fn is_ctrl(key: &KeyEvent, c: char) -> bool {
    key.modifiers.contains(KeyModifiers::CONTROL) && key.code == KeyCode::Char(c)
}

/// Route a key. `leader` is whether `Ctrl+Space` was pressed just before.
pub fn route_key(leader: bool, key: &KeyEvent) -> SplitAction {
    if leader {
        return match key.code {
            KeyCode::Left | KeyCode::Char('h') => SplitAction::Focus(Direction::Left),
            KeyCode::Right | KeyCode::Char('l') => SplitAction::Focus(Direction::Right),
            KeyCode::Up | KeyCode::Char('k') => SplitAction::Focus(Direction::Up),
            KeyCode::Down | KeyCode::Char('j') => SplitAction::Focus(Direction::Down),
            KeyCode::Char('z') => SplitAction::Zoom,
            KeyCode::Char('v') | KeyCode::Char('q') => SplitAction::Leave,
            KeyCode::Char(']') | KeyCode::Char('n') => SplitAction::NextPage,
            KeyCode::Char('[') | KeyCode::Char('p') => SplitAction::PrevPage,
            KeyCode::Char('i') => SplitAction::FocusAttention,
            // A second Ctrl+Space sends a literal one to the agent.
            KeyCode::Char(' ') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                SplitAction::Forward
            }
            _ => SplitAction::Nothing,
        };
    }
    if is_ctrl(key, ' ') {
        return SplitAction::Leader;
    }
    if is_ctrl(key, 'q') {
        return SplitAction::Leave;
    }
    SplitAction::Forward
}

#[cfg(test)]
mod tests {
    use super::*;

    fn area() -> Rect {
        Rect::new(0, 0, 200, 54)
    }

    fn covers_exactly(area: Rect, rects: &[Rect]) {
        let total: u32 = rects.iter().map(|r| r.width as u32 * r.height as u32).sum();
        assert_eq!(total, area.width as u32 * area.height as u32, "{rects:?}");
        for (i, a) in rects.iter().enumerate() {
            assert!(area.contains(a.as_position()), "{a:?} outside");
            for b in &rects[i + 1..] {
                assert!(!a.intersects(*b), "{a:?} overlaps {b:?}");
            }
        }
    }

    #[test]
    fn one_tile_fills_the_area() {
        assert_eq!(tile_rects(area(), 1), vec![area()]);
        assert!(tile_rects(area(), 0).is_empty());
        assert!(tile_rects(Rect::new(0, 0, 0, 10), 2).is_empty());
    }

    #[test]
    fn two_tiles_side_by_side() {
        let rects = tile_rects(area(), 2);
        assert_eq!(
            rects,
            vec![Rect::new(0, 0, 100, 54), Rect::new(100, 0, 100, 54)]
        );
        covers_exactly(area(), &rects);
    }

    #[test]
    fn three_tiles_one_large_two_stacked() {
        let odd = Rect::new(3, 2, 101, 41);
        let rects = tile_rects(odd, 3);
        assert_eq!(rects[0], Rect::new(3, 2, 50, 41));
        assert_eq!(rects[1], Rect::new(53, 2, 51, 20));
        assert_eq!(rects[2], Rect::new(53, 22, 51, 21));
        covers_exactly(odd, &rects);
    }

    #[test]
    fn four_tiles_two_by_two_and_more_clamps() {
        let rects = tile_rects(area(), 4);
        assert_eq!(
            rects,
            vec![
                Rect::new(0, 0, 100, 27),
                Rect::new(100, 0, 100, 27),
                Rect::new(0, 27, 100, 27),
                Rect::new(100, 27, 100, 27),
            ]
        );
        covers_exactly(area(), &rects);
        assert_eq!(tile_rects(area(), 7), rects);
    }

    #[test]
    fn content_is_inside_the_border_and_footer_is_reserved() {
        let frame = Rect::new(0, 0, 120, 40);
        let tiles = tiles_area(frame);
        assert_eq!(tiles, Rect::new(0, 0, 120, 39));
        let rects = tile_rects(tiles, 2);
        assert_eq!(tile_content(rects[0]), Rect::new(1, 1, 58, 37));
        assert_eq!(tile_content(Rect::new(0, 0, 1, 1)).width, 0);
    }

    #[test]
    fn paging_covers_every_tile_once() {
        assert_eq!(page_count(0), 1);
        assert_eq!(page_count(4), 1);
        assert_eq!(page_count(5), 2);
        assert_eq!(page_count(9), 3);
        assert_eq!(page_range(9, 0), 0..4);
        assert_eq!(page_range(9, 1), 4..8);
        assert_eq!(page_range(9, 2), 8..9);
        assert_eq!(page_range(9, 5), 9..9);
        assert_eq!(page_of(3), 0);
        assert_eq!(page_of(4), 1);
        let shown: usize = (0..page_count(9)).map(|p| page_range(9, p).len()).sum();
        assert_eq!(shown, 9);
    }

    #[test]
    fn focus_moves_across_the_2x2_grid_and_stops_at_edges() {
        let rects = tile_rects(area(), 4);
        assert_eq!(move_focus(&rects, 0, Direction::Right), 1);
        assert_eq!(move_focus(&rects, 0, Direction::Down), 2);
        assert_eq!(move_focus(&rects, 3, Direction::Left), 2);
        assert_eq!(move_focus(&rects, 3, Direction::Up), 1);
        assert_eq!(move_focus(&rects, 0, Direction::Left), 0);
        assert_eq!(move_focus(&rects, 0, Direction::Up), 0);
        assert_eq!(move_focus(&rects, 3, Direction::Down), 3);
    }

    #[test]
    fn focus_moves_in_the_three_tile_layout() {
        let rects = tile_rects(area(), 3);
        // From the big left tile, right goes to the top-right tile.
        assert_eq!(move_focus(&rects, 0, Direction::Right), 1);
        assert_eq!(move_focus(&rects, 1, Direction::Down), 2);
        assert_eq!(move_focus(&rects, 2, Direction::Up), 1);
        assert_eq!(move_focus(&rects, 2, Direction::Left), 0);
        assert_eq!(move_focus(&rects, 1, Direction::Left), 0);
        assert_eq!(move_focus(&rects, 0, Direction::Down), 0);
    }

    #[test]
    fn focus_with_one_or_two_tiles() {
        let one = tile_rects(area(), 1);
        assert_eq!(move_focus(&one, 0, Direction::Right), 0);
        let two = tile_rects(area(), 2);
        assert_eq!(move_focus(&two, 0, Direction::Right), 1);
        assert_eq!(move_focus(&two, 1, Direction::Left), 0);
        assert_eq!(move_focus(&two, 1, Direction::Up), 1);
        assert_eq!(move_focus(&two, 9, Direction::Left), 9);
    }

    #[test]
    fn clicks_hit_the_right_tile() {
        let rects = tile_rects(area(), 4);
        assert_eq!(tile_at(&rects, 0, 0), Some(0));
        assert_eq!(tile_at(&rects, 150, 10), Some(1));
        assert_eq!(tile_at(&rects, 99, 27), Some(2));
        assert_eq!(tile_at(&rects, 199, 53), Some(3));
        assert_eq!(tile_at(&rects, 200, 10), None);
        assert_eq!(tile_at(&rects, 10, 54), None);
    }

    fn key(code: KeyCode) -> KeyEvent {
        KeyEvent::new(code, KeyModifiers::NONE)
    }

    fn ctrl(c: char) -> KeyEvent {
        KeyEvent::new(KeyCode::Char(c), KeyModifiers::CONTROL)
    }

    #[test]
    fn plain_keys_go_to_the_focused_agent() {
        for k in [
            key(KeyCode::Char('a')),
            key(KeyCode::Enter),
            key(KeyCode::Left),
            key(KeyCode::Esc),
            ctrl('c'),
            key(KeyCode::Char('z')),
        ] {
            assert_eq!(route_key(false, &k), SplitAction::Forward, "{k:?}");
        }
    }

    #[test]
    fn ctrl_space_and_ctrl_q_are_split_view_keys() {
        assert_eq!(route_key(false, &ctrl(' ')), SplitAction::Leader);
        assert_eq!(route_key(false, &ctrl('q')), SplitAction::Leave);
    }

    #[test]
    fn leader_commands() {
        let cases = [
            (key(KeyCode::Left), SplitAction::Focus(Direction::Left)),
            (key(KeyCode::Right), SplitAction::Focus(Direction::Right)),
            (key(KeyCode::Up), SplitAction::Focus(Direction::Up)),
            (key(KeyCode::Down), SplitAction::Focus(Direction::Down)),
            (
                key(KeyCode::Char('l')),
                SplitAction::Focus(Direction::Right),
            ),
            (key(KeyCode::Char('z')), SplitAction::Zoom),
            (key(KeyCode::Char('v')), SplitAction::Leave),
            (key(KeyCode::Char('q')), SplitAction::Leave),
            (key(KeyCode::Char(']')), SplitAction::NextPage),
            (key(KeyCode::Char('[')), SplitAction::PrevPage),
            (key(KeyCode::Char('i')), SplitAction::FocusAttention),
            (ctrl(' '), SplitAction::Forward),
            (key(KeyCode::Char('x')), SplitAction::Nothing),
        ];
        for (k, expected) in cases {
            assert_eq!(route_key(true, &k), expected, "{k:?}");
        }
    }
}
