// crates/core/src/document/mod.rs
use super::*;
use crate::layer_ops::blend_image_at_mode;

#[derive(Debug, Clone)]
pub struct Document {
    pub source: Option<std::path::PathBuf>,
    pub canvas: Size,
    /// 裁剪窗口：`view` 是背景图里当前可见的那一块（`layers[0]` 保存完整
    /// 像素，offset 恒为 `(-view.x, -view.y)`）。只用于「重置裁剪」与
    /// 坐标换算，不参与合成。
    pub view: Rect,
    /// 图层栈：索引 0 = 最底（背景），`len-1` = 最顶。
    /// 背景层 `background == true`、默认锁定；矢量标记作为 `LayerContent::Mark`
    /// 与位图图层同栈、同排序、同可见性 / 锁定 / 不透明度。
    pub layers: Vec<Layer>,
    /// 图层分组元数据（平行字段，不进 `layers`）。
    /// 组只是组织标签：成员由 `Layer::group` 反查，z 序与合成完全不变，
    /// 因此普通图片（只有 1 层背景）和 PSD 分层态都能随时编组。
    pub groups: Vec<LayerGroup>,
    /// Active selection (PS-style alpha coverage map, canvas coordinates).
    /// Lives here instead of on the UI session so Ctrl+Z walks it back in the
    /// same history as pixel edits. `None` = nothing selected.
    pub selection: Option<SelectionState>,
    /// Monotonic counter bumped by every committed / structural change
    /// (execute, undo, redo). Preview caches key on it: any real edit
    /// invalidates them, while history-free live mutations (layer offset
    /// during a drag, live mark moves) deliberately do NOT bump it so each
    /// drag tick can reuse cached scaled layers.
    pub edit_epoch: u64,
    /// Auto-naming counter: new layers (extract/paste) are named by their
    /// creation order ("1", "2", ...). Not part of snapshots on purpose, so
    /// names keep increasing across undo/redo.
    pub(crate) next_layer_seq: u32,
    /// PSD 内嵌 ICC 的「文档色彩空间 -> sRGB」变换。
    ///
    /// 只有 `display` 路径才会设置（解码时转的图层已经是 sRGB，写回也不需要
    /// 再转）；`flatten()` 在整幅合成完成后一次性用它转换，与 Photoshop
    /// 「先在文档空间合成、最后转显示」的顺序一致。不进快照 / 历史：
    /// 它描述的是文件本身，而不是某一步编辑。
    pub icc: Option<SharedIcc>,
    pub(crate) history: History,
}

/// Full document state taken before a pixel-modifying op, so undo can restore
/// it exactly.
#[derive(Debug, Clone, PartialEq)]
pub struct DocumentSnapshot {
    pub canvas: Size,
    pub view: Rect,
    pub layers: Vec<Layer>,
    /// 快照类 op（缩放 / 裁剪 / 清背景）会整份还原 layers，分组元数据必须
    /// 一起带上，否则 Ctrl+Z 会把分组悄悄抹掉。
    pub groups: Vec<LayerGroup>,
}

impl Document {
    pub fn snapshot(&self) -> DocumentSnapshot {
        DocumentSnapshot {
            canvas: self.canvas,
            view: self.view,
            layers: self.layers.clone(),
            groups: self.groups.clone(),
        }
    }

    /// Replace everything except history (used when undoing a pixel op).
    pub fn restore(&mut self, snapshot: DocumentSnapshot) {
        self.canvas = snapshot.canvas;
        self.view = snapshot.view;
        self.layers = snapshot.layers;
        self.groups = snapshot.groups;
    }

    // ---- 背景层访问（`layers[0]`，恒存在）----------------------------

    pub fn background(&self) -> &Layer {
        &self.layers[0]
    }

    pub fn background_mut(&mut self) -> &mut Layer {
        &mut self.layers[0]
    }

    pub fn base_image(&self) -> &RgbaImage {
        self.layers[0].image().expect("background layer is raster")
    }

    pub fn base_image_mut(&mut self) -> &mut RgbaImage {
        self.layers[0]
            .image_mut()
            .expect("background layer is raster")
    }

    /// 覆盖背景像素并保持它对齐 `view`（不做额外位移）。
    pub fn set_base_image(&mut self, image: RgbaImage) {
        if let Some(r) = self.layers[0].raster_mut() {
            r.image = image;
            r.transform = None;
            r.offset = (-self.view.x, -self.view.y);
        }
    }

    pub fn base_visible(&self) -> bool {
        self.layers[0].visible
    }

    pub fn base_hidden(&self) -> bool {
        !self.layers[0].visible
    }

    /// 背景像素是否已全部透明（面板显示"已删除"用）。
    ///
    /// 这是**内容**判断而不是结构状态：没有单独标志位，避免出现「标志说
    /// 删了、像素还在」的不一致。只用于面板展示，按行抽样（每 16 行一次）
    /// 以避开整幅扫描。
    pub fn base_deleted(&self) -> bool {
        let Some(img) = self.layers[0].image() else {
            return true;
        };
        let step = (img.height() / 64).max(1);
        let mut y = 0;
        while y < img.height() {
            for x in (0..img.width()).step_by(16) {
                if img.get_pixel(x, y).0[3] != 0 {
                    return false;
                }
            }
            y += step;
        }
        true
    }

    pub fn base_converted(&self) -> bool {
        !self.layers[0].locked
    }
}




/// One PSD layer ready to be assembled into a `Document`. Kept in `core` so
/// the decode crate can hand layers over without a circular dependency.
#[derive(Debug, Clone)]
pub struct PsdLayerSeed {
    pub name: String,
    pub offset: (i32, i32),
    pub image: RgbaImage,
    /// 折叠祖先组显隐**之后**的合成真值（渲染只认成员 visible）。
    pub visible: bool,
    /// 成员自己的 PSD 眼睛旗标（折叠**之前**的值）。装配时隐藏组用它
    /// 回填 `hidden_snapshot`，面板眼睛才能在加载后立刻显示真实状态
    /// ——否则「一开始就是隐藏的组」成员眼睛全显示闭眼，要点一次组
    /// 眼睛开→关（走 `set_group_visible` 的快照路径）才恢复。
    pub own_visible: bool,
    pub opacity: f32,
    /// Blend mode carried by the PSD layer record.
    pub blend: BlendMode,
    /// PSD 里这层所属的组（`PsdGroupSeed::psd_id`）。PSD 用 u32 给组编号，
    /// 我们的 `GroupId` 是进程内自增，装配时再映射一次。
    pub group: Option<u32>,
    /// 图层蒙版（`-2` 通道），已裁剪到画布范围。
    pub mask: Option<LayerMask>,
    /// 该层是剪贴蒙版（只在下层基底的不透明区域内可见）。
    pub clipping: bool,
    /// 烘焙样式**之前**的内容 alpha；作为剪贴基底时限幅用它（见
    /// [`Layer::clip_shape`]）。`None` = 该层没有烘焙样式。
    pub clip_shape: Option<LayerMask>,
    /// 烘焙样式画在内容之外的笔迹与它自己的混合模式（见 [`StylePixels`]）。
    pub style: Option<StylePixels>,
    /// 混合颜色带（PSD blending ranges + `lclr`），全默认 = `None`。
    pub blend_if: Option<BlendIf>,
    /// 挖空（PSD `knko`），默认关闭。
    pub knockout: Knockout,
    /// 透明形状图层（PSD `tsly`），默认开。
    pub transparency_shapes: bool,
    /// 组样式烘焙出的影子层（见 [`Layer::synthetic`]）：锁定、不写回 PSD。
    pub synthetic: bool,
}

/// One PSD folder ("layer group"). `layers` 里的成员通过 `PsdLayerSeed::group`
/// 反查，组与组的嵌套通过 `parent` 表达。
#[derive(Debug, Clone)]
pub struct PsdGroupSeed {
    pub psd_id: u32,
    pub parent: Option<u32>,
    pub name: String,
    pub collapsed: bool,
    /// Photoshop 的「组隐藏」旗标：组记录自己的 visible。
    pub visible: bool,
    /// 组自己的混合模式、不透明度与穿透标记（合成时整组应用）。
    pub blend: BlendMode,
    pub opacity: f32,
    pub pass_through: bool,
    /// 组样式（folder 记录 `lfxs`）的原始字节，保存时透传写回；
    /// `None` = 该组没有组样式。
    pub raw_style: Option<Vec<u8>>,
}

#[cfg(test)]
mod psd_assembly_tests {
    use super::*;
    use image::{GrayImage, Luma, Rgba};

    fn seed(
        name: &str,
        offset: (i32, i32),
        color: [u8; 4],
        size: (u32, u32),
        visible: bool,
        opacity: f32,
    ) -> PsdLayerSeed {
        PsdLayerSeed {
            name: name.into(),
            offset,
            image: RgbaImage::from_pixel(size.0, size.1, Rgba(color)),
            visible,
            own_visible: visible,
            opacity,
            blend: BlendMode::Normal,
            group: None,
            mask: None,
            clipping: false,
            clip_shape: None,
            style: None,
            blend_if: None,
            knockout: Knockout::None,
            transparency_shapes: true,
            synthetic: false,
        }
    }

    fn group_seed(
        psd_id: u32,
        name: &str,
        blend: BlendMode,
        opacity: f32,
        pass_through: bool,
    ) -> PsdGroupSeed {
        PsdGroupSeed {
            psd_id,
            parent: None,
            name: name.into(),
            collapsed: false,
            visible: true,
            blend,
            opacity,
            pass_through,
            raw_style: None,
        }
    }

    /// 隔离组：组先合成一块，再按组模式与下层混合；穿透组：成员各自与下层混合。
    ///
    /// 用「组里只有一个滤色红层、组本身正片叠底」来区分：穿透时红色直接跟
    /// 白底滤色（仍是白），隔离时它先在组里合成成纯红，再乘白底（仍是红）。
    #[test]
    fn isolated_group_multiplies_as_a_whole() {
        let white = seed("底", (0, 0), [255, 255, 255, 255], (2, 2), true, 1.0);
        let make = |pass_through: bool| {
            let mut red = seed("红", (0, 0), [255, 0, 0, 255], (2, 2), true, 1.0);
            red.group = Some(9);
            red.blend = BlendMode::Screen;
            Document::from_psd_layers_with_groups(
                (2, 2),
                vec![white.clone(), red],
                vec![group_seed(9, "组", BlendMode::Multiply, 1.0, pass_through)],
            )
        };
        assert_eq!(make(false).flatten().get_pixel(0, 0).0, [255, 0, 0, 255]);
        assert_eq!(make(true).flatten().get_pixel(0, 0).0, [255, 255, 255, 255]);
    }

