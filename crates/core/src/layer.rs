// crates/core/src/layer.rs
use super::*;

#[derive(Debug, Clone, PartialEq)]
pub struct Layer {
    pub name: String,
    pub image: RgbaImage,
    pub offset: (i32, i32),
    pub opacity: f32,
    pub visible: bool,
    /// UI-only guard: locked layers reject move / scale / delete in the
    /// bridge. Plain flag, not part of undo history.
    pub locked: bool,
}

impl Layer {
    pub fn new(name: impl Into<String>, image: RgbaImage) -> Self {
        Self {
            name: name.into(),
            image,
            offset: (0, 0),
            opacity: 1.0,
            visible: true,
            locked: false,
        }
    }

    pub fn bounds(&self) -> Rect {
        Rect::new(
            self.offset.0,
            self.offset.1,
            self.image.width(),
            self.image.height(),
        )
    }
}

