// crates/core/src/geometry.rs

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Size {
    pub width: u32,
    pub height: u32,
}

impl Size {
    pub const fn new(width: u32, height: u32) -> Self {
        Self { width, height }
    }

    pub const fn pixels(self) -> u64 {
        self.width as u64 * self.height as u64
    }

    pub const fn megapixels(self) -> f64 {
        self.pixels() as f64 / 1_000_000.0
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Rect {
    pub x: i32,
    pub y: i32,
    pub width: u32,
    pub height: u32,
}

impl Rect {
    pub const fn new(x: i32, y: i32, width: u32, height: u32) -> Self {
        Self {
            x,
            y,
            width,
            height,
        }
    }

    pub const fn is_empty(self) -> bool {
        self.width == 0 || self.height == 0
    }

    pub const fn right(self) -> i32 {
        self.x + self.width as i32
    }

    pub const fn bottom(self) -> i32 {
        self.y + self.height as i32
    }

    pub fn intersect(self, other: Rect) -> Option<Rect> {
        let x = self.x.max(other.x);
        let y = self.y.max(other.y);
        let right = self.right().min(other.right());
        let bottom = self.bottom().min(other.bottom());
        if right <= x || bottom <= y {
            None
        } else {
            Some(Rect::new(x, y, (right - x) as u32, (bottom - y) as u32))
        }
    }

    pub fn union(self, other: Rect) -> Rect {
        if self.is_empty() {
            return other;
        }
        if other.is_empty() {
            return self;
        }
        let x = self.x.min(other.x);
        let y = self.y.min(other.y);
        let right = self.right().max(other.right());
        let bottom = self.bottom().max(other.bottom());
        Rect::new(x, y, (right - x) as u32, (bottom - y) as u32)
    }

    pub fn translated(self, dx: i32, dy: i32) -> Rect {
        Rect::new(self.x + dx, self.y + dy, self.width, self.height)
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum Selection {
    Rect(Rect),
    Ellipse(Rect),
    Lasso(Vec<(f32, f32)>),
}

impl Selection {
    pub fn bounds(&self) -> Rect {
        match self {
            Selection::Rect(r) | Selection::Ellipse(r) => *r,
            Selection::Lasso(points) => {
                if points.is_empty() {
                    return Rect::default();
                }
                let mut min_x = f32::INFINITY;
                let mut min_y = f32::INFINITY;
                let mut max_x = f32::NEG_INFINITY;
                let mut max_y = f32::NEG_INFINITY;
                for (x, y) in points {
                    min_x = min_x.min(*x);
                    min_y = min_y.min(*y);
                    max_x = max_x.max(*x);
                    max_y = max_y.max(*y);
                }
                Rect::new(
                    min_x.floor() as i32,
                    min_y.floor() as i32,
                    (max_x - min_x).ceil().max(0.0) as u32,
                    (max_y - min_y).ceil().max(0.0) as u32,
                )
            }
        }
    }

    pub fn contains(&self, x: f32, y: f32) -> bool {
        match self {
            Selection::Rect(r) => {
                x >= r.x as f32
                    && y >= r.y as f32
                    && x < r.right() as f32
                    && y < r.bottom() as f32
            }
            Selection::Ellipse(r) => {
                if r.is_empty() {
                    return false;
                }
                let cx = r.x as f32 + r.width as f32 / 2.0;
                let cy = r.y as f32 + r.height as f32 / 2.0;
                let rx = r.width as f32 / 2.0;
                let ry = r.height as f32 / 2.0;
                let dx = (x - cx) / rx;
                let dy = (y - cy) / ry;
                dx * dx + dy * dy <= 1.0
            }
            Selection::Lasso(points) => {
                let n = points.len();
                if n < 3 {
                    return false;
                }
                let mut inside = false;
                let mut j = n - 1;
                for i in 0..n {
                    let (xi, yi) = points[i];
                    let (xj, yj) = points[j];
                    if (yi > y) != (yj > y) && x < (xj - xi) * (y - yi) / (yj - yi) + xi {
                        inside = !inside;
                    }
                    j = i;
                }
                inside
            }
        }
    }
}

// ---- 标记（markup annotations）------------------------------------------
//
// Vector annotations drawn ON TOP of the flattened image at render / export
// time. They never touch pixels until composition, so adding / moving /
// restyling one is cheap and fully undoable. Coordinates are canvas-space
// f32; stroke widths are image pixels at full resolution.