    /// PSD 里一开始就是隐藏的组：成员 visible 被 decode 折叠成 false，但
    /// 面板眼睛要按自身旗标（`own_visible`）保持睁眼 —— 装配时给隐藏组
    /// 回填 `hidden_snapshot`；点组眼睛显示时按快照还原各自意愿。
    #[test]
    fn loaded_hidden_group_keeps_member_eye_and_restores_on_show() {
        let mut member = seed("成员", (0, 0), [255, 0, 0, 255], (2, 2), false, 1.0);
        member.own_visible = true; // 自己的眼睛是开的，因组隐藏被折叠
        member.group = Some(5);
        let mut off = seed("关", (0, 0), [0, 255, 0, 255], (2, 2), false, 1.0);
        off.own_visible = false; // 自己本来就隐藏
        off.group = Some(5);
        let mut g = group_seed(5, "组", BlendMode::Normal, 1.0, false);
        g.visible = false;
        let mut doc = Document::from_psd_layers_with_groups((2, 2), vec![member, off], vec![g]);
        let gid = doc.groups[0].id;
        assert!(!doc.layers[1].visible, "合成真值仍是隐藏");
        assert!(doc.layer_display_visible(1), "折叠隐藏的成员眼睛应保持睁眼");
        assert!(!doc.layer_display_visible(2), "自身就隐藏的成员仍是闭眼");
        assert_eq!(doc.groups[0].hidden_snapshot.len(), 2);
        doc.set_group_visible(gid, true).expect("显示组");
        assert!(doc.layers[1].visible, "显示组后按自身意愿还原");
        assert!(!doc.layers[2].visible, "自身隐藏的成员不跟着打开");
    }

    /// 组不透明度只应用一次：两个 50% 黑层在 50% 的组里应当是 0.375 的黑。
    #[test]
    fn group_opacity_applies_once_to_the_group() {
        let white = seed("底", (0, 0), [255, 255, 255, 255], (2, 2), true, 1.0);
        let mut a = seed("黑1", (0, 0), [0, 0, 0, 128], (2, 2), true, 1.0);
        a.group = Some(3);
        let mut b = seed("黑2", (0, 0), [0, 0, 0, 128], (2, 2), true, 1.0);
        b.group = Some(3);
        let doc = Document::from_psd_layers_with_groups(
            (2, 2),
            vec![white, a, b],
            vec![group_seed(3, "组", BlendMode::Normal, 0.5, true)],
        );
        assert_eq!(doc.flatten().get_pixel(0, 0).0, [159, 159, 159, 255]);
    }

    /// 半透明挖空层：挖掉下层后，浅挖空会露出组下方的背景（不透明度仍是满的）。
    #[test]
    fn shallow_knockout_reveals_group_backdrop() {
        let backdrop = seed("底", (0, 0), [255, 255, 255, 255], (1, 1), true, 1.0);
        let make = |pass_through: bool| {
            let mut red = seed("红", (0, 0), [255, 0, 0, 255], (1, 1), true, 1.0);
            red.group = Some(7);
            let mut blue = seed("蓝", (0, 0), [0, 0, 255, 128], (1, 1), true, 1.0);
            blue.group = Some(7);
            blue.knockout = Knockout::Shallow;
            Document::from_psd_layers_with_groups(
                (1, 1),
                vec![backdrop.clone(), red, blue],
                vec![group_seed(7, "组", BlendMode::Normal, 1.0, pass_through)],
            )
            .flatten()
            .get_pixel(0, 0)
            .0
        };
        // 隔离组与穿透组结果一致：组内红色被削掉一半，透出白底后的合成值。
        for pass_through in [false, true] {
            let got = make(pass_through);
            for (i, want) in [127u8, 64, 191, 255].iter().enumerate() {
                assert!(
                    (got[i] as i32 - *want as i32).abs() <= 1,
                    "pass_through={pass_through} 通道 {i}: got {got:?}"
                );
            }
        }
    }

    /// 根层（不在任何组里）的浅挖空等价于挖掉文档下层：
    /// 半透明挖空层下方不再垫着实底背景。
    #[test]
    fn root_shallow_knockout_clears_document_below() {
        let make = |knockout: Knockout| {
            let backdrop = seed("底", (0, 0), [255, 255, 255, 255], (1, 1), true, 1.0);
            let mut blue = seed("蓝", (0, 0), [0, 0, 255, 128], (1, 1), true, 1.0);
            blue.knockout = knockout;
            Document::from_psd_layers_with_groups(
                (1, 1),
                vec![backdrop, blue],
                Vec::new(),
            )
            .flatten()
            .get_pixel(0, 0)
            .0
        };
        assert_eq!(make(Knockout::None), [127, 127, 255, 255]);
        let got = make(Knockout::Shallow);
        for (i, want) in [85u8, 85, 255, 191].iter().enumerate() {
            assert!(
                (got[i] as i32 - *want as i32).abs() <= 1,
                "通道 {i}: got {got:?}"
            );
        }
    }

    /// 深挖空连文档背景一起挖：结果不再是不透明的白底合成，
    /// 而是留下与挖空覆盖率相应的透明。
    #[test]
    fn deep_knockout_removes_document_backdrop() {
        let make = |knockout: Knockout| {
            let backdrop = seed("底", (0, 0), [255, 255, 255, 255], (1, 1), true, 1.0);
            let mut red = seed("红", (0, 0), [255, 0, 0, 255], (1, 1), true, 1.0);
            red.group = Some(7);
            let mut blue = seed("蓝", (0, 0), [0, 0, 255, 128], (1, 1), true, 1.0);
            blue.group = Some(7);
            blue.knockout = knockout;
            Document::from_psd_layers_with_groups(
                (1, 1),
                vec![backdrop, red, blue],
                vec![group_seed(7, "组", BlendMode::Normal, 1.0, false)],
            )
            .flatten()
            .get_pixel(0, 0)
            .0
        };
        let shallow = make(Knockout::Shallow);
        assert_eq!(shallow[3], 255, "浅挖空只挖组内，背景仍是实底");
        let deep = make(Knockout::Deep);
        assert!(deep[3] < 240, "深挖空应把白底一起挖掉：{deep:?}");
    }

    /// 混合颜色带按本层颜色遮罩：白端 128 以上整片消失，其余照常。
    #[test]
    fn blend_if_hides_source_pixels_outside_range() {
        let white = seed("底", (0, 0), [255, 255, 255, 255], (2, 1), true, 1.0);
        let mut layer = seed("灰黑", (0, 0), [200, 200, 200, 255], (2, 1), true, 1.0);
        layer.image.put_pixel(1, 0, image::Rgba([0, 0, 0, 255]));
        layer.blend_if = Some(BlendIf {
            source: BlendRange {
                black: [0, 0],
                white: [128, 128],
            },
            ..Default::default()
        });
        let doc = Document::from_psd_layers((2, 1), vec![white, layer]);
        let flat = doc.flatten();
        assert_eq!(flat.get_pixel(0, 0).0, [255, 255, 255, 255], "亮像素被挡住");
        assert_eq!(flat.get_pixel(1, 0).0, [0, 0, 0, 255], "暗像素保留");
    }

    /// 下层全透明时只按本层范围限制，不会因为「下层黑」把浮动图层吃掉。
    #[test]
    fn blend_if_ignores_destination_when_backdrop_is_transparent() {
        let mut layer = seed("红", (0, 0), [255, 0, 0, 255], (2, 1), true, 1.0);
        layer.blend_if = Some(BlendIf {
            destination: BlendRange {
                black: [64, 64],
                white: [255, 255],
            },
            ..Default::default()
        });
        let doc = Document::from_psd_layers((2, 1), vec![layer]);
        assert_eq!(doc.flatten().get_pixel(0, 0).0, [255, 0, 0, 255]);
    }

    #[test]
    fn zero_index_is_transparent_background_and_order_is_preserved() {
        let doc = Document::from_psd_layers(
            (8, 6),
            vec![
                seed("bottom", (0, 0), [10, 20, 30, 255], (8, 6), true, 1.0),
                seed("top", (2, 1), [200, 100, 50, 128], (3, 2), false, 0.5),
            ],
        );
        assert_eq!(doc.layers.len(), 3, "背景 + 两个 PSD 层");
        assert!(doc.layers[0].background);
        assert!(doc.layers[0].locked);
        assert_eq!(
            doc.layers[0].image().unwrap().get_pixel(0, 0),
            &Rgba([0, 0, 0, 0])
        );
        assert_eq!(doc.canvas, Size::new(8, 6));
        assert_eq!(doc.view, Rect::new(0, 0, 8, 6));
        assert_eq!(doc.layers[1].name, "bottom");
        assert_eq!(doc.layers[1].offset(), (0, 0));
        assert_eq!(doc.layers[2].name, "top");
        assert_eq!(doc.layers[2].offset(), (2, 1));
        assert!(!doc.layers[2].visible);
        assert_eq!(doc.layers[2].opacity, 0.5);
        assert!(doc.layers[2].is_raster());
        assert_eq!(doc.edit_epoch, 0);
        assert!(doc.selection.is_none());
    }

    #[test]
    fn flatten_matches_visible_layers() {
        let doc = Document::from_psd_layers(
            (4, 4),
            vec![seed("opaque", (0, 0), [200, 10, 30, 255], (4, 4), true, 1.0)],
        );
        assert_eq!(doc.flatten().get_pixel(1, 1), &Rgba([200, 10, 30, 255]));
    }

    #[test]
    fn hidden_layer_does_not_appear_in_flatten() {
        let doc = Document::from_psd_layers(
            (4, 4),
            vec![seed("hidden", (0, 0), [200, 10, 30, 255], (4, 4), false, 1.0)],
        );
        assert_eq!(doc.flatten().get_pixel(1, 1).0[3], 0);
    }

    /// 蒙版把 alpha 按 `mask/255` 缩放：左边遮住的部分完全透明。
    #[test]
    fn layer_mask_multiplies_alpha() {
        use image::{GrayImage, Luma};
        let mut s = seed("masked", (0, 0), [200, 10, 30, 255], (4, 1), true, 1.0);
        let mut m = GrayImage::new(4, 1);
        for x in 0..4u32 {
            m.put_pixel(x, 0, Luma([if x < 2 { 0 } else { 255 }]));
        }
        s.mask = Some(LayerMask::new(m, (0, 0)));
        let doc = Document::from_psd_layers((4, 1), vec![s]);
        let flat = doc.flatten();
        assert_eq!(flat.get_pixel(0, 0).0[3], 0, "蒙版 0 处必须透明");
        assert_eq!(flat.get_pixel(3, 0).0, [200, 10, 30, 255]);
    }

    /// 剪贴层只在基底不透明的地方出现。
    #[test]
    fn clipped_layer_only_shows_where_base_is_opaque() {
        let base = seed("base", (0, 0), [0, 0, 0, 255], (2, 4), true, 1.0);
        let mut top = seed("clip", (0, 0), [255, 0, 0, 255], (4, 4), true, 1.0);
        top.clipping = true;
        let doc = Document::from_psd_layers((4, 4), vec![base, top]);
        let flat = doc.flatten();
        assert_eq!(flat.get_pixel(0, 0).0, [255, 0, 0, 255], "基底内应显示剪贴层");
        assert_eq!(flat.get_pixel(3, 0).0[3], 0, "基底外剪贴层必须不可见");
    }

