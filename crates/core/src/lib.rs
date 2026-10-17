use image::RgbaImage;

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

#[derive(Debug, Clone, PartialEq)]
pub struct Layer {
    pub name: String,
    pub image: RgbaImage,
    pub offset: (i32, i32),
    pub opacity: f32,
    pub visible: bool,
}

impl Layer {
    pub fn new(name: impl Into<String>, image: RgbaImage) -> Self {
        Self {
            name: name.into(),
            image,
            offset: (0, 0),
            opacity: 1.0,
            visible: true,
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

#[derive(Debug, Clone)]
pub struct Document {
    pub source: Option<std::path::PathBuf>,
    pub canvas: Size,
    pub base: RgbaImage,
    /// Visible region of `base`, in `base` coordinates. Cropping only shrinks
    /// this rect (no pixel copy), so crop is O(1) and reversible.
    pub view: Rect,
    pub layers: Vec<Layer>,
    /// Auto-naming counter: new layers (extract/paste) are named by their
    /// creation order ("1", "2", ...). Not part of snapshots on purpose, so
    /// names keep increasing across undo/redo.
    next_layer_seq: u32,
    history: History,
}

#[derive(Debug, thiserror::Error, PartialEq)]
pub enum EditError {
    #[error("layer index {0} out of range")]
    LayerIndex(usize),
    #[error("selection is empty")]
    EmptySelection,
    #[error("crop rect is empty or outside the image")]
    EmptyCrop,
    #[error("target size must be at least 1x1")]
    InvalidCanvasSize,
    #[error("nothing to undo")]
    NothingToUndo,
    #[error("nothing to redo")]
    NothingToRedo,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Transform {
    Rotate90Cw,
    Rotate90Ccw,
    Rotate180,
    FlipHorizontal,
    FlipVertical,
}

impl Transform {
    pub const fn inverted(self) -> Self {
        match self {
            Transform::Rotate90Cw => Transform::Rotate90Ccw,
            Transform::Rotate90Ccw => Transform::Rotate90Cw,
            Transform::Rotate180 => Transform::Rotate180,
            Transform::FlipHorizontal => Transform::FlipHorizontal,
            Transform::FlipVertical => Transform::FlipVertical,
        }
    }

    const fn swaps_axes(self) -> bool {
        matches!(self, Transform::Rotate90Cw | Transform::Rotate90Ccw)
    }

    fn apply_to_image(self, image: &RgbaImage) -> RgbaImage {
        match self {
            Transform::Rotate90Cw => image::imageops::rotate90(image),
            Transform::Rotate90Ccw => image::imageops::rotate270(image),
            Transform::Rotate180 => image::imageops::rotate180(image),
            Transform::FlipHorizontal => image::imageops::flip_horizontal(image),
            Transform::FlipVertical => image::imageops::flip_vertical(image),
        }
    }
}

fn transform_rect(rect: Rect, transform: Transform, canvas: Size) -> Rect {
    let (cw, ch) = (canvas.width as i32, canvas.height as i32);
    let (lw, lh) = (rect.width as i32, rect.height as i32);
    let (ox, oy) = (rect.x, rect.y);
    let (x, y, width, height) = match transform {
        Transform::Rotate90Cw => (ch - (oy + lh), ox, lh, lw),
        Transform::Rotate90Ccw => (oy, cw - (ox + lw), lh, lw),
        Transform::Rotate180 => (cw - (ox + lw), ch - (oy + lh), lw, lh),
        Transform::FlipHorizontal => (cw - (ox + lw), oy, lw, lh),
        Transform::FlipVertical => (ox, ch - (oy + lh), lw, lh),
    };
    Rect::new(x, y, width.max(0) as u32, height.max(0) as u32)
}

fn transform_document(doc: &mut Document, transform: Transform) {
    let canvas = doc.canvas;
    for layer in &mut doc.layers {
        let bounds = layer.bounds();
        let (ox, oy) = (bounds.x, bounds.y);
        let (lw, lh) = (bounds.width as i32, bounds.height as i32);
        let (cw, ch) = (canvas.width as i32, canvas.height as i32);
        layer.image = transform.apply_to_image(&layer.image);
        layer.offset = match transform {
            Transform::Rotate90Cw => (ch - (oy + lh), ox),
            Transform::Rotate90Ccw => (oy, cw - (ox + lw)),
            Transform::Rotate180 => (cw - (ox + lw), ch - (oy + lh)),
            Transform::FlipHorizontal => (cw - (ox + lw), oy),
            Transform::FlipVertical => (ox, ch - (oy + lh)),
        };
    }
    // `view` lives in base coordinates, so it maps against the base dimensions
    // (layers use canvas dimensions, since their offsets are canvas-relative).
    let base_before = Size::new(doc.base.width(), doc.base.height());
    doc.view = transform_rect(doc.view, transform, base_before);
    doc.base = transform.apply_to_image(&doc.base);
    if transform.swaps_axes() {
        doc.canvas = Size::new(canvas.height, canvas.width);
    }
}

/// Full document state taken before a pixel-modifying op, so undo can restore
/// it exactly.
#[derive(Debug, Clone, PartialEq)]
pub struct DocumentSnapshot {
    pub canvas: Size,
    pub base: RgbaImage,
    pub view: Rect,
    pub layers: Vec<Layer>,
}

impl Document {
    pub fn snapshot(&self) -> DocumentSnapshot {
        DocumentSnapshot {
            canvas: self.canvas,
            base: self.base.clone(),
            view: self.view,
            layers: self.layers.clone(),
        }
    }

    /// Replace everything except history (used when undoing a pixel op).
    pub fn restore(&mut self, snapshot: DocumentSnapshot) {
        self.canvas = snapshot.canvas;
        self.base = snapshot.base;
        self.view = snapshot.view;
        self.layers = snapshot.layers;
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum EditOp {
    TransformDocument { transform: Transform },
    CropDocument { from: Rect, to: Rect },
    ResizeDocument { from: DocumentSnapshot, to: Size },
    AddLayer { layer: Layer, index: usize },
    RemoveLayer { layer: Layer, index: usize },
    MoveLayer { from: usize, to: usize },
    SetVisible { index: usize, from: bool, to: bool },
    SetOpacity { index: usize, from: f32, to: f32 },
    TranslateLayer { index: usize, from: (i32, i32), to: (i32, i32) },
    RenameLayer { index: usize, from: String, to: String },
    /// Non-destructive "delete selection pixels": the base pixels under the
    /// selection are made fully transparent while layers stay untouched. The
    /// pre-op snapshot makes undo exact (same pattern as `ResizeDocument`).
    EraseBasePixels { from: DocumentSnapshot, selection: Selection },
    /// Scale a layer's image to `to` (Lanczos3). The offset (top-left corner)
    /// is kept, so only the content size changes. The pre-op snapshot makes
    /// undo exact (same pattern as `ResizeDocument`).
    ScaleLayer { index: usize, from: DocumentSnapshot, to: Size },
}

impl EditOp {
    pub fn apply(&self, doc: &mut Document) -> Result<(), EditError> {
        match self {
            EditOp::TransformDocument { transform } => {
                transform_document(doc, *transform);
                Ok(())
            }
            EditOp::CropDocument { to, .. } => {
                apply_view(doc, *to);
                Ok(())
            }
            EditOp::ResizeDocument { to, .. } => {
                apply_resize(doc, *to);
                Ok(())
            }
            EditOp::AddLayer { layer, index } => {
                if *index > doc.layers.len() {
                    return Err(EditError::LayerIndex(*index));
                }
                doc.layers.insert(*index, layer.clone());
                Ok(())
            }
            EditOp::RemoveLayer { index, .. } => {
                if *index >= doc.layers.len() {
                    return Err(EditError::LayerIndex(*index));
                }
                doc.layers.remove(*index);
                Ok(())
            }
            EditOp::MoveLayer { from, to } => {
                if *from >= doc.layers.len() || *to >= doc.layers.len() {
                    return Err(EditError::LayerIndex((*from).max(*to)));
                }
                let layer = doc.layers.remove(*from);
                doc.layers.insert(*to, layer);
                Ok(())
            }
            EditOp::SetVisible { index, to, .. } => {
                let layer = doc
                    .layers
                    .get_mut(*index)
                    .ok_or(EditError::LayerIndex(*index))?;
                layer.visible = *to;
                Ok(())
            }
            EditOp::SetOpacity { index, to, .. } => {
                let layer = doc
                    .layers
                    .get_mut(*index)
                    .ok_or(EditError::LayerIndex(*index))?;
                layer.opacity = to.clamp(0.0, 1.0);
                Ok(())
            }
            EditOp::TranslateLayer { index, to, .. } => {
                let layer = doc
                    .layers
                    .get_mut(*index)
                    .ok_or(EditError::LayerIndex(*index))?;
                layer.offset = *to;
                Ok(())
            }
            EditOp::RenameLayer { index, to, .. } => {
                let layer = doc
                    .layers
                    .get_mut(*index)
                    .ok_or(EditError::LayerIndex(*index))?;
                layer.name = to.clone();
                Ok(())
            }
            EditOp::EraseBasePixels { selection, .. } => {
                erase_base_pixels(doc, selection);
                Ok(())
            }
            EditOp::ScaleLayer { index, to, .. } => {
                let layer = doc
                    .layers
                    .get_mut(*index)
                    .ok_or(EditError::LayerIndex(*index))?;
                if to.width < 1 || to.height < 1 {
                    return Err(EditError::InvalidCanvasSize);
                }
                layer.image = image::imageops::resize(
                    &layer.image,
                    to.width,
                    to.height,
                    image::imageops::FilterType::Lanczos3,
                );
                Ok(())
            }
        }
    }

    pub fn revert(&self, doc: &mut Document) -> Result<(), EditError> {
        match self {
            EditOp::TransformDocument { transform } => {
                transform_document(doc, transform.inverted());
                Ok(())
            }
            EditOp::CropDocument { from, .. } => {
                apply_view(doc, *from);
                Ok(())
            }
            EditOp::ResizeDocument { from, .. } => {
                doc.restore(from.clone());
                Ok(())
            }
            EditOp::AddLayer { index, .. } => {
                if *index >= doc.layers.len() {
                    return Err(EditError::LayerIndex(*index));
                }
                doc.layers.remove(*index);
                Ok(())
            }
            EditOp::RemoveLayer { layer, index } => {
                if *index > doc.layers.len() {
                    return Err(EditError::LayerIndex(*index));
                }
                doc.layers.insert(*index, layer.clone());
                Ok(())
            }
            EditOp::MoveLayer { from, to } => {
                if *from >= doc.layers.len() || *to >= doc.layers.len() {
                    return Err(EditError::LayerIndex((*from).max(*to)));
                }
                let layer = doc.layers.remove(*to);
                doc.layers.insert(*from, layer);
                Ok(())
            }
            EditOp::SetVisible { index, from, .. } => {
                let layer = doc
                    .layers
                    .get_mut(*index)
                    .ok_or(EditError::LayerIndex(*index))?;
                layer.visible = *from;
                Ok(())
            }
            EditOp::SetOpacity { index, from, .. } => {
                let layer = doc
                    .layers
                    .get_mut(*index)
                    .ok_or(EditError::LayerIndex(*index))?;
                layer.opacity = *from;
                Ok(())
            }
            EditOp::TranslateLayer { index, from, .. } => {
                let layer = doc
                    .layers
                    .get_mut(*index)
                    .ok_or(EditError::LayerIndex(*index))?;
                layer.offset = *from;
                Ok(())
            }
            EditOp::RenameLayer { index, from, .. } => {
                let layer = doc
                    .layers
                    .get_mut(*index)
                    .ok_or(EditError::LayerIndex(*index))?;
                layer.name = from.clone();
                Ok(())
            }
            EditOp::EraseBasePixels { from, .. } => {
                doc.restore(from.clone());
                Ok(())
            }
            EditOp::ScaleLayer { from, .. } => {
                doc.restore(from.clone());
                Ok(())
            }
        }
    }
}

#[derive(Debug, Clone, Default)]
struct History {
    undo: Vec<EditOp>,
    redo: Vec<EditOp>,
    limit: usize,
}

impl History {
    fn new(limit: usize) -> Self {
        Self {
            undo: Vec::new(),
            redo: Vec::new(),
            limit,
        }
    }

    fn push(&mut self, op: EditOp) {
        self.undo.push(op);
        if self.limit > 0 && self.undo.len() > self.limit {
            self.undo.remove(0);
        }
        self.redo.clear();
    }
}

fn apply_view(doc: &mut Document, view: Rect) {
    doc.view = view;
    doc.canvas = Size::new(view.width, view.height);
}

/// Make every base pixel under `selection` fully transparent. Selection
/// coordinates are canvas coordinates and map into `base` through `view`.
fn erase_base_pixels(doc: &mut Document, selection: &Selection) {
    let canvas_rect = Rect::new(0, 0, doc.canvas.width, doc.canvas.height);
    let Some(clipped) = selection.bounds().intersect(canvas_rect) else {
        return;
    };
    for y in 0..clipped.height {
        for x in 0..clipped.width {
            let cx = clipped.x + x as i32;
            let cy = clipped.y + y as i32;
            if !selection.contains(cx as f32 + 0.5, cy as f32 + 0.5) {
                continue;
            }
            let bx = doc.view.x + cx;
            let by = doc.view.y + cy;
            if bx < 0 || by < 0 {
                continue;
            }
            let (bx, by) = (bx as u32, by as u32);
            if bx < doc.base.width() && by < doc.base.height() {
                *doc.base.get_pixel_mut(bx, by) = image::Rgba([0, 0, 0, 0]);
            }
        }
    }
}

fn apply_resize(doc: &mut Document, to: Size) {
    // 以当前画布（所见）为参照缩放到 `to`：把当前可见内容重采样为新底图，
    // 缩放后不再保留裁剪窗口（view 回到全尺寸）。
    let sx = to.width as f64 / doc.canvas.width as f64;
    let sy = to.height as f64 / doc.canvas.height as f64;
    let visible = doc.flatten();
    doc.base = image::imageops::resize(
        &visible,
        to.width,
        to.height,
        image::imageops::FilterType::Lanczos3,
    );
    doc.view = Rect::new(0, 0, to.width, to.height);
    doc.canvas = to;
    for layer in &mut doc.layers {
        let ow = ((layer.image.width() as f64) * sx).round() as u32;
        let oh = ((layer.image.height() as f64) * sy).round() as u32;
        layer.image = image::imageops::resize(
            &layer.image,
            ow.max(1),
            oh.max(1),
            image::imageops::FilterType::Lanczos3,
        );
        layer.offset = (
            ((layer.offset.0 as f64) * sx).round() as i32,
            ((layer.offset.1 as f64) * sy).round() as i32,
        );
    }
}

impl Document {
    pub fn from_image(image: RgbaImage) -> Self {
        let canvas = Size::new(image.width(), image.height());
        Self {
            source: None,
            canvas,
            base: image,
            view: Rect::new(0, 0, canvas.width, canvas.height),
            layers: Vec::new(),
            next_layer_seq: 0,
            history: History::new(256),
        }
    }

    pub fn execute(&mut self, op: EditOp) -> Result<(), EditError> {
        op.apply(self)?;
        self.history.push(op);
        Ok(())
    }

    /// Crop to `rect`, given in canvas coordinates. The rect is clamped to the
    /// current canvas; a rect that falls outside it is rejected. Use
    /// [`Document::reset_crop`] to reveal the full base image again.
    pub fn crop(&mut self, rect: Rect) -> Result<(), EditError> {
        let canvas_rect = Rect::new(0, 0, self.canvas.width, self.canvas.height);
        let clamped = rect.intersect(canvas_rect).ok_or(EditError::EmptyCrop)?;
        if clamped.is_empty() {
            return Err(EditError::EmptyCrop);
        }
        self.execute(EditOp::CropDocument {
            from: self.view,
            to: Rect::new(
                self.view.x + clamped.x,
                self.view.y + clamped.y,
                clamped.width,
                clamped.height,
            ),
        })
    }

    /// Undo every crop, showing the full base image again.
    pub fn reset_crop(&mut self) -> Result<(), EditError> {
        if self.view.x == 0
            && self.view.y == 0
            && self.view.width == self.base.width()
            && self.view.height == self.base.height()
        {
            return Ok(());
        }
        self.execute(EditOp::CropDocument {
            from: self.view,
            to: Rect::new(0, 0, self.base.width(), self.base.height()),
        })
    }

    pub fn apply_transform(&mut self, transform: Transform) -> Result<(), EditError> {
        self.execute(EditOp::TransformDocument { transform })
    }

    /// Resize the document to exactly `to`: the current canvas (what the user
    /// sees) is resampled (Lanczos3) into a new base, any crop is dropped, and
    /// layers are scaled proportionally. Full previous state is snapshotted for
    /// undo.
    pub fn resize(&mut self, to: Size) -> Result<(), EditError> {
        if to.width < 1 || to.height < 1 {
            return Err(EditError::InvalidCanvasSize);
        }
        let from = self.snapshot();
        self.execute(EditOp::ResizeDocument { from, to })
    }

    pub fn add_layer(&mut self, layer: Layer) -> Result<usize, EditError> {
        let index = self.layers.len();
        self.execute(EditOp::AddLayer { layer, index })?;
        Ok(index)
    }

    pub fn remove_layer(&mut self, index: usize) -> Result<(), EditError> {
        let layer = self
            .layers
            .get(index)
            .cloned()
            .ok_or(EditError::LayerIndex(index))?;
        self.execute(EditOp::RemoveLayer { layer, index })
    }

    pub fn move_layer(&mut self, from: usize, to: usize) -> Result<(), EditError> {
        self.execute(EditOp::MoveLayer { from, to })
    }

    pub fn set_layer_visible(&mut self, index: usize, visible: bool) -> Result<(), EditError> {
        let from = self
            .layers
            .get(index)
            .ok_or(EditError::LayerIndex(index))?
            .visible;
        self.execute(EditOp::SetVisible {
            index,
            from,
            to: visible,
        })
    }

    pub fn set_layer_opacity(&mut self, index: usize, opacity: f32) -> Result<(), EditError> {
        let from = self
            .layers
            .get(index)
            .ok_or(EditError::LayerIndex(index))?
            .opacity;
        self.execute(EditOp::SetOpacity {
            index,
            from,
            to: opacity.clamp(0.0, 1.0),
        })
    }

    pub fn translate_layer(&mut self, index: usize, dx: i32, dy: i32) -> Result<(), EditError> {
        let layer = self
            .layers
            .get(index)
            .ok_or(EditError::LayerIndex(index))?;
        let from = layer.offset;
        let to = (from.0 + dx, from.1 + dy);
        self.execute(EditOp::TranslateLayer { index, from, to })
    }

    pub fn rename_layer(&mut self, index: usize, name: impl Into<String>) -> Result<(), EditError> {
        let from = self
            .layers
            .get(index)
            .ok_or(EditError::LayerIndex(index))?
            .name
            .clone();
        self.execute(EditOp::RenameLayer {
            index,
            from,
            to: name.into(),
        })
    }

    /// Scale a layer's image to exactly `to` (Lanczos3). The layer offset
    /// (top-left corner) is unchanged; only the content is resampled. Undo
    /// restores the full previous state via snapshot.
    pub fn scale_layer(&mut self, index: usize, to: Size) -> Result<(), EditError> {
        if to.width < 1 || to.height < 1 {
            return Err(EditError::InvalidCanvasSize);
        }
        if index >= self.layers.len() {
            return Err(EditError::LayerIndex(index));
        }
        let from = self.snapshot();
        self.execute(EditOp::ScaleLayer { index, from, to })
    }

    pub fn undo(&mut self) -> Result<(), EditError> {
        let op = self.history.undo.pop().ok_or(EditError::NothingToUndo)?;
        op.revert(self)?;
        self.history.redo.push(op);
        Ok(())
    }

    pub fn redo(&mut self) -> Result<(), EditError> {
        let op = self.history.redo.pop().ok_or(EditError::NothingToRedo)?;
        op.apply(self)?;
        self.history.undo.push(op);
        Ok(())
    }

    pub fn can_undo(&self) -> bool {
        !self.history.undo.is_empty()
    }

    pub fn can_redo(&self) -> bool {
        !self.history.redo.is_empty()
    }

    pub fn selection_layer(&self, selection: &Selection) -> Result<Layer, EditError> {
        let bounds = selection.bounds();
        if bounds.is_empty() {
            return Err(EditError::EmptySelection);
        }
        let mut image = RgbaImage::new(bounds.width, bounds.height);
        for y in 0..bounds.height {
            for x in 0..bounds.width {
                let sx = bounds.x + x as i32;
                let sy = bounds.y + y as i32;
                if !selection.contains(sx as f32 + 0.5, sy as f32 + 0.5) {
                    continue;
                }
                image.put_pixel(x, y, self.composite_pixel(sx, sy));
            }
        }
        let mut layer = Layer::new("Selection", image);
        layer.offset = (bounds.x, bounds.y);
        Ok(layer)
    }

    /// Next auto name for a created layer ("1", "2", ...), by creation order.
    pub fn next_layer_name(&mut self) -> String {
        self.next_layer_seq += 1;
        self.next_layer_seq.to_string()
    }

    pub fn extract_selection(&mut self, selection: &Selection) -> Result<usize, EditError> {
        let mut layer = self.selection_layer(selection)?;
        layer.name = self.next_layer_name();
        let index = self.layers.len();
        self.execute(EditOp::AddLayer { layer, index })?;
        Ok(index)
    }

    /// Non-destructive "delete selection pixels": base pixels under the
    /// selection become transparent; layers are untouched; undo restores the
    /// exact previous state via snapshot.
    pub fn erase_selection(&mut self, selection: &Selection) -> Result<(), EditError> {
        let canvas_rect = Rect::new(0, 0, self.canvas.width, self.canvas.height);
        let clipped = selection
            .bounds()
            .intersect(canvas_rect)
            .ok_or(EditError::EmptySelection)?;
        if clipped.is_empty() {
            return Err(EditError::EmptySelection);
        }
        let from = self.snapshot();
        self.execute(EditOp::EraseBasePixels {
            from,
            selection: selection.clone(),
        })
    }

    pub fn composite_pixel(&self, x: i32, y: i32) -> image::Rgba<u8> {
        if x < 0 || y < 0 || x >= self.canvas.width as i32 || y >= self.canvas.height as i32 {
            return image::Rgba([0, 0, 0, 0]);
        }
        let bx = (self.view.x + x) as u32;
        let by = (self.view.y + y) as u32;
        let mut out = if bx < self.base.width() && by < self.base.height() {
            self.base.get_pixel(bx, by).0
        } else {
            [0, 0, 0, 0]
        };
        for layer in &self.layers {
            if !layer.visible || layer.opacity <= 0.0 {
                continue;
            }
            let lx = x - layer.offset.0;
            let ly = y - layer.offset.1;
            if lx < 0
                || ly < 0
                || lx >= layer.image.width() as i32
                || ly >= layer.image.height() as i32
            {
                continue;
            }
            let src = layer.image.get_pixel(lx as u32, ly as u32).0;
            let a = (src[3] as f32 / 255.0) * layer.opacity;
            if a <= 0.0 {
                continue;
            }
            let inv = 1.0 - a;
            for c in 0..3 {
                out[c] = (src[c] as f32 * a + out[c] as f32 * inv).round() as u8;
            }
            out[3] = ((a + out[3] as f32 / 255.0 * inv) * 255.0).round() as u8;
        }
        image::Rgba(out)
    }

    pub fn flatten(&self) -> RgbaImage {
        let mut out = RgbaImage::new(self.canvas.width, self.canvas.height);
        for y in 0..self.canvas.height {
            for x in 0..self.canvas.width {
                out.put_pixel(x, y, self.composite_pixel(x as i32, y as i32));
            }
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn checker(width: u32, height: u32) -> RgbaImage {
        RgbaImage::from_fn(width, height, |x, y| {
            if (x / 8 + y / 8) % 2 == 0 {
                image::Rgba([255, 0, 0, 255])
            } else {
                image::Rgba([0, 0, 255, 255])
            }
        })
    }

    #[test]
    fn extract_rect_selection_produces_layer_of_selection_bounds() {
        let mut doc = Document::from_image(checker(64, 32));
        let index = doc
            .extract_selection(&Selection::Rect(Rect::new(8, 4, 16, 12)))
            .unwrap();
        let layer = &doc.layers[index];
        assert_eq!(layer.image.width(), 16);
        assert_eq!(layer.image.height(), 12);
        assert_eq!(layer.offset, (8, 4));
        assert_eq!(layer.image.get_pixel(0, 0).0, [0, 0, 255, 255]);
        assert_eq!(layer.image.get_pixel(0, 4).0, [255, 0, 0, 255]);
    }

    #[test]
    fn ellipse_selection_masks_corners() {
        let doc = Document::from_image(checker(64, 64));
        let layer = doc
            .selection_layer(&Selection::Ellipse(Rect::new(0, 0, 64, 64)))
            .unwrap();
        assert_eq!(layer.image.get_pixel(0, 0).0[3], 0);
        assert_eq!(layer.image.get_pixel(32, 32).0[3], 255);
    }

    #[test]
    fn lasso_selection_contains_polygon_interior() {
        let sel = Selection::Lasso(vec![
            (0.0, 0.0),
            (10.0, 0.0),
            (10.0, 10.0),
            (0.0, 10.0),
        ]);
        assert!(sel.contains(5.0, 5.0));
        assert!(!sel.contains(15.0, 5.0));
        assert_eq!(sel.bounds(), Rect::new(0, 0, 10, 10));
    }

    #[test]
    fn history_roundtrips_layer_ops() {
        let mut doc = Document::from_image(checker(16, 16));
        doc.add_layer(Layer::new("clip", checker(4, 4))).unwrap();
        assert_eq!(doc.layers.len(), 1);
        doc.undo().unwrap();
        assert_eq!(doc.layers.len(), 0);
        doc.redo().unwrap();
        assert_eq!(doc.layers.len(), 1);
    }

    #[test]
    fn undo_restores_scalar_layer_properties() {
        let mut doc = Document::from_image(checker(16, 16));
        let index = doc.add_layer(Layer::new("clip", checker(4, 4))).unwrap();
        doc.set_layer_opacity(index, 0.4).unwrap();
        doc.set_layer_visible(index, false).unwrap();
        doc.translate_layer(index, 3, -2).unwrap();
        doc.rename_layer(index, "renamed").unwrap();

        doc.undo().unwrap();
        assert_eq!(doc.layers[index].name, "clip");
        doc.undo().unwrap();
        assert_eq!(doc.layers[index].offset, (0, 0));
        doc.undo().unwrap();
        assert!(doc.layers[index].visible);
        doc.undo().unwrap();
        assert_eq!(doc.layers[index].opacity, 1.0);
    }

    #[test]
    fn cross_document_paste_keeps_pixels_and_offset() {
        let mut src = Document::from_image(checker(32, 32));
        let index = src
            .extract_selection(&Selection::Rect(Rect::new(4, 6, 8, 8)))
            .unwrap();
        let layer = src.layers[index].clone();

        let mut dst = Document::from_image(checker(16, 16));
        let pasted = dst.add_layer(layer).unwrap();
        assert_eq!(dst.layers[pasted].offset, (4, 6));
        assert_eq!(dst.layers[pasted].image.width(), 8);
        assert_eq!(dst.composite_pixel(4, 6).0, [255, 0, 0, 255]);
    }

    #[test]
    fn erase_selection_makes_base_pixels_transparent_and_keeps_layers() {
        let mut doc = Document::from_image(checker(32, 16));
        let index = doc
            .extract_selection(&Selection::Rect(Rect::new(20, 2, 8, 8)))
            .unwrap();

        doc.erase_selection(&Selection::Rect(Rect::new(4, 4, 8, 8)))
            .unwrap();
        // Erased base area is transparent (no layer above it there).
        assert_eq!(doc.composite_pixel(4, 4).0, [0, 0, 0, 0]);
        assert_eq!(doc.composite_pixel(11, 11).0, [0, 0, 0, 0]);
        // Outside the selection the base is untouched.
        assert_eq!(doc.composite_pixel(0, 0).0[3], 255);
        // The layer extracted before erasing is untouched.
        assert_eq!(doc.layers[index].image.get_pixel(0, 0).0[3], 255);
        assert_eq!(doc.composite_pixel(20, 2).0[3], 255);
    }

    #[test]
    fn erase_selection_clips_to_canvas_and_roundtrips_undo_redo() {
        let mut doc = Document::from_image(checker(16, 16));
        let original = doc.flatten();

        // Selection partially outside the canvas: only the inside part erases.
        doc.erase_selection(&Selection::Rect(Rect::new(12, 12, 20, 20)))
            .unwrap();
        assert_eq!(doc.composite_pixel(12, 12).0, [0, 0, 0, 0]);
        assert_eq!(doc.composite_pixel(15, 15).0, [0, 0, 0, 0]);
        assert_eq!(doc.composite_pixel(11, 11).0[3], 255);

        doc.undo().unwrap();
        assert_eq!(doc.flatten(), original);
        doc.redo().unwrap();
        assert_eq!(doc.composite_pixel(12, 12).0, [0, 0, 0, 0]);
    }

    #[test]
    fn erase_selection_rejects_empty_and_outside_selections() {
        let mut doc = Document::from_image(checker(16, 16));
        assert_eq!(
            doc.erase_selection(&Selection::Rect(Rect::new(0, 0, 0, 0))),
            Err(EditError::EmptySelection)
        );
        assert_eq!(
            doc.erase_selection(&Selection::Rect(Rect::new(40, 40, 4, 4))),
            Err(EditError::EmptySelection)
        );
        assert_eq!(doc.can_undo(), false);
    }

    #[test]
    fn erase_selection_composes_with_crop_view_mapping() {
        // Erase must map canvas coords through `view` after a crop.
        let mut doc = Document::from_image(ramp(32, 32));
        doc.crop(Rect::new(8, 8, 16, 16)).unwrap();
        doc.erase_selection(&Selection::Rect(Rect::new(0, 0, 4, 4)))
            .unwrap();
        assert_eq!(doc.composite_pixel(0, 0).0, [0, 0, 0, 0]);
        // base pixel at (8,8) is what canvas (0,0) maps to.
        assert_eq!(doc.base.get_pixel(8, 8).0, [0, 0, 0, 0]);
        assert_eq!(doc.composite_pixel(4, 4).0, [12, 12, 0, 255]);
    }

    #[test]
    fn scale_layer_resamples_content_and_keeps_offset() {
        let mut doc = Document::from_image(checker(32, 32));
        let index = doc
            .add_layer(Layer::new("clip", checker(8, 4)))
            .unwrap();
        doc.translate_layer(index, 5, 3).unwrap();

        doc.scale_layer(index, Size::new(16, 8)).unwrap();
        let layer = &doc.layers[index];
        assert_eq!(layer.image.width(), 16);
        assert_eq!(layer.image.height(), 8);
        // Offset (top-left anchor) is untouched by scaling.
        assert_eq!(layer.offset, (5, 3));

        doc.undo().unwrap();
        assert_eq!(doc.layers[index].image.width(), 8);
        assert_eq!(doc.layers[index].image.height(), 4);
        assert_eq!(doc.layers[index].offset, (5, 3));
        doc.redo().unwrap();
        assert_eq!(doc.layers[index].image.width(), 16);
    }

    #[test]
    fn scale_layer_rejects_bad_size_and_index() {
        let mut doc = Document::from_image(checker(16, 16));
        let index = doc.add_layer(Layer::new("clip", checker(4, 4))).unwrap();
        let undo_depth = doc.history.undo.len();
        assert_eq!(
            doc.scale_layer(index, Size::new(0, 4)),
            Err(EditError::InvalidCanvasSize)
        );
        assert_eq!(
            doc.scale_layer(9, Size::new(4, 4)),
            Err(EditError::LayerIndex(9))
        );
        // Rejected calls must not touch the history stack.
        assert_eq!(doc.history.undo.len(), undo_depth);
        assert_eq!(doc.layers[index].image.width(), 4);
    }

    #[test]
    fn move_layer_is_single_reorder_op() {
        // Drag-reorder in the panel relies on move_layer(from, to) being one
        // undoable step that lands the layer exactly at `to`.
        let mut doc = Document::from_image(checker(16, 16));
        doc.add_layer(Layer::new("a", checker(2, 2))).unwrap();
        doc.add_layer(Layer::new("b", checker(2, 2))).unwrap();
        doc.add_layer(Layer::new("c", checker(2, 2))).unwrap();

        doc.move_layer(2, 0).unwrap();
        let names: Vec<&str> = doc.layers.iter().map(|l| l.name.as_str()).collect();
        assert_eq!(names, ["c", "a", "b"]);

        doc.undo().unwrap();
        let names: Vec<&str> = doc.layers.iter().map(|l| l.name.as_str()).collect();
        assert_eq!(names, ["a", "b", "c"]);
    }

    #[test]
    fn rotate_four_times_is_identity() {
        let mut doc = Document::from_image(corner_marked(20, 10));
        let original = doc.flatten();
        for _ in 0..4 {
            doc.apply_transform(Transform::Rotate90Cw).unwrap();
        }
        assert_eq!(doc.canvas, Size::new(20, 10));
        assert_eq!(doc.flatten(), original);
    }

    #[test]
    fn rotate90_swaps_canvas_and_moves_marked_corner() {
        let mut doc = Document::from_image(corner_marked(20, 10));
        assert_eq!(doc.composite_pixel(0, 0).0, [255, 255, 255, 255]);

        doc.apply_transform(Transform::Rotate90Cw).unwrap();
        assert_eq!(doc.canvas, Size::new(10, 20));
        assert_eq!(doc.composite_pixel(9, 0).0, [255, 255, 255, 255]);
        assert_eq!(doc.composite_pixel(0, 0).0, [0, 0, 0, 255]);

        doc.apply_transform(Transform::Rotate90Ccw).unwrap();
        assert_eq!(doc.canvas, Size::new(20, 10));
        assert_eq!(doc.composite_pixel(0, 0).0, [255, 255, 255, 255]);
    }

    #[test]
    fn flips_are_self_inverse_and_move_marked_corner() {
        for transform in [Transform::FlipHorizontal, Transform::FlipVertical] {
            let mut doc = Document::from_image(corner_marked(16, 8));
            doc.apply_transform(transform).unwrap();
            let expected = match transform {
                Transform::FlipHorizontal => (15, 0),
                _ => (0, 7),
            };
            assert_eq!(doc.composite_pixel(expected.0, expected.1).0, [255, 255, 255, 255]);
            doc.apply_transform(transform).unwrap();
            assert_eq!(doc.composite_pixel(0, 0).0, [255, 255, 255, 255]);
        }
    }

    #[test]
    fn transform_moves_layers_and_undo_restores_everything() {
        let mut doc = Document::from_image(RgbaImage::from_pixel(
            20,
            10,
            image::Rgba([0, 0, 0, 255]),
        ));
        let mut layer = Layer::new(
            "dot",
            RgbaImage::from_pixel(2, 2, image::Rgba([255, 255, 255, 255])),
        );
        layer.offset = (1, 1);
        doc.add_layer(layer).unwrap();

        doc.apply_transform(Transform::Rotate90Cw).unwrap();
        assert_eq!(doc.canvas, Size::new(10, 20));
        assert_eq!(doc.layers[0].offset, (10 - (1 + 2), 1));
        assert_eq!(doc.composite_pixel(7, 1).0, [255, 255, 255, 255]);

        doc.undo().unwrap();
        assert_eq!(doc.canvas, Size::new(20, 10));
        assert_eq!(doc.layers[0].offset, (1, 1));
        assert_eq!(doc.composite_pixel(1, 1).0, [255, 255, 255, 255]);

        doc.redo().unwrap();
        assert_eq!(doc.canvas, Size::new(10, 20));
        assert_eq!(doc.layers[0].offset, (7, 1));
    }

    #[test]
    fn rotate_ccw_moves_layer_to_expected_place() {
        let mut doc = Document::from_image(RgbaImage::from_pixel(
            20,
            10,
            image::Rgba([0, 0, 0, 255]),
        ));
        let mut layer = Layer::new(
            "dot",
            RgbaImage::from_pixel(2, 2, image::Rgba([255, 255, 255, 255])),
        );
        layer.offset = (1, 1);
        doc.add_layer(layer).unwrap();

        doc.apply_transform(Transform::Rotate90Ccw).unwrap();
        assert_eq!(doc.canvas, Size::new(10, 20));
        assert_eq!(doc.layers[0].offset, (1, 20 - 1 - 2));
        assert_eq!(doc.composite_pixel(1, 17).0, [255, 255, 255, 255]);
    }

    fn corner_marked(width: u32, height: u32) -> RgbaImage {
        let mut image = RgbaImage::from_pixel(width, height, image::Rgba([0, 0, 0, 255]));
        image.put_pixel(0, 0, image::Rgba([255, 255, 255, 255]));
        image
    }

    fn ramp(width: u32, height: u32) -> RgbaImage {
        RgbaImage::from_fn(width, height, |x, y| {
            image::Rgba([x as u8, y as u8, 0, 255])
        })
    }

    #[test]
    fn crop_shrinks_canvas_and_shifts_visible_region() {
        let mut doc = Document::from_image(ramp(64, 48));
        doc.crop(Rect::new(10, 6, 20, 12)).unwrap();

        assert_eq!(doc.canvas, Size::new(20, 12));
        assert_eq!(doc.view, Rect::new(10, 6, 20, 12));
        assert_eq!(doc.base.width(), 64);
        assert_eq!(doc.composite_pixel(0, 0).0, [10, 6, 0, 255]);
        assert_eq!(doc.composite_pixel(19, 11).0, [29, 17, 0, 255]);
    }

    #[test]
    fn crop_undo_and_redo_restore_canvas_and_pixels() {
        let mut doc = Document::from_image(ramp(64, 48));
        let original = doc.flatten();

        doc.crop(Rect::new(10, 6, 20, 12)).unwrap();
        doc.undo().unwrap();
        assert_eq!(doc.canvas, Size::new(64, 48));
        assert_eq!(doc.flatten(), original);

        doc.redo().unwrap();
        assert_eq!(doc.canvas, Size::new(20, 12));
        assert_eq!(doc.composite_pixel(0, 0).0, [10, 6, 0, 255]);
    }

    #[test]
    fn crop_composes_and_clamps_to_the_image() {
        let mut doc = Document::from_image(ramp(64, 48));
        doc.crop(Rect::new(10, 6, 20, 12)).unwrap();
        doc.crop(Rect::new(2, 2, 100, 100)).unwrap();

        assert_eq!(doc.canvas, Size::new(18, 10));
        assert_eq!(doc.view, Rect::new(12, 8, 18, 10));
        assert_eq!(doc.composite_pixel(0, 0).0, [12, 8, 0, 255]);

        let mut doc = Document::from_image(ramp(16, 16));
        assert_eq!(doc.crop(Rect::new(20, 20, 4, 4)), Err(EditError::EmptyCrop));
        assert_eq!(doc.crop(Rect::new(0, 0, 0, 4)), Err(EditError::EmptyCrop));
    }

    #[test]
    fn reset_crop_shows_the_full_image_again() {
        let mut doc = Document::from_image(ramp(64, 48));
        doc.crop(Rect::new(10, 6, 20, 12)).unwrap();
        doc.reset_crop().unwrap();

        assert_eq!(doc.canvas, Size::new(64, 48));
        assert_eq!(doc.composite_pixel(0, 0).0, [0, 0, 0, 255]);
        assert_eq!(doc.composite_pixel(63, 47).0, [63, 47, 0, 255]);
        doc.undo().unwrap();
        assert_eq!(doc.canvas, Size::new(20, 12));
    }

    #[test]
    fn crop_moves_layers_relative_to_the_new_origin() {
        let mut doc = Document::from_image(RgbaImage::from_pixel(
            40,
            30,
            image::Rgba([0, 0, 0, 255]),
        ));
        let mut layer = Layer::new(
            "dot",
            RgbaImage::from_pixel(4, 4, image::Rgba([255, 255, 255, 255])),
        );
        layer.offset = (2, 3);
        doc.add_layer(layer).unwrap();

        doc.crop(Rect::new(5, 5, 20, 20)).unwrap();
        assert_eq!(doc.canvas, Size::new(20, 20));
        // 图层坐标未改动，但视口左上角移到了 (5,5)，所以它落在画布外左侧/上方。
        assert_eq!(doc.layers[0].offset, (2, 3));
        assert_eq!(doc.composite_pixel(0, 0).0, [0, 0, 0, 255]);

        doc.reset_crop().unwrap();
        assert_eq!(doc.composite_pixel(2, 3).0, [255, 255, 255, 255]);
    }

    #[test]
    fn crop_then_rotate_keeps_the_view_on_the_same_content() {
        let mut doc = Document::from_image(ramp(64, 48));
        doc.crop(Rect::new(10, 6, 20, 12)).unwrap();
        doc.apply_transform(Transform::Rotate90Cw).unwrap();

        assert_eq!(doc.canvas, Size::new(12, 20));
        assert_eq!(doc.view, Rect::new(48 - 6 - 12, 10, 12, 20));
        // 裁剪区左上角的内容在旋转后落到右上角。
        assert_eq!(doc.composite_pixel(11, 0).0, [10, 6, 0, 255]);
        assert_eq!(doc.composite_pixel(0, 19).0, [29, 17, 0, 255]);

        doc.apply_transform(Transform::Rotate90Ccw).unwrap();
        assert_eq!(doc.canvas, Size::new(20, 12));
        assert_eq!(doc.composite_pixel(0, 0).0, [10, 6, 0, 255]);
    }

    fn solid(width: u32, height: u32, color: [u8; 4]) -> RgbaImage {
        RgbaImage::from_pixel(width, height, image::Rgba(color))
    }

    #[test]
    fn resize_sets_canvas_and_drops_crop() {
        let mut doc = Document::from_image(solid(64, 48, [10, 20, 30, 255]));
        doc.crop(Rect::new(8, 6, 32, 24)).unwrap();
        assert_eq!(doc.canvas, Size::new(32, 24));

        doc.resize(Size::new(16, 12)).unwrap();
        assert_eq!(doc.canvas, Size::new(16, 12));
        assert_eq!(doc.view, Rect::new(0, 0, 16, 12));
        assert_eq!(doc.base.dimensions(), (16, 12));
        assert_eq!(doc.composite_pixel(0, 0).0, [10, 20, 30, 255]);
    }

    #[test]
    fn resize_upscale_grows_dimensions() {
        let mut doc = Document::from_image(solid(10, 5, [1, 2, 3, 255]));
        doc.resize(Size::new(30, 15)).unwrap();
        assert_eq!(doc.canvas, Size::new(30, 15));
        assert_eq!(doc.base.dimensions(), (30, 15));
        assert_eq!(doc.composite_pixel(29, 14).0, [1, 2, 3, 255]);
    }

    #[test]
    fn resize_undo_restores_base_view_and_layers() {
        let mut doc = Document::from_image(ramp(64, 48));
        let mut layer = Layer::new(
            "dot",
            RgbaImage::from_pixel(4, 4, image::Rgba([255, 255, 255, 255])),
        );
        layer.offset = (6, 6);
        doc.add_layer(layer).unwrap();
        let original = doc.flatten();
        let original_layers = doc.layers.clone();

        doc.resize(Size::new(32, 24)).unwrap();
        assert_eq!(doc.canvas, Size::new(32, 24));

        doc.undo().unwrap();
        assert_eq!(doc.canvas, Size::new(64, 48));
        assert_eq!(doc.base.dimensions(), (64, 48));
        assert_eq!(doc.layers, original_layers);
        assert_eq!(doc.flatten(), original);

        doc.redo().unwrap();
        assert_eq!(doc.canvas, Size::new(32, 24));
    }

    #[test]
    fn resize_rejects_zero_dimensions() {
        let mut doc = Document::from_image(solid(16, 16, [0, 0, 0, 255]));
        assert_eq!(
            doc.resize(Size::new(0, 16)),
            Err(EditError::InvalidCanvasSize)
        );
        assert_eq!(
            doc.resize(Size::new(16, 0)),
            Err(EditError::InvalidCanvasSize)
        );
        assert_eq!(doc.canvas, Size::new(16, 16));
    }

    #[test]
    fn resize_scales_layer_offset_and_image() {
        let mut doc = Document::from_image(solid(40, 20, [0, 0, 0, 255]));
        let mut layer = Layer::new(
            "dot",
            RgbaImage::from_pixel(4, 4, image::Rgba([255, 255, 255, 255])),
        );
        layer.offset = (10, 5);
        doc.add_layer(layer).unwrap();

        doc.resize(Size::new(20, 10)).unwrap();
        let l = &doc.layers[0];
        assert_eq!(l.offset, (5, 3));
        assert_eq!(l.image.dimensions(), (2, 2));
        assert_eq!(doc.composite_pixel(5, 3).0, [255, 255, 255, 255]);
    }

    #[test]
    fn flatten_composites_translated_layer() {
        let mut doc = Document::from_image(RgbaImage::from_pixel(
            16,
            16,
            image::Rgba([0, 0, 0, 255]),
        ));
        let mut layer = Layer::new(
            "dot",
            RgbaImage::from_pixel(2, 2, image::Rgba([255, 255, 255, 255])),
        );
        layer.offset = (5, 5);
        doc.add_layer(layer).unwrap();
        let flat = doc.flatten();
        assert_eq!(flat.get_pixel(5, 5).0, [255, 255, 255, 255]);
        assert_eq!(flat.get_pixel(0, 0).0, [0, 0, 0, 255]);
    }
}
