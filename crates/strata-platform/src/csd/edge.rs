//! The window's edges and corners, and the bands that resize from them.

use rune::Cursor;

/// A side or corner of the window, the way a resize goes.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Edge {
    North,
    NorthEast,
    East,
    SouthEast,
    South,
    SouthWest,
    West,
    NorthWest,
}

impl Edge {
    pub const ALL: [Edge; 8] = [
        Edge::North,
        Edge::NorthEast,
        Edge::East,
        Edge::SouthEast,
        Edge::South,
        Edge::SouthWest,
        Edge::West,
        Edge::NorthWest,
    ];

    /// The cursor over this edge's band. Corners show `Move` until Rune has diagonal resize
    /// cursors (upstream #145).
    pub fn cursor(self) -> Cursor {
        match self {
            Edge::East | Edge::West => Cursor::ResizeColumn,
            Edge::North | Edge::South => Cursor::ResizeRow,
            _ => Cursor::Move,
        }
    }

    /// The band's element key.
    pub fn key(self) -> &'static str {
        match self {
            Edge::North => "csd-edge-n",
            Edge::NorthEast => "csd-edge-ne",
            Edge::East => "csd-edge-e",
            Edge::SouthEast => "csd-edge-se",
            Edge::South => "csd-edge-s",
            Edge::SouthWest => "csd-edge-sw",
            Edge::West => "csd-edge-w",
            Edge::NorthWest => "csd-edge-nw",
        }
    }

    /// The `_NET_WM_MOVERESIZE` direction (EWMH: 0 is top-left, then clockwise).
    pub(super) fn net_wm_direction(self) -> u32 {
        match self {
            Edge::NorthWest => 0,
            Edge::North => 1,
            Edge::NorthEast => 2,
            Edge::East => 3,
            Edge::SouthEast => 4,
            Edge::South => 5,
            Edge::SouthWest => 6,
            Edge::West => 7,
        }
    }
}

/// How wide the bands are, in points, inside the window's edges.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Bands {
    /// The side bands' thickness.
    pub thickness: f32,
    /// A corner band's width and height.
    pub corner: f32,
}

impl Default for Bands {
    fn default() -> Self {
        Bands {
            thickness: 6.0,
            corner: 12.0,
        }
    }
}