    /// 画布点选的文档级命中：剪贴层在基底内容之外的 alpha 必须 = 0，
    /// 与渲染一致 —— 否则点空白会选中剪贴蒙版裁掉的透明区。
    #[test]
    fn hit_alpha_respects_clip_chain() {
        let base = seed("base", (0, 0), [0, 0, 0, 255], (2, 4), true, 1.0);
        let mut top = seed("clip", (0, 0), [255, 0, 0, 255], (4, 4), true, 1.0);
        top.clipping = true;
        let doc = Document::from_psd_layers((4, 4), vec![base, top]);
        // 装配会在 layers[0] 插透明占位背景，clip 层下标动态找。
        let clip_ix = doc.layers.iter().position(|l| l.clipping).unwrap();
        assert_eq!(doc.hit_alpha_at(clip_ix, 0, 0), 255, "基底内照常命中");
        assert_eq!(
            doc.hit_alpha_at(clip_ix, 3, 0),
            0,
            "基底外（蒙版裁掉的区域）不命中"
        );
        // 非剪贴层原样透传（基底自己在内容外框内仍命中）。
        let base_ix = doc.layers.iter().position(|l| !l.clipping && l.name == "base").unwrap();
        assert_eq!(doc.hit_alpha_at(base_ix, 0, 0), 255);
        // 隐藏基底 → 整条剪贴链一起消失（与渲染同语义）。
        let mut doc2 = doc.clone();
        doc2.layers[base_ix].visible = false;
        assert_eq!(doc2.hit_alpha_at(clip_ix, 0, 0), 0);
    }

    /// 基底带烘焙样式时，剪贴边界取「内容」形状，不取被样式撑大的位图 alpha。
    ///
    /// PS 的剪贴蒙版只认基底的内容：外描边 / 投影不算边界。烘焙后的位图
    /// alpha 包含描边，拿它当边界剪贴内容就会铺到描边上。
    #[test]
    fn clip_base_boundary_uses_content_shape_not_baked_effects() {
        // 基底位图 4×4 全不透明（= 内容 + 烘焙出来的描边），内容形状只有中间 2×2。
        let mut base = seed("base", (0, 0), [0, 0, 0, 255], (4, 4), true, 1.0);
        let mut shape = GrayImage::new(4, 4);
        for y in 1..3u32 {
            for x in 1..3u32 {
                shape.put_pixel(x, y, Luma([255]));
            }
        }
        base.clip_shape = Some(LayerMask::new(shape, (0, 0)));
        let mut top = seed("clip", (0, 0), [255, 0, 0, 255], (4, 4), true, 1.0);
        top.clipping = true;
        let doc = Document::from_psd_layers((4, 4), vec![base, top]);
        let flat = doc.flatten();
        assert_eq!(flat.get_pixel(1, 1).0, [255, 0, 0, 255], "内容内应显示剪贴层");
        assert_eq!(
            flat.get_pixel(0, 0).0,
            [0, 0, 0, 255],
            "描边带（内容形状之外）不能铺剪贴层"
        );
    }

    /// 基底被隐藏时，整条剪贴链跟着消失（PS 语义）。
    #[test]
    fn hidden_base_hides_the_whole_clip_chain() {
        let mut base = seed("base", (0, 0), [0, 0, 0, 255], (4, 4), true, 1.0);
        base.visible = false;
        let mut top = seed("clip", (0, 0), [255, 0, 0, 255], (4, 4), true, 1.0);
        top.clipping = true;
        let doc = Document::from_psd_layers((4, 4), vec![base, top]);
        assert_eq!(doc.flatten().get_pixel(1, 1).0[3], 0);
    }

    /// 剪贴链共享最下面那个非剪贴基底：两张剪贴层叠起来仍受同一个基底约束。
    #[test]
    fn clip_chain_shares_one_base() {
        let base = seed("base", (0, 0), [0, 0, 0, 255], (2, 2), true, 1.0);
        let mut a = seed("a", (0, 0), [255, 0, 0, 255], (4, 4), true, 1.0);
        a.clipping = true;
        let mut b = seed("b", (0, 0), [0, 0, 255, 255], (4, 4), true, 1.0);
        b.clipping = true;
        let doc = Document::from_psd_layers((4, 4), vec![base, a, b]);
        let flat = doc.flatten();
        assert_eq!(flat.get_pixel(0, 0).0, [0, 0, 255, 255]);
        assert_eq!(flat.get_pixel(3, 3).0[3], 0);
    }

    #[test]
    fn negative_offsets_are_kept_as_is() {
        let doc = Document::from_psd_layers(
            (4, 4),
            vec![seed("neg", (-3, -2), [1, 2, 3, 255], (5, 5), true, 1.0)],
        );
        assert_eq!(doc.layers[1].offset(), (-3, -2));
        assert_eq!(doc.layers[1].bounds().x, -3);
    }

    #[test]
    fn empty_seed_list_still_has_a_background() {
        let doc = Document::from_psd_layers((3, 2), vec![]);
        assert_eq!(doc.layers.len(), 1);
        assert!(doc.layers[0].background);
        assert_eq!(doc.canvas, Size::new(3, 2));
    }
}

impl Document {
    pub fn from_image(image: RgbaImage) -> Self {
        let canvas = Size::new(image.width(), image.height());
        Self {
            source: None,
            canvas,
            view: Rect::new(0, 0, canvas.width, canvas.height),
            layers: vec![Layer::background(image)],
            groups: Vec::new(),
            selection: None,
            edit_epoch: 0,
            next_layer_seq: 0,
            icc: None,
            history: History::new(256),
        }
    }

    /// Assemble an editable document from PSD layers.
    ///
    /// `layers[0]` is ALWAYS a transparent full-canvas background (the
    /// invariant the whole edit/history/panel system relies on); every PSD
    /// layer becomes an ordinary raster layer above it, bottom -> top.
    /// A PSD file is therefore opened with `1 + N` rows in the layer panel.
    pub fn from_psd_layers(canvas: (u32, u32), seeds: Vec<PsdLayerSeed>) -> Self {
        Self::from_psd_layers_with_groups(canvas, seeds, Vec::new())
    }

    /// Same as [`Document::from_psd_layers`], plus the PSD folder tree.
    ///
    /// Groups are **parallel metadata** (they never enter `layers`), so the
    /// composite still sees exactly the same pixel stack. `psd_groups` is in the
    /// same order as the file (bottom -> top), and a child group can appear
    /// before its parent, so the parent links are resolved in a second pass.
    pub fn from_psd_layers_with_groups(
        canvas: (u32, u32),
        seeds: Vec<PsdLayerSeed>,
        psd_groups: Vec<PsdGroupSeed>,
    ) -> Self {
        let size = Size::new(canvas.0.max(1), canvas.1.max(1));
        let mut layers = Vec::with_capacity(seeds.len() + 1);
        layers.push(Layer::background(RgbaImage::new(size.width, size.height)));
        let mut groups: Vec<LayerGroup> = Vec::with_capacity(psd_groups.len());
        let mut group_of: std::collections::HashMap<u32, GroupId> =
            std::collections::HashMap::with_capacity(psd_groups.len());
        for seed in &psd_groups {
            let group = LayerGroup::imported(seed.name.clone(), seed.collapsed);
            group_of.insert(seed.psd_id, group.id);
            groups.push(group);
        }
        // 父组回填（子组可能排在自己的父组之前）。
        for (i, seed) in psd_groups.iter().enumerate() {
            let parent = seed.parent.and_then(|pid| group_of.get(&pid).copied());
            groups[i].parent = parent;
            groups[i].visible = seed.visible;
            groups[i].blend = seed.blend;
            groups[i].opacity = seed.opacity.clamp(0.0, 1.0);
            groups[i].pass_through = seed.pass_through;
            groups[i].raw_style = seed.raw_style.clone();
        }
        // 成员自身旗标（折叠前），与 `layers[1..]` 一一对应；循环按值消费
        // `seeds`，先取出来。
        let own_flags: Vec<bool> = seeds.iter().map(|s| s.own_visible).collect();
        for seed in seeds {
            let mut layer = Layer::new(seed.name, seed.image);
            layer.visible = seed.visible;
            layer.opacity = seed.opacity.clamp(0.0, 1.0);
            layer.blend = seed.blend;
            layer.mask = seed.mask;
            layer.clipping = seed.clipping;
            layer.clip_shape = seed.clip_shape;
            layer.style = seed.style;
            layer.blend_if = seed.blend_if;
            layer.knockout = seed.knockout;
            layer.transparency_shapes = seed.transparency_shapes;
            layer.synthetic = seed.synthetic;
            layer.locked = seed.synthetic;
            layer.background = false;
            layer.group = seed.group.and_then(|gid| group_of.get(&gid).copied());
            if let Some(raster) = layer.raster_mut() {
                raster.offset = seed.offset;
            }
            layers.push(layer);
        }
        // PSD 里一开始就是隐藏的组：成员 visible 已被 decode 折叠成 false
        // （合成只认成员），但面板眼睛要显示成员自己的状态。给链上每个
        // 隐藏祖先组回填 `hidden_snapshot`，值为「自身旗标 && 介于中间的
        // 组都可见」。**子树全部成员都要入快照**（自身隐藏的记 false）：
        // `set_group_visible(true)` 对不在快照里的成员默认点亮
        // （`unwrap_or(true)`），漏记会把手动隐藏的成员误点亮。与
        // `set_group_visible(false)` 的落点一致 —— 眼睛加载后立刻正确。
        for (layer, &own) in layers.iter().skip(1).zip(own_flags.iter()) {
            if layer.visible {
                // 有隐藏祖先必被折叠成 false；仍可见说明链上没有隐藏组。
                continue;
            }
            let mut gid = layer.group;
            let mut cur = own;
            let mut guard = 0;
            while let Some(id) = gid {
                guard += 1;
                if guard > 64 {
                    break;
                }
                let Some(g) = groups.iter_mut().find(|g| g.id == id) else {
                    break;
                };
                if !g.visible {
                    g.hidden_snapshot.push((layer.id, cur));
                }
                cur &= g.visible;
                gid = g.parent;
            }
        }
        Self {
            source: None,
            canvas: size,
            view: Rect::new(0, 0, size.width, size.height),
            layers,
            groups,
            selection: None,
            edit_epoch: 0,
            next_layer_seq: 0,
            icc: None,
            history: History::new(256),
        }
    }

    pub fn execute(&mut self, op: EditOp) -> Result<(), EditError> {
        // 纯分组元数据不改变任何像素：不 bump edit_epoch，预览缓存 / 原生
        // 合成校准就不会因为「只是把几层收进一个组」而失效切回重建画面。
        // 面板刷新由 UI 侧的 groupViewEpoch 负责。
        let metadata_only = op.is_metadata_only();
        op.apply(self)?;
        self.history.push(op);
        if !metadata_only {
            self.edit_epoch = self.edit_epoch.wrapping_add(1);
        }
        Ok(())
    }

    // ---- 选区（可撤销的文档状态，见 `EditOp::SetSelection`）-------------

    /// 换一个选区（新建 / 反选 / 取消 / 平移 / 缩放）。与当前完全相同则
    /// 不产生历史条目。
    pub fn set_selection(&mut self, next: Option<SelectionState>) -> Result<(), EditError> {
        if self.selection == next {
            return Ok(());
        }
        let from = self.selection.clone();
        self.execute_selection(EditOp::SetSelection { from, to: next })
    }

    /// 拖动类工具（快速选择 / 笔刷选区）的临时选区：只改状态，**不进
    /// 历史、不 bump epoch**。
    ///
    /// 这类工具拖动期间每帧都要重算一次选区，若每帧都走 `set_selection`，
    /// 一次涂抹就会塞进几十条撤销记录（Ctrl+Z 要按到手酸）。松开时用
    /// [`Document::commit_selection_change`] 把整段操作补成一条。
    pub fn preview_selection(&mut self, next: Option<SelectionState>) {
        self.selection = next;
    }

    /// 把一次已经生效的选区变更记进历史，与 `preview_selection` 配对。
    pub fn commit_selection_change(&mut self, from: Option<SelectionState>) {
        let to = self.selection.clone();
        if from == to {
            return;
        }
        self.history.push(EditOp::SetSelection { from, to });
    }

    /// 改边界 / 平滑 / 羽化参数（从 base 重算，不叠加）。
    pub fn set_selection_ops(&mut self, ops: SelOps) -> Result<(), EditError> {
        let Some(state) = self.selection.as_ref() else {
            return Ok(());
        };
        let from = state.ops;
        if from == ops {
            return Ok(());
        }
        self.execute_selection(EditOp::SelectionOptions { from, to: ops })
    }

    /// 选区不碰任何一个像素，所以**不 bump `edit_epoch`**：预览合成器的
    /// 分层缓存按 epoch 失效，不该因为拖了一下选区就整块重新重采样。
    fn execute_selection(&mut self, op: EditOp) -> Result<(), EditError> {
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

    /// Crop to the selection's **outline** (not just its bounding box): the
    /// canvas shrinks to the bounds AND every pixel outside the outline fades
    /// to transparent (scaled by the mask's coverage, so feathered edges stay
    /// soft). Ellipse / lasso / feathered selections therefore crop to their
    /// own shape instead of leaving a hard rectangular cut. Base and layers
    /// are both punched; undo restores the whole snapshot.
    pub fn crop_to_selection(&mut self, selection: &SelectionMask) -> Result<(), EditError> {
        let canvas_rect = Rect::new(0, 0, self.canvas.width, self.canvas.height);
        let area = selection
            .bounds
            .intersect(canvas_rect)
            .ok_or(EditError::EmptyCrop)?;
        if area.is_empty() || selection.area() == 0 {
            return Err(EditError::EmptyCrop);
        }
        let from = self.snapshot();
        self.execute(EditOp::CropToSelection {
            from,
            selection: selection.clone(),
        })
    }

    /// Undo every crop, showing the full base image again.
    pub fn reset_crop(&mut self) -> Result<(), EditError> {
        let (bw, bh) = self
            .layers[0]
            .image()
            .map(|img| (img.width(), img.height()))
            .unwrap_or((self.canvas.width, self.canvas.height));
        if self.view.x == 0 && self.view.y == 0 && self.view.width == bw && self.view.height == bh {
            return Ok(());
        }
        self.execute(EditOp::CropDocument {
            from: self.view,
            to: Rect::new(0, 0, bw, bh),
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

    /// Change only the canvas resolution: no pixel is resampled. Content is
    /// re-anchored to the center of the new canvas (see `apply_canvas_resize`).
    pub fn resize_canvas(&mut self, to: Size) -> Result<(), EditError> {
        if to.width < 1 || to.height < 1 {
            return Err(EditError::InvalidCanvasSize);
        }
        let from = self.snapshot();
        self.execute(EditOp::CanvasResize { from, to })
    }

    pub fn add_layer(&mut self, layer: Layer) -> Result<usize, EditError> {
        let index = self.layers.len();
        self.execute(EditOp::AddLayer { layer, index })?;
        Ok(index)
    }

    /// Append a markup annotation as a new topmost layer. Returns the mark's
    /// ordinal among marks (0-based, z order) — the same numbering the panel
    /// and the `mark-*` script ops use.
    pub fn add_mark(&mut self, mark: Mark) -> Result<usize, EditError> {
        let ordinal = self.mark_count();
        let layer = Layer::from_mark(mark);
        self.execute(EditOp::AddLayer {
            layer,
            index: self.layers.len(),
        })?;
        Ok(ordinal)
    }

    /// Replace a markup annotation (move / resize / restyle), undoable.
    /// `index` is the **mark ordinal** (see [`Document::mark_layer_index`]).
    pub fn update_mark(&mut self, index: usize, to: Mark) -> Result<(), EditError> {
        let li = self
            .mark_layer_index(index)
            .ok_or(EditError::MarkIndex(index))?;
        let from = self.layers[li]
            .mark()
            .cloned()
            .ok_or(EditError::MarkIndex(index))?;
        self.execute(EditOp::UpdateMark {
            index: li,
            from,
            to,
        })
    }

    /// History-free live preview of [`Document::update_mark`] for handle
    /// drags; the committing call records the true pre-drag mark.
    pub fn update_mark_live(&mut self, index: usize, mark: Mark) -> Result<(), EditError> {
        let li = self
            .mark_layer_index(index)
            .ok_or(EditError::MarkIndex(index))?;
        match &mut self.layers[li].content {
            LayerContent::Mark(m) => {
                *m = mark;
                Ok(())
            }
            LayerContent::Raster(_) => Err(EditError::MarkIndex(index)),
        }
    }

    /// Remove a markup annotation, undoable.
    pub fn delete_mark(&mut self, index: usize) -> Result<(), EditError> {
        let li = self
            .mark_layer_index(index)
            .ok_or(EditError::MarkIndex(index))?;
        let layer = self.layers[li].clone();
        self.execute(EditOp::RemoveLayer {
            layer,
            index: li,
        })
    }

    // ---- 标记视图（标记现在是图层，这里按 z 序暴露序号 ↔ 图层下标）------

    /// 全部标记图层的图层下标，按 z 序（底 → 顶）。
    pub fn mark_layer_indices(&self) -> Vec<usize> {
        self.layers
            .iter()
            .enumerate()
            .filter(|(_, l)| l.is_mark())
            .map(|(i, _)| i)
            .collect()
    }

    pub fn mark_count(&self) -> usize {
        self.layers.iter().filter(|l| l.is_mark()).count()
    }

    /// 第 `k` 个标记（z 序）对应的图层下标。
    pub fn mark_layer_index(&self, k: usize) -> Option<usize> {
        self.mark_layer_indices().get(k).copied()
    }

    pub fn mark(&self, k: usize) -> Option<&Mark> {
        self.mark_layer_index(k)
            .and_then(|i| self.layers[i].mark())
    }

    pub fn mark_mut(&mut self, k: usize) -> Option<&mut Mark> {
        let i = self.mark_layer_index(k)?;
        self.layers[i].mark_mut()
    }

    pub fn marks(&self) -> impl Iterator<Item = &Mark> {
        self.layers.iter().filter_map(|l| l.mark())
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

    /// 图层混合模式（PS 面板顶部下拉）。与锁定一样进统一历史。
    pub fn set_layer_blend(&mut self, index: usize, blend: BlendMode) -> Result<(), EditError> {
        let from = self
            .layers
            .get(index)
            .ok_or(EditError::LayerIndex(index))?
            .blend;
        if from == blend {
            return Ok(());
        }
        self.execute(EditOp::SetBlend {
            index,
            from,
            to: blend,
        })
    }

    pub fn set_layer_locked(&mut self, index: usize, locked: bool) -> Result<(), EditError> {
        let from = self
            .layers
            .get(index)
            .ok_or(EditError::LayerIndex(index))?
            .locked;
        if from == locked {
            return Ok(());
        }
        self.execute(EditOp::SetLocked {
            index,
            from,
            to: locked,
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

    /// P0-2b2：移动也汇流到一条 `SetLayerTransform`。旧 `TranslateLayer`
    /// 只写 `layer.offset`，会让 params.center 与位图脱节（连续旋转 / 数值
    /// 面板随后读到错误中心）；参数级历史 undo 还能精确还原尺寸与角度。
    pub fn translate_layer(&mut self, index: usize, dx: i32, dy: i32) -> Result<(), EditError> {
        if dx == 0 && dy == 0 {
            return Ok(());
        }
        let layer = self
            .layers
            .get(index)
            .ok_or(EditError::LayerIndex(index))?;
        if layer.is_mark() {
            // 标记没有 offset，直接平移几何。
            let from = layer.offset();
            let to = (from.0 + dx, from.1 + dy);
            return self.execute(EditOp::TranslateLayer { index, from, to });
        }
        let r = layer.raster().ok_or(EditError::LayerIndex(index))?;
        let mut to = r.transform.clone().unwrap_or_else(|| {
            LayerTransform::from_offset(r.image.clone(), r.offset.0, r.offset.1)
        });
        to.center.0 += dx as f64;
        to.center.1 += dy as f64;
        self.set_layer_transform(index, Some(to))
    }

    /// 批量平移：所有目标包进一条 `EditOp::Batch` = **一条撤销**。
    ///
    /// 与 [`Document::translate_layer`] 同语义（参数层同步 `center`，标记层
    /// 走 `TranslateLayer`），只是把 N 层合成一步。多选拖动 / 组选中整组
    /// 拖动都走这里，否则「拖一下按一次 Ctrl+Z 只退一层」。
    ///
    /// 锁定 / 越界的层由调用方过滤后传入；core 遇到越界会整体失败（Batch
    /// 已生效的部分会回滚），不做静默跳过 —— 宁可整步不做，也不要「拖了
    /// 一半」的状态。
    pub fn translate_layers(
        &mut self,
        indices: &[usize],
        dx: i32,
        dy: i32,
    ) -> Result<(), EditError> {
        if dx == 0 && dy == 0 || indices.is_empty() {
            return Ok(());
        }
        let mut ops: Vec<EditOp> = Vec::with_capacity(indices.len());
        let mut seen: Vec<usize> = Vec::with_capacity(indices.len());
        for &i in indices {
            if seen.contains(&i) {
                continue;
            }
            seen.push(i);
            let layer = self
                .layers
                .get(i)
                .ok_or(EditError::LayerIndex(i))?;
            if layer.is_mark() {
                let from = layer.offset();
                ops.push(EditOp::TranslateLayer {
                    index: i,
                    from,
                    to: (from.0 + dx, from.1 + dy),
                });
                continue;
            }
            let r = layer.raster().ok_or(EditError::LayerIndex(i))?;
            let from = r.transform.clone().unwrap_or_else(|| {
                LayerTransform::from_offset(r.image.clone(), r.offset.0, r.offset.1)
            });
            let mut to = from.clone();
            to.center.0 += dx as f64;
            to.center.1 += dy as f64;
            ops.push(EditOp::SetLayerTransform {
                index: i,
                from: Some(from),
                to: Some(to),
            });
        }
        if ops.is_empty() {
            return Ok(());
        }
        self.execute(EditOp::Batch(ops))
    }

    /// 批量参数变换（缩放 / 旋转落定）：一次 `Batch` = 一条撤销。
    ///
    /// `items` 是 `(图层下标, 目标变换)`；与当前参数完全相同的条目会被
    /// 跳过（不产生空历史）。标记层没有参数变换，传进来会整体失败 ——
    /// 调用方应只挑位图图层。
    pub fn set_layer_transforms(
        &mut self,
        items: &[(usize, Option<LayerTransform>)],
    ) -> Result<(), EditError> {
        let mut ops: Vec<EditOp> = Vec::with_capacity(items.len());
        for (i, to) in items {
            let r = self
                .layers
                .get(*i)
                .and_then(|l| l.raster())
                .ok_or(EditError::LayerIndex(*i))?;
            let from = r.transform.clone().unwrap_or_else(|| {
                LayerTransform::from_offset(r.image.clone(), r.offset.0, r.offset.1)
            });
            if to.as_ref() == Some(&from) {
                continue;
            }
            ops.push(EditOp::SetLayerTransform {
                index: *i,
                from: Some(from),
                to: to.clone(),
            });
        }
        if ops.is_empty() {
            return Ok(());
        }
        self.execute(EditOp::Batch(ops))
    }

    /// P0-1 参数化变换入口。from/to 只带参数与 `Arc<原图>`，一条历史即可
    /// 精确撤销；P0-2 的统一会话会把移动 / 缩放 / 旋转全部汇流到这里。
    /// 传 `None` = 把该层烘焙成普通位图图层（丢弃无损状态）。
    pub fn set_layer_transform(
        &mut self,
        index: usize,
        to: Option<LayerTransform>,
    ) -> Result<(), EditError> {
        if index >= self.layers.len() {
            return Err(EditError::LayerIndex(index));
        }
        // 「变换前是普通位图图层」也必须留下可恢复的状态，否则 undo 时
        // 没有位图可还原（只有 None 会让 image 停在变换后的尺寸）。用当前
        // image 建一个 identity 状态，撤销走 1:1 索引快路径精确还原。
        let from = match self.layers[index].raster() {
            Some(r) => r.transform.clone().unwrap_or_else(|| {
                LayerTransform::from_offset(r.image.clone(), r.offset.0, r.offset.1)
            }),
            None => return Err(EditError::LayerIndex(index)),
        };
        if to.as_ref() == Some(&from) {
            return Ok(());
        }
        self.execute(EditOp::SetLayerTransform {
            index,
            from: Some(from),
            to,
        })
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

    /// Scale a layer's image to exactly `to` (Lanczos3) and place its
    /// top-left corner at `offset`, so handle-driven resizes anchor at the
    /// dragged corner / edge. Undo restores the full previous state via
    /// snapshot.
    pub fn scale_layer(
        &mut self,
        index: usize,
        to: Size,
        offset: (i32, i32),
    ) -> Result<(), EditError> {
        self.scale_layer_flipped(index, to, offset, (false, false))
    }

    /// `scale_layer` plus mirroring: `flip` mirrors the resampled bitmap on
    /// that axis. The UI passes `true` when a handle was dragged past the
    /// opposite edge (negative width/height), which is how "resize to a
    /// negative size" is expressed here — `Size` stays unsigned.
    pub fn scale_layer_flipped(
        &mut self,
        index: usize,
        to: Size,
        offset: (i32, i32),
        flip: (bool, bool),
    ) -> Result<(), EditError> {
        if to.width < 1 || to.height < 1 {
            return Err(EditError::InvalidCanvasSize);
        }
        if index >= self.layers.len() {
            return Err(EditError::LayerIndex(index));
        }
        let from = self.snapshot();
        self.execute(EditOp::ScaleLayer {
            index,
            from,
            to,
            offset,
            flip,
        })
    }

    /// Flip / rotate one layer in place (its box — offset and image size
    /// except for 90° rotations — stays where it is). This is the PS
    /// behaviour when a layer is selected: the transform hits that layer
    /// only, not the whole document.
    pub fn transform_layer(&mut self, index: usize, transform: Transform) -> Result<(), EditError> {
        if index >= self.layers.len() {
            return Err(EditError::LayerIndex(index));
        }
        self.execute(EditOp::TransformLayer { index, transform })
    }

    /// "Delete" the background: its pixels become fully transparent while the
    /// layer entries (and therefore the layer stack) stay untouched.
    pub fn delete_base(&mut self) -> Result<(), EditError> {
        let from = self.snapshot();
        self.execute(EditOp::DeleteBase { from })
    }

    /// Hide/show the background layer. No-op (no history entry) when the
    /// state already matches.
    pub fn set_base_visible(&mut self, visible: bool) -> Result<(), EditError> {
        if self.layers[0].visible == visible {
            return Ok(());
        }
        self.execute(EditOp::SetBaseVisible {
            from: !visible,
            to: visible,
        })
    }

    pub fn undo(&mut self) -> Result<(), EditError> {
        let op = self.history.undo.pop().ok_or(EditError::NothingToUndo)?;
        let metadata_only = op.is_metadata_only();
        op.revert(self)?;
        self.history.redo.push(op);
        if !metadata_only {
            self.edit_epoch = self.edit_epoch.wrapping_add(1);
        }
        Ok(())
    }

    pub fn redo(&mut self) -> Result<(), EditError> {
        let op = self.history.redo.pop().ok_or(EditError::NothingToRedo)?;
        let metadata_only = op.is_metadata_only();
        op.apply(self)?;
        self.history.undo.push(op);
        if !metadata_only {
            self.edit_epoch = self.edit_epoch.wrapping_add(1);
        }
        Ok(())
    }

    pub fn can_undo(&self) -> bool {
        !self.history.undo.is_empty()
    }

    /// 已提交历史的条数。旋转会话用它判断「图层在这之后有没有被别的操作
    /// 改过」—— 改过就不能再拿会话开头那份原图继续转，否则会吃掉别人的修改。
    pub fn history_len(&self) -> usize {
        self.history.undo.len()
    }

    pub fn can_redo(&self) -> bool {
        !self.history.redo.is_empty()
    }

    pub fn selection_layer(&self, selection: &SelectionMask) -> Result<Layer, EditError> {
        let canvas_rect = Rect::new(0, 0, self.canvas.width, self.canvas.height);
        let bounds = selection
            .bounds
            .intersect(canvas_rect)
            .ok_or(EditError::EmptySelection)?;
        if bounds.is_empty() || selection.area() == 0 {
            return Err(EditError::EmptySelection);
        }
        let mut image = RgbaImage::new(bounds.width, bounds.height);
        let mut any = false;
        for y in 0..bounds.height {
            for x in 0..bounds.width {
                let sx = bounds.x + x as i32;
                let sy = bounds.y + y as i32;
                let cov = selection.coverage_at(sx, sy);
                if cov == 0 {
                    continue;
                }
                any = true;
                // The selection is an alpha coverage map: feathered edges
                // come out of the extracted layer already semi-transparent.
                let mut px = self.composite_pixel(sx, sy);
                px.0[3] = ((px.0[3] as u32 * cov as u32 + 127) / 255) as u8;
                image.put_pixel(x, y, px);
            }
        }
        if !any {
            return Err(EditError::EmptySelection);
        }
        let mut layer = Layer::new("Selection", image);
        layer.set_offset((bounds.x, bounds.y));
        Ok(layer)
    }

    /// Next auto name for a created layer ("1", "2", ...), by creation order.
    pub fn next_layer_name(&mut self) -> String {
        self.next_layer_seq += 1;
        self.next_layer_seq.to_string()
    }

    pub fn extract_selection(&mut self, selection: &SelectionMask) -> Result<usize, EditError> {
        let mut layer = self.selection_layer(selection)?;
        layer.name = self.next_layer_name();
        let index = self.layers.len();
        self.execute(EditOp::AddLayer { layer, index })?;
        Ok(index)
    }

    /// Non-destructive "delete selection pixels": base pixels under the
    /// selection become transparent (scaled by the selection's coverage, so a
    /// feathered edge fades out instead of being cut); layers are untouched;
    /// undo restores the exact previous state via snapshot.
    pub fn erase_selection(&mut self, selection: &SelectionMask) -> Result<(), EditError> {
        let canvas_rect = Rect::new(0, 0, self.canvas.width, self.canvas.height);
        let clipped = selection
            .bounds
            .intersect(canvas_rect)
            .ok_or(EditError::EmptySelection)?;
        if clipped.is_empty() || selection.area() == 0 {
            return Err(EditError::EmptySelection);
        }
        let from = self.snapshot();
        self.execute(EditOp::EraseBasePixels {
            from,
            selection: selection.clone(),
        })
    }

    /// 选区像素的取样源由 `target` 决定：`Base` = 背景，`Layer(i)` = 第 i 层。
    ///
    /// 和 [`Document::selection_layer`] 的区别：那个从**可见合成**里抠（合并
    /// 拷贝），这里只取单层像素。PS 的复制 / 剪切 / 提取默认只作用于当前
    /// 图层 —— 否则选区一抠就把上层的内容也带进来。图层可能只覆盖画布的
    /// 一部分，所以产出范围 = 选区包围盒 ∩ 图层矩形。
    pub fn selection_layer_on(
        &self,
        target: AdjustTarget,
        selection: &SelectionMask,
    ) -> Result<Layer, EditError> {
        let canvas_rect = Rect::new(0, 0, self.canvas.width, self.canvas.height);
        let mut bounds = selection
            .bounds
            .intersect(canvas_rect)
            .ok_or(EditError::EmptySelection)?;
        if let AdjustTarget::Layer(index) = target {
            let r = self
                .layers
                .get(index)
                .and_then(|l| l.raster())
                .ok_or(EditError::LayerIndex(index))?;
            let layer_rect = Rect::new(
                r.offset.0,
                r.offset.1,
                r.image.width(),
                r.image.height(),
            );
            bounds = bounds
                .intersect(layer_rect)
                .ok_or(EditError::EmptySelection)?;
        }
        if bounds.is_empty() || selection.area() == 0 {
            return Err(EditError::EmptySelection);
        }
        let mut image = RgbaImage::new(bounds.width, bounds.height);
        let mut any = false;
        for y in 0..bounds.height {
            for x in 0..bounds.width {
                let sx = bounds.x + x as i32;
                let sy = bounds.y + y as i32;
                let cov = selection.coverage_at(sx, sy);
                if cov == 0 {
                    continue;
                }
                let mut px = match target {
                    AdjustTarget::Base => self.base_pixel_at(sx, sy),
                    AdjustTarget::Layer(index) => self.layer_pixel(index, sx, sy),
                };
                // 图层在这个位置本来就是透明的：留透明，别把一个"看得见空
                // 洞"的图层抠出来。
                if px[3] == 0 {
                    continue;
                }
                any = true;
                px[3] = ((px[3] as u32 * cov as u32 + 127) / 255) as u8;
                image.put_pixel(x, y, image::Rgba(px));
            }
        }
        if !any {
            return Err(EditError::EmptySelection);
        }
        let mut layer = Layer::new("Selection", image);
        layer.set_offset((bounds.x, bounds.y));
        Ok(layer)
    }

    /// 把选区像素抠成新图层，取样源见 [`Document::selection_layer_on`]。
    pub fn extract_selection_on(
        &mut self,
        target: AdjustTarget,
        selection: &SelectionMask,
    ) -> Result<usize, EditError> {
        let mut layer = self.selection_layer_on(target, selection)?;
        layer.name = self.next_layer_name();
        let index = self.layers.len();
        self.execute(EditOp::AddLayer { layer, index })?;
        Ok(index)
    }

    /// 「删除选区像素」，只挖 `target` 那一层：base 走整份快照的
    /// `EraseBasePixels`，图层走一块像素补丁（前后两份位图各留一份，
    /// 撤销/重做都不必复制整层），覆盖率缩放 alpha，羽化边自然淡化。
    /// 锁定图层不动（调用方负责提示）。
    pub fn erase_selection_on(
        &mut self,
        target: AdjustTarget,
        selection: &SelectionMask,
    ) -> Result<(), EditError> {
        let AdjustTarget::Layer(index) = target else {
            return self.erase_selection(selection);
        };
        let layer = self.layers.get(index).ok_or(EditError::LayerIndex(index))?;
        if layer.locked {
            return Err(EditError::LayerIndex(index));
        }
        let r = layer.raster().ok_or(EditError::LayerIndex(index))?;
        let canvas_rect = Rect::new(0, 0, self.canvas.width, self.canvas.height);
        let layer_rect = Rect::new(
            r.offset.0,
            r.offset.1,
            r.image.width(),
            r.image.height(),
        );
        let area = selection
            .bounds
            .intersect(canvas_rect)
            .and_then(|r| r.intersect(layer_rect))
            .ok_or(EditError::EmptySelection)?;
        if area.is_empty() || selection.area() == 0 {
            return Err(EditError::EmptySelection);
        }
        let lx = area.x - r.offset.0;
        let ly = area.y - r.offset.1;
        let (rect, before) = crop_region(
            &r.image,
            Rect::new(lx, ly, area.width, area.height),
        );
        if rect.is_empty() {
            return Err(EditError::EmptySelection);
        }
        let mut after = before.clone();
        for y in 0..rect.height {
            for x in 0..rect.width {
                let cov = selection.coverage_at(area.x + x as i32, area.y + y as i32);
                if cov == 0 {
                    continue;
                }
                let px = after.get_pixel_mut(x, y);
                px.0[3] = ((px.0[3] as u32 * (255 - cov as u32) + 127) / 255) as u8;
                if px.0[3] == 0 {
                    px.0 = [0, 0, 0, 0];
                }
            }
        }
        self.commit_patch(target, rect, before, after)
    }


    /// Pixels a color adjustment would touch (histogram source / live scope).
    /// `AdjustTarget::Base` 恒指背景层（`layers[0]`）。
    pub fn scope_image(&self, target: AdjustTarget) -> Option<&RgbaImage> {
        match target {
            AdjustTarget::Base => self.layers[0].image(),
            AdjustTarget::Layer(index) => self.layers.get(index).and_then(|layer| layer.image()),
        }
    }

    /// Histogram of the adjustment scope ("调整颜色" panel graph + auto levels).
    pub fn scope_histogram(&self, target: AdjustTarget) -> Option<Histogram> {
        self.scope_image(target).map(Histogram::of)
    }

    /// Bake `params` into the scope's pixels as one undoable operation.
    /// A neutral parameter set is a no-op (no history entry, no epoch bump).
    pub fn adjust_pixels(
        &mut self,
        target: AdjustTarget,
        params: &Adjust,
    ) -> Result<(), EditError> {
        let from = match target {
            AdjustTarget::Base => self.base_image().clone(),
            AdjustTarget::Layer(index) => self
                .layers
                .get(index)
                .and_then(|layer| layer.image())
                .map(|img| img.clone())
                .ok_or(EditError::LayerIndex(index))?,
        };
        if params.is_identity() {
            return Ok(());
        }
        self.execute(EditOp::AdjustPixels {
            target,
            from,
            params: *params,
        })
    }

    /// 拖动中的实时预览：把一块像素补丁直接写进目标并 bump epoch（渲染缓存
    /// 因此失效、下一帧就是新像素），**不**写历史。松手时由 `commit_patch`
    /// 把同一个矩形的前后两份位图补成一条撤销记录。
    pub fn write_patch(&mut self, target: AdjustTarget, patch: &Patch) {
        if patch.is_empty() {
            return;
        }
        apply_region(self, target, patch.rect, &patch.image);
        self.edit_epoch = self.edit_epoch.wrapping_add(1);
    }

    /// 提交一块已经生效的像素改动。`before` / `after` 是同一矩形的两份位图，
    /// 撤销贴回 before、重做贴回 after，不必为一次涂抹留整图层快照。
    pub fn commit_patch(
        &mut self,
        target: AdjustTarget,
        rect: Rect,
        before: RgbaImage,
        after: RgbaImage,
    ) -> Result<(), EditError> {
        if rect.width == 0 || rect.height == 0 {
            return Ok(());
        }
        self.execute(EditOp::PixelPatch {
            target,
            rect,
            before,
            after,
        })
    }

    /// 实时预览用：直接替换某图层的位图 / 偏移，不写历史，只 bump epoch。
    /// 拖动旋转手柄或角度对话框改数值时每帧调它。
    pub fn preview_layer(&mut self, index: usize, layer: Layer) -> Result<(), EditError> {
        if index >= self.layers.len() {
            return Err(EditError::LayerIndex(index));
        }
        self.layers[index] = layer;
        self.edit_epoch = self.edit_epoch.wrapping_add(1);
        Ok(())
    }

    /// 提交一次图层整体替换（旋转手柄 / 角度对话框松手时）。
    pub fn commit_layer(
        &mut self,
        index: usize,
        before: Layer,
        after: Layer,
    ) -> Result<(), EditError> {
        if index >= self.layers.len() {
            return Err(EditError::LayerIndex(index));
        }
        if before == after {
            return Ok(());
        }
        self.execute(EditOp::ReplaceLayer {
            index,
            before,
            after,
        })
    }

    /// 任意角度旋转某个图层（绕图层中心）。`expand` = true 时位图按旋转后
    /// 的包围盒放大，内容不裁切；false 时保持原尺寸。
    pub fn rotate_layer_angle(
        &mut self,
        index: usize,
        degrees: f32,
        expand: bool,
    ) -> Result<(), EditError> {
        let layer = self
            .layers
            .get(index)
            .ok_or(EditError::LayerIndex(index))?;
        if degrees.abs() < 0.01 {
            return Ok(());
        }
        let before = layer.clone();
        let prev = layer.raster().ok_or(EditError::LayerIndex(index))?;
        // 落定用双三次：双线性转完一圈下来整层会明显发软（只此一次，代价
        // 摊不进拖动帧，没有理由省）。
        let (image, delta) = rotate_image_hq(&prev.image, degrees, expand);
        let mut after = before.clone();
        // after 的位图是从 before.image 重新旋转出来的，原始像素已经不再
        // 对应新的内容，烘焙掉非破坏性状态（P0-2 会把旋转也参数化）。
        after.ensure_baked();
        let (po, pi) = (prev.offset, image.clone());
        match &mut after.content {
            LayerContent::Raster(r) => {
                r.image = pi;
                r.offset = (po.0 + delta.0, po.1 + delta.1);
            }
            LayerContent::Mark(_) => return Err(EditError::LayerIndex(index)),
        }
        if after == before {
            return Ok(());
        }
        self.execute(EditOp::ReplaceLayer {
            index,
            before,
            after,
        })
    }

    /// 单点合成（**只含位图图层**）。
    ///
    /// 矢量标记不参与逐像素采样 —— 与统一模型之前的行为一致（旧实现的
    /// `composite_pixel` 也只看 base + layers，标记仅在 `flatten` 里绘制）。
    /// 魔术棒「取样全部图层」因此不会把标记颜色算进去。
    pub fn composite_pixel(&self, x: i32, y: i32) -> image::Rgba<u8> {
        if x < 0 || y < 0 || x >= self.canvas.width as i32 || y >= self.canvas.height as i32 {
            return image::Rgba([0, 0, 0, 0]);
        }
        let mut out = [0u8, 0, 0, 0];
        for layer in &self.layers {
            if !layer.visible || layer.opacity <= 0.0 {
                continue;
            }
            let Some(r) = layer.raster() else {
                continue;
            };
            let lx = x - r.offset.0;
            let ly = y - r.offset.1;
            if lx < 0
                || ly < 0
                || lx >= r.image.width() as i32
                || ly >= r.image.height() as i32
            {
                continue;
            }
            let src = r.image.get_pixel(lx as u32, ly as u32).0;
            blend_over_with(&mut out, src, layer.opacity, layer.blend, x, y);
        }
        image::Rgba(out)
    }

    /// 魔术棒采样：选区的形状由「某一层（或可见合成）的颜色」决定。
    ///
    /// `target = None` → 可见合成（PS 的「取样全部图层」）；`Some(Base)` /
    /// `Some(Layer(i))` → 只取那一层。坐标是画布坐标，图层偏移在这里换算，
    /// 调用方（QML/探针）给的都是用户在画布上点到的位置。
    pub fn magic_wand_selection(
        &self,
        x: i32,
        y: i32,
        opts: WandOptions,
        target: Option<AdjustTarget>,
    ) -> SelectionMask {
        match target {
            None => SelectionMask::magic_wand(self.canvas, (x, y), opts, |px, py| {
                self.composite_pixel(px, py).0
            }),
            Some(AdjustTarget::Base) => {
                SelectionMask::magic_wand(self.canvas, (x, y), opts, |px, py| {
                    self.base_pixel_at(px, py)
                })
            }
            Some(AdjustTarget::Layer(index)) => {
                SelectionMask::magic_wand(self.canvas, (x, y), opts, |px, py| {
                    self.layer_pixel(index, px, py)
                })
            }
        }
    }

    /// 快速选择的取样源，规则与 `magic_wand_selection` 相同：
    /// `None` = 可见合成（取样全部图层），`Some(..)` = 只取该层。
    pub fn quick_select_selection(
        &self,
        pen: &[(f32, f32)],
        radius: u32,
        opts: QuickSelectOptions,
        target: Option<AdjustTarget>,
    ) -> SelectionMask {
        match target {
            None => SelectionMask::quick_select(self.canvas, pen, radius, opts, |px, py| {
                self.composite_pixel(px, py).0
            }),
            Some(AdjustTarget::Base) => {
                SelectionMask::quick_select(self.canvas, pen, radius, opts, |px, py| {
                    self.base_pixel_at(px, py)
                })
            }
            Some(AdjustTarget::Layer(index)) => {
                SelectionMask::quick_select(self.canvas, pen, radius, opts, |px, py| {
                    self.layer_pixel(index, px, py)
                })
            }
        }
    }

    /// 背景像素（画布坐标 → 背景层坐标）。不可见 / 越界 = 全透明。
    pub fn base_pixel_at(&self, x: i32, y: i32) -> [u8; 4] {
        let bg = &self.layers[0];
        if !bg.visible {
            return [0, 0, 0, 0];
        }
        let Some(r) = bg.raster() else {
            return [0, 0, 0, 0];
        };
        let bx = x - r.offset.0;
        let by = y - r.offset.1;
        if bx < 0 || by < 0 || bx as u32 >= r.image.width() || by as u32 >= r.image.height() {
            return [0, 0, 0, 0];
        }
        r.image.get_pixel(bx as u32, by as u32).0
    }

    /// 单个图层的像素（画布坐标 → 图层坐标）。不可见 / 图层外 / 标记 = 全透明。
    pub fn layer_pixel(&self, index: usize, x: i32, y: i32) -> [u8; 4] {
        let Some(layer) = self.layers.get(index) else {
            return [0, 0, 0, 0];
        };
        if !layer.visible || layer.opacity <= 0.0 {
            return [0, 0, 0, 0];
        }
        let Some(r) = layer.raster() else {
            return [0, 0, 0, 0];
        };
        let lx = x - r.offset.0;
        let ly = y - r.offset.1;
        if lx < 0 || ly < 0 || lx as u32 >= r.image.width() || ly as u32 >= r.image.height() {
            return [0, 0, 0, 0];
        }
        let mut p = r.image.get_pixel(lx as u32, ly as u32).0;
        p[3] = (p[3] as f32 * layer.opacity).round().clamp(0.0, 255.0) as u8;
        p
    }

    /// 全幅合成（**显示口径**）：先按 [`Document::flatten_in_doc_space`] 在
    /// 文档空间合成，再按 `display` 路径转成 sRGB。没有内嵌 ICC / 开关关闭
    /// 时与前者完全一致，不会多一次转换。
    pub fn flatten(&self) -> RgbaImage {
        let mut out = self.flatten_in_doc_space();
        self.apply_display_color(&mut out);
        out
    }

    /// 把一张合成图从文档空间转到显示空间（sRGB）。没有内嵌 ICC 时是空操作。
    pub fn apply_display_color(&self, image: &mut RgbaImage) {
        if let Some(icc) = &self.icc {
            if let Ok(xform) = icc.lock() {
                xform.apply(image);
            }
        }
    }

    /// 按 `IMAGEEDIT_PSD_ICC` 把 PSD 内嵌 profile 挂到文档上。
    ///
    /// 只有 `display` 路径会建变换：`decode` 路径的图层位图在解码时已经是
    /// sRGB，这里再转就是双重转换；`off` 路径保持原样。
    pub fn set_icc_profile(&mut self, profile: Option<&[u8]>) {
        self.icc = match (IccMode::from_env(), profile) {
            (IccMode::Display, Some(bytes)) => IccTransform::shared(bytes),
            _ => None,
        };
    }

    /// 全幅合成（**文档色彩空间**口径）：按 `layers` 顺序自底向上，位图按
    /// offset + 不透明度混合，标记按不透明度栅格化到累积图。**顺序即 z 序**
    /// —— 标记不再天然置顶。
    ///
    /// 两件 PSD 语义在这里落地：
    /// - **图层蒙版**：`alpha *= mask/255`（蒙版有自己的矩形）；
    /// - **剪贴蒙版**：剪贴层只在下方第一个非剪贴层的不透明区域内可见。
    ///
    /// 累积缓冲是 f32 直通 alpha，**整幅只在最后量化一次**。旧实现每叠一层
    /// 就 round 到 u8，这个文件平均每像素叠 33 层，累积漂移 1~2/255 —— 在
    /// 保真模式下就是肉眼可见的色差。
    ///
    /// PSD 写回必须用这一份：文件里内嵌的 profile 是文档空间，写 sRGB 像素
    /// 进去会让 PS 重开时整体偏色。
    pub fn flatten_in_doc_space(&self) -> RgbaImage {
        let w = self.canvas.width;
        let h = self.canvas.height;
        if w == 0 || h == 0 {
            return RgbaImage::new(w, h);
        }
        let n = w as usize * h as usize;
        let mut acc = vec![[0f32; 4]; n];
        // 剪贴基底层的 alpha 覆盖率（不含不透明度）。剪贴层逐像素乘上它；
        // 隐藏的基底覆盖率是 0，于是整条剪贴链一起消失 —— 与 PS 一致。
        let mut clip = vec![0f32; n];
        self.composite_range(&self.layers, &mut acc, &mut clip, w, h, None, None);
        acc_to_image(&acc, w, h)
    }

    /// 合成一段连续的图层：整份文档，或某个组的成员。
    ///
    /// PS 的组是「整组先合成一块，再按组自己的混合模式与不透明度盖到下层」，
    /// 与成员模式逐层直接叠加是两回事。遇到需要隔离的组（非穿透，或组不透明度
    /// < 100%）时，把它连续的成员递归合成到独立缓冲，再整体混合一次；穿透组
    /// 仍按原来的逐层叠加走，保证既有画面不变。
    ///
    /// `base` 是当前组起点处的累积背景，只有穿透组才给（浅挖空要把组内下层
    /// 恢复成它）；隔离组与根层传 `None`，等价于「背景是透明」。
    ///
    /// 返回深挖空涉及的像素下标与保留系数：调用方在把本段结果混到自己的
    /// 累积缓冲前，要先把这些像素按下层内容乘上保留系数 —— 深挖空要求连
    /// 组外的背景一起挖掉。
    fn composite_range(
        &self,
        layers: &[Layer],
        acc: &mut [[f32; 4]],
        clip: &mut [f32],
        w: u32,
        h: u32,
        inside: Option<GroupId>,
        base: Option<&[[f32; 4]]>,
    ) -> Vec<(usize, f32)> {
        let mut deep: Vec<(usize, f32)> = Vec::new();
        let mut i = 0;
        while i < layers.len() {
            let layer = &layers[i];
            if let Some(gid) = layer.group {
                if inside != Some(gid) {
                    if let Some(group) = self.groups.iter().find(|g| g.id == gid) {
                        let end = self.group_run_end(layers, i, gid);
                        let has_knockout =
                            layers[i..end].iter().any(|l| l.knockout != Knockout::None);
                        if group.isolated() {
                            let n = w as usize * h as usize;
                            let mut sub = vec![[0f32; 4]; n];
                            let mut sub_clip = vec![0f32; n];
                            let sub_deep = self.composite_range(
                                &layers[i..end],
                                &mut sub,
                                &mut sub_clip,
                                w,
                                h,
                                Some(gid),
                                None,
                            );
                            for &(p, keep) in &sub_deep {
                                for c in 0..4 {
                                    acc[p][c] *= keep;
                                }
                            }
                            blend_group(acc, &sub, group);
                            deep.extend(sub_deep);
                            i = end;
                            continue;
                        } else if has_knockout {
                            // 穿透组的成员直接跟组下方的画面混合。要支持浅挖空
                            // （只挖组内成员），必须先把组起点的背景复制一份，
                            // 组内挖空时恢复它；否则分不清「组内下层」和
                            // 「组外背景」。组里没有挖空层时仍走原来的逐层路径。
                            let n = w as usize * h as usize;
                            let base_snapshot = acc.to_vec();
                            let mut sub = base_snapshot.clone();
                            let mut sub_clip = vec![0f32; n];
                            let sub_deep = self.composite_range(
                                &layers[i..end],
                                &mut sub,
                                &mut sub_clip,
                                w,
                                h,
                                Some(gid),
                                Some(&base_snapshot),
                            );
                            acc.copy_from_slice(&sub);
                            deep.extend(sub_deep);
                            i = end;
                            continue;
                        }
                    }
                }
            }
            if !layer.clipping {
                // 隐藏 / 全透明的基底覆盖率为 0，于是整条剪贴链一起消失。
                // 注意：基底只是「半透明」（opacity 0.5）时覆盖率仍是满的 ——
                // PS 里降基底层不透明度不会削弱上面的剪贴层。
                if layer.visible && layer.opacity > 0.0 {
                    fill_clip_coverage(clip, w, h, layer);
                } else {
                    clip.fill(0.0);
                }
            }
            if layer.visible && layer.opacity > 0.0 {
                let clip_alpha = if layer.clipping {
                    Some(&clip[..])
                } else {
                    None
                };
                match &layer.content {
                    LayerContent::Raster(r) => {
                        composite_raster(acc, w, h, r, layer, clip_alpha, base, &mut deep)
                    }
                    LayerContent::Mark(m) => {
                        // 标记不能先栅格化到空图：马赛克要采样**当前已合成**的
                        // 画面才能算出块平均。所以先把累积缓冲物化，再按旧语义
                        // 直接画上去，最后写回。
                        let mut tmp = acc_to_image(acc, w, h);
                        if layer.opacity >= 0.999 {
                            m.draw(&mut tmp);
                        } else {
                            let before = tmp.clone();
                            m.draw(&mut tmp);
                            for (dst, src) in tmp.pixels_mut().zip(before.pixels()) {
                                let p = dst.0;
                                let q = src.0;
                                dst.0 = [
                                    (p[0] as f32 * (1.0 - layer.opacity)
                                        + q[0] as f32 * layer.opacity)
                                        .round() as u8,
                                    (p[1] as f32 * (1.0 - layer.opacity)
                                        + q[1] as f32 * layer.opacity)
                                        .round() as u8,
                                    (p[2] as f32 * (1.0 - layer.opacity)
                                        + q[2] as f32 * layer.opacity)
                                        .round() as u8,
                                    (p[3] as f32 * (1.0 - layer.opacity)
                                        + q[3] as f32 * layer.opacity)
                                        .round() as u8,
                                ];
                            }
                        }
                        store_image(acc, &tmp);
                    }
                }
            }
            i += 1;
        }
        deep
    }

    /// `gid` 组从 `start` 开始的一段连续成员（含嵌套组）。PSD 里组总是连续
    /// 的一块，应用内拖动也可能把成员拖散 —— 拖散后按连续段处理，行为可预测。
    fn group_run_end(&self, layers: &[Layer], start: usize, gid: GroupId) -> usize {
        let mut end = start;
        while end < layers.len() {
            match layers[end].group {
                Some(g) if self.belongs_to_group(g, gid) => end += 1,
                _ => break,
            }
        }
        end.max(start + 1)
    }

    fn belongs_to_group(&self, gid: GroupId, target: GroupId) -> bool {
        let mut cur = Some(gid);
        let mut guard = 0;
        while let Some(g) = cur {
            if g == target {
                return true;
            }
            cur = self.groups.iter().find(|x| x.id == g).and_then(|x| x.parent);
            guard += 1;
            if guard > 64 {
                break;
            }
        }
        false
    }
}

/// 把隔离组的合成结果整体混合到下层：颜色/覆盖率来自组缓冲，模式和
/// 不透明度来自组记录。
fn blend_group(acc: &mut [[f32; 4]], sub: &[[f32; 4]], group: &LayerGroup) {
    if group.opacity <= 0.0 {
        return;
    }
    for (dst, src) in acc.iter_mut().zip(sub.iter()) {
        let a = src[3];
        if a <= 0.0 {
            continue;
        }
        let straight = [src[0] / a, src[1] / a, src[2] / a, a];
        crate::blend::blend_pixel_f32(dst, straight, group.opacity, group.blend);
    }
}

/// 把 `src` 按 `opacity` 叠到 `out` 上（正常模式，与旧 `composite_pixel` 逐像素一致）。
#[allow(dead_code)]
pub(crate) fn blend_over(out: &mut [u8; 4], src: [u8; 4], opacity: f32) {
    blend_over_with(out, src, opacity, BlendMode::Normal, 0, 0);
}

/// 带混合模式的版本：PSD 打开时带回的 `Multiply` / `Color` 等都走这里。
/// `(x, y)` 是画布坐标，只有「溶解」需要它做抖动。
pub(crate) fn blend_over_with(
    out: &mut [u8; 4],
    src: [u8; 4],
    opacity: f32,
    mode: BlendMode,
    x: i32,
    y: i32,
) {
    if mode.is_dissolve() {
        let keep = crate::blend::dissolve_hit(x, y, src[3] as f32 / 255.0 * opacity);
        let (src, opacity) = if keep {
            ([src[0], src[1], src[2], 255], 1.0)
        } else {
            ([0, 0, 0, 0], 0.0)
        };
        crate::blend::blend_pixel(out, src, opacity, BlendMode::Normal);
        return;
    }
    crate::blend::blend_pixel(out, src, opacity, mode);
}

/// 把一个位图图层按 offset 混合进整幅画布。
#[allow(dead_code)]
fn blend_raster_into(out: &mut RgbaImage, r: &RasterLayer, opacity: f32, mode: BlendMode) {
    blend_image_at_mode(out, &r.image, r.offset.0, r.offset.1, opacity, mode);
}

/// 把 f32 累积缓冲量化成一张 8 位图（整幅唯一的一次量化）。
///
/// 缓冲里存的是**预乘**色，输出的 8 位图是**直通** alpha（全工程、PSD 写回、
/// 缩略图都按直通读）。少了这一步除法，半透明画布的整幅画面会按覆盖率变暗：
/// alpha=152 的纯白会被写成 (152,152,152)，bg.psd 这类「透明底 + 半透明贴图」
/// 的稿子整片偏暗（实测 MAE 13.66，除法之后 0.20）。
fn acc_to_image(acc: &[[f32; 4]], w: u32, h: u32) -> RgbaImage {
    let mut out = RgbaImage::new(w, h);
    for (px, v) in out.pixels_mut().zip(acc.iter()) {
        let a = v[3];
        let (r, g, b) = if a > 0.0 {
            (v[0] / a, v[1] / a, v[2] / a)
        } else {
            (0.0, 0.0, 0.0)
        };
        px.0 = [
            (r * 255.0).round().clamp(0.0, 255.0) as u8,
            (g * 255.0).round().clamp(0.0, 255.0) as u8,
            (b * 255.0).round().clamp(0.0, 255.0) as u8,
            (a * 255.0).round().clamp(0.0, 255.0) as u8,
        ];
    }
    out
}

/// 把一张 8 位图写回 f32 累积缓冲（标记直绘之后用）。
///
/// 输入是直通 alpha 的 8 位图（标记栅格化 / 蒙版绘制都在直通空间工作），
/// 缓冲则按预乘保存，所以这里乘回 alpha。
fn store_image(acc: &mut [[f32; 4]], img: &RgbaImage) {
    for (v, px) in acc.iter_mut().zip(img.pixels()) {
        let p = px.0;
        let a = p[3] as f32 / 255.0;
        *v = [
            p[0] as f32 / 255.0 * a,
            p[1] as f32 / 255.0 * a,
            p[2] as f32 / 255.0 * a,
            a,
        ];
    }
}

/// 写出一层自身的 alpha 覆盖率（位图 alpha × 蒙版），画布尺寸、f32 0..=1。
///
/// 只用于剪贴蒙版的基底层：剪贴层看到的是基底的**不透明度**，而不是它的
/// 混合效果，所以这里不能走合成路径。隐藏的基底同样要写（写出来是 0），
/// 否则上一条剪贴链会错误地沿用更下面那层的覆盖率。
fn fill_clip_coverage(out: &mut [f32], w: u32, h: u32, layer: &Layer) {
    out.fill(0.0);
    // 基底带烘焙样式时，剪贴边界取它的**内容**形状（样式画出来的外描边 /
    // 投影不算内容）。拿烘焙后的位图 alpha 当边界会把剪贴内容铺到描边上。
    if let Some(shape) = layer.clip_shape.as_ref() {
        let x0 = shape.offset.0.max(0);
        let y0 = shape.offset.1.max(0);
        let x1 = (shape.offset.0 + shape.image.width() as i32).min(w as i32);
        let y1 = (shape.offset.1 + shape.image.height() as i32).min(h as i32);
        for y in y0..y1 {
            for x in x0..x1 {
                let mut cov = shape.coverage(x, y);
                if cov <= 0.0 {
                    continue;
                }
                if let Some(m) = layer.mask.as_ref() {
                    cov *= m.coverage(x, y);
                }
                if cov > 0.0 {
                    out[y as usize * w as usize + x as usize] = cov;
                }
            }
        }
        return;
    }
    match &layer.content {
        LayerContent::Raster(r) => {
            write_coverage(out, w, h, &r.image, r.offset, layer.mask.as_ref())
        }
        LayerContent::Mark(_) => {
            let (plane, offset) = layer.plane();
            write_coverage(out, w, h, &plane, offset, layer.mask.as_ref())
        }
    }
}

fn write_coverage(
    out: &mut [f32],
    w: u32,
    h: u32,
    img: &RgbaImage,
    offset: (i32, i32),
    mask: Option<&LayerMask>,
) {
    let x0 = offset.0.max(0);
    let y0 = offset.1.max(0);
    let x1 = (offset.0 + img.width() as i32).min(w as i32);
    let y1 = (offset.1 + img.height() as i32).min(h as i32);
    for y in y0..y1 {
        for x in x0..x1 {
            let a = img.get_pixel((x - offset.0) as u32, (y - offset.1) as u32).0[3];
            if a == 0 {
                continue;
            }
            let mut cov = a as f32 / 255.0;
            if let Some(m) = mask {
                cov *= m.coverage(x, y);
            }
            if cov > 0.0 {
                out[y as usize * w as usize + x as usize] = cov;
            }
        }
    }
}

/// 一层位图叠进 f32 累积缓冲（蒙版、剪贴、混合模式都在这里生效）。
fn composite_raster(
    acc: &mut [[f32; 4]],
    w: u32,
    h: u32,
    r: &RasterLayer,
    layer: &Layer,
    clip: Option<&[f32]>,
    base: Option<&[[f32; 4]]>,
    deep: &mut Vec<(usize, f32)>,
) {
    if layer.opacity <= 0.0 {
        return;
    }
    let x0 = r.offset.0.max(0);
    let y0 = r.offset.1.max(0);
    let x1 = (r.offset.0 + r.image.width() as i32).min(w as i32);
    let y1 = (r.offset.1 + r.image.height() as i32).min(h as i32);
    if x1 <= x0 || y1 <= y0 {
        return;
    }
    for y in y0..y1 {
        for x in x0..x1 {
            let px = r
                .image
                .get_pixel((x - r.offset.0) as u32, (y - r.offset.1) as u32)
                .0;
            if px[3] == 0 {
                continue;
            }
            let mut src = [
                px[0] as f32 / 255.0,
                px[1] as f32 / 255.0,
                px[2] as f32 / 255.0,
                px[3] as f32 / 255.0,
            ];
            if let Some(m) = &layer.mask {
                src[3] *= m.coverage(x, y);
            }
            if let Some(c) = clip {
                src[3] *= c[y as usize * w as usize + x as usize];
            }
            // 混合颜色带：按本层颜色 / 下层颜色再遮一层。蒙版与剪贴之后、
            // 混合模式之前乘到 alpha 上 —— 若放在混合之后，被遮掉的像素
            // 已经改过颜色，等于用 Blend If 半透明地盖了一层。
            if let Some(bf) = layer.blend_if.as_ref() {
                let dst = &acc[y as usize * w as usize + x as usize];
                let f = if dst[3] > 0.0 {
                    // 累积缓冲是预乘的，换回直通 RGB 再比范围。
                    let dst_rgb = [
                        (dst[0] / dst[3] * 255.0).round().clamp(0.0, 255.0) as u8,
                        (dst[1] / dst[3] * 255.0).round().clamp(0.0, 255.0) as u8,
                        (dst[2] / dst[3] * 255.0).round().clamp(0.0, 255.0) as u8,
                    ];
                    bf.factor([px[0], px[1], px[2]], dst_rgb)
                } else {
                    // 下层全透明时没有「下层颜色」可比，只按本层范围限制。
                    bf.source_factor([px[0], px[1], px[2]])
                };
                src[3] *= f;
            }
            // 挖空：先按本层覆盖率把下层挖掉，再正常绘制本层。
            // 浅挖空只削弱当前组内的下层内容（穿透组用组起点背景做「地板」，
            // 组外背景保持原样）；深挖空连组外背景一起削弱，并记下像素与
            // 保留系数，由上层在混合前作用到自己的累积缓冲。
            if layer.knockout != Knockout::None {
                let cov = (src[3] * layer.opacity).clamp(0.0, 1.0);
                if cov > 0.0 {
                    let p = y as usize * w as usize + x as usize;
                    let keep = 1.0 - cov;
                    match layer.knockout {
                        Knockout::Shallow => match base {
                            // 组内下层 = 当前累积与组起点背景之差；只削弱这部分。
                            Some(b) => {
                                let bp = b[p];
                                let cur = acc[p];
                                acc[p] = [
                                    bp[0] + (cur[0] - bp[0]) * keep,
                                    bp[1] + (cur[1] - bp[1]) * keep,
                                    bp[2] + (cur[2] - bp[2]) * keep,
                                    bp[3] + (cur[3] - bp[3]) * keep,
                                ];
                            }
                            // 隔离组 / 根层：当前缓冲就是「组内下层」，直接削弱。
                            None => {
                                for c in 0..4 {
                                    acc[p][c] *= keep;
                                }
                            }
                        },
                        Knockout::Deep => {
                            for c in 0..4 {
                                acc[p][c] *= keep;
                            }
                            deep.push((p, keep));
                        }
                        Knockout::None => {}
                    }
                }
            }
            if src[3] <= 0.0 {
                continue;
            }
            // 内容之外的样式笔迹（投影 / 外发光）按样式自己的混合模式与
            // 已合成的下层混合；像图层混合模式那样用会把它盖成不透明色带。
            let style_px = match layer.style.as_ref() {
                Some(s)
                    if s.at(
                        (x - r.offset.0) as u32,
                        (y - r.offset.1) as u32,
                        r.image.width(),
                    ) =>
                {
                    Some(s.blend)
                }
                _ => None,
            };
            let mode = style_px.unwrap_or(layer.blend);
            trace_flat(x, y, layer, src, mode, style_px.is_some());
            // 溶解没有逐像素公式：按 `(x, y)` 抖动把 alpha 二值化，保留的像素
            // 完全不透明，其余完全不画，再走 Normal。
            let (src, opacity, mode) = if mode.is_dissolve() {
                if crate::blend::dissolve_hit(x, y, src[3] * layer.opacity) {
                    ([src[0], src[1], src[2], 1.0], 1.0, BlendMode::Normal)
                } else {
                    ([0.0; 4], 0.0, BlendMode::Normal)
                }
            } else {
                (src, layer.opacity, mode)
            };
            crate::blend::blend_pixel_f32(
                &mut acc[y as usize * w as usize + x as usize],
                src,
                opacity,
                mode,
            );
        }
    }
}

/// Diagnostic: `IMAGEEDIT_FLAT_TRACE=x,y` prints every layer that contributes
/// to that canvas pixel in `flatten_in_doc_space`, with the mode actually used.
fn trace_flat(x: i32, y: i32, layer: &Layer, src: [f32; 4], mode: BlendMode, styled: bool) {
    static AT: std::sync::OnceLock<Option<(i32, i32)>> = std::sync::OnceLock::new();
    let at = AT.get_or_init(|| {
        let v = std::env::var("IMAGEEDIT_FLAT_TRACE").ok()?;
        let mut it = v.split(',');
        Some((it.next()?.trim().parse().ok()?, it.next()?.trim().parse().ok()?))
    });
    if *at != Some((x, y)) {
        return;
    }
    eprintln!(
        "flat-trace ({x},{y}) layer {:?} src {:?} opacity {} mode {:?} style_px {}",
        layer.name, src, layer.opacity, mode, styled
    );
}

